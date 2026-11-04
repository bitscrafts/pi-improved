//! `pii-store`: THE ONLY crate that knows redb.
//!
//! # Single-writer rule
//!
//! A [`redb::WriteTransaction`] is opened and committed inside one method body.
//! It is never stored in a struct, never returned, and never held across a node
//! dispatch. Do not break this invariant — it is what makes the journal's
//! "one write txn per event" property hold (pass-1 §3.4, pass-3 §4.8).
#![forbid(unsafe_code)]

pub mod approval;
pub mod backup;
pub mod channel;
mod codec;
pub mod compact;
pub mod durable;
pub mod heal;
mod identity;
pub mod mem;
pub mod queue;
pub mod session;
#[cfg(test)]
mod stamp_tests;
mod sys_status;
pub mod testing;
pub mod timer;
pub use mem::MemStore;
mod tables;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use pii_abi::{PiiError, RunEvent, RunHeader, RunId, RunOrigin, RunStatus};
use redb::{Database, ReadableDatabase, ReadableTable};
use tables::{
    GRAPH_BODY, META, RUN, RUN_BY_STATUS, RUN_JOURNAL, RUN_SNAPSHOT, SESSION, SESSION_MSG,
    SESSION_SNAPSHOT, SESSION_TOKENS, SYS_STATUS, SYS_STATUS_RUN,
};

pub use codec::{from_bytes, hash, to_bytes};
pub use redb::WriteTransaction;
pub use sys_status::{
    AppendObserver, SysTable, for_each_run_journal, runs_up_to, sys_clear, sys_get, sys_put,
};

/// Persistence handle over one redb database file.
pub struct Store {
    db: Database,
    path: PathBuf,
    /// Test instrumentation: the number of redb write transactions committed
    /// by the append path since the store was opened. **Not a public API
    /// promise** — exposed so the batched-append tests can assert 1-vs-N
    /// write transactions without strace. Monotonic.
    pub(crate) write_txn_count: AtomicU64,
    /// Run ids issued from memory: `(last issued, reserved ceiling)`. The
    /// ceiling is what is durable in `META`; see [`Store::next_run_id`].
    run_ids: parking_lot::Mutex<(u64, u64)>,
    /// Test instrumentation: cumulative bytes written by the session
    /// durability path (meta + per-message appends + periodic snapshots)
    /// since the store was opened (D3/D6, 026 report 5.0.4). **Not a
    /// public API promise** — exposed so the counting-store test can prove
    /// the append-per-message fix keeps total bytes written close to
    /// linear in the transcript size instead of the pre-fix O(n^2)
    /// full-session rewrite. Monotonic.
    session_bytes_written: AtomicU64,
    /// Test instrumentation only — not a public API promise. When `true`,
    /// [`Store::append_session_message`] and [`Store::put_session`] return
    /// a synthetic [`PiiError::Internal`] instead of touching redb, for
    /// deterministically exercising failure-handling call sites (D3/D6:
    /// the "persist failure reaches the Observer" test) without relying on
    /// flaky OS-level tricks (rlimits, permission bits — both no-ops for
    /// root, and cargo test's shared-process threading makes a real
    /// process-wide rlimit unsafe to use from one test among many).
    fail_session_writes_for_test: std::sync::atomic::AtomicBool,
    /// Number of upcoming [`Store::append_events`] calls to fail with a
    /// synthetic error (test instrumentation; decremented per failure).
    fail_append_events_for_test: AtomicU64,
    /// When set, [`Store::read_journal`] fails with a synthetic error
    /// (test instrumentation; resume fail-closed tests).
    fail_read_journal_for_test: std::sync::atomic::AtomicBool,
    /// When set, [`Store::get_run`] fails with a synthetic error
    /// (test instrumentation; resume fail-closed tests).
    fail_get_run_for_test: std::sync::atomic::AtomicBool,
    /// When set, [`Store::get_graph`] fails with a synthetic error
    /// (test instrumentation; resume fail-closed tests).
    fail_get_graph_for_test: std::sync::atomic::AtomicBool,
    /// Set once by [`Store::set_observer`].
    observer: std::sync::OnceLock<std::sync::Arc<dyn pii_observe::Observer>>,
    /// Spec 370: the hook every append runs inside its transaction.
    append_observer: parking_lot::Mutex<Option<AppendObserver>>,
    /// What the process owning this handle is (spec 188): stamped onto every
    /// header [`Store::stamp_origin`] is asked to name.
    origin: parking_lot::RwLock<RunOrigin>,
}

const RUN_COUNTER_KEY: &str = "run_counter";

/// Run ids reserved per durable counter commit (spec 116 F3).
const RUN_ID_BLOCK: u64 = 32;

pub use identity::{FORMAT_VERSION, StoreInfo, file_stats};

impl Store {
    /// Opens (creating if needed) the store at `path`.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] if the database cannot be opened or its
    /// tables cannot be defined.
    pub fn open(path: &Path) -> Result<Self, PiiError> {
        // RSS-gate fix (2026-09-05): redb defaults to a 1 GiB cache — far
        // beyond any budget a control-plane daemon should hold for a store
        // that is typically a few MB. Under sustained soak this showed up
        // as a slow, sublinear RSS climb (thread count flat, fd count flat,
        // store file itself only a few MB) that no amount of connection- or
        // thread-level fixing touched: it was redb's own page cache growing
        // toward its default ceiling. 8 MiB is generous for this store's
        // real working set and leaves ample headroom under the 50 MB
        // rss-gate budget.
        let db = Database::builder()
            .set_cache_size(8 * 1024 * 1024)
            .create(path)
            .map_err(|e| PiiError::store_op("open", &path.display().to_string(), e))?;
        // Define tables up front so reads on first use never race a write.
        {
            let write = db
                .begin_write()
                .map_err(|e| PiiError::store_op("begin_write", "db", e))?;
            {
                let mut table = write
                    .open_table(META)
                    .map_err(|e| PiiError::store_op("open", "meta", e))?;
                if table
                    .get(RUN_COUNTER_KEY)
                    .map_err(|e| PiiError::store_op("read", "run_counter", e))?
                    .is_none()
                {
                    table
                        .insert(RUN_COUNTER_KEY, 0)
                        .map_err(|e| PiiError::store_op("init", "run_counter", e))?;
                }
                identity::ensure_format_version(&mut table, path)?;
            }
            write
                .open_table(GRAPH_BODY)
                .map_err(|e| PiiError::store_op("open", "graph_body", e))?;
            write
                .open_table(RUN)
                .map_err(|e| PiiError::store_op("open", "run", e))?;
            write
                .open_table(RUN_JOURNAL)
                .map_err(|e| PiiError::store_op("open", "run_journal", e))?;
            write
                .open_table(RUN_SNAPSHOT)
                .map_err(|e| PiiError::store_op("open", "run_snapshot", e))?;
            write
                .open_table(RUN_BY_STATUS)
                .map_err(|e| PiiError::store_op("open", "run_by_status", e))?;
            write
                .open_table(SYS_STATUS)
                .map_err(|e| PiiError::store_op("open", "sys_status", e))?;
            write
                .open_table(SYS_STATUS_RUN)
                .map_err(|e| PiiError::store_op("open", "sys_status_run", e))?;
            write
                .open_table(SESSION)
                .map_err(|e| PiiError::store_op("open", "session", e))?;
            write
                .open_table(SESSION_TOKENS)
                .map_err(|e| PiiError::store_op("open", "session_tokens", e))?;
            write
                .open_table(SESSION_MSG)
                .map_err(|e| PiiError::store_op("open", "session_msg", e))?;
            write
                .open_table(SESSION_SNAPSHOT)
                .map_err(|e| PiiError::store_op("open", "session_snapshot", e))?;
            write
                .commit()
                .map_err(|e| PiiError::store_op("commit", "tables", e))?;
        }
        Ok(Self {
            db,
            path: path.to_path_buf(),
            write_txn_count: AtomicU64::new(0),
            run_ids: parking_lot::Mutex::new((0, 0)),
            session_bytes_written: AtomicU64::new(0),
            fail_session_writes_for_test: std::sync::atomic::AtomicBool::new(false),
            fail_append_events_for_test: AtomicU64::new(0),
            fail_read_journal_for_test: std::sync::atomic::AtomicBool::new(false),
            fail_get_run_for_test: std::sync::atomic::AtomicBool::new(false),
            fail_get_graph_for_test: std::sync::atomic::AtomicBool::new(false),
            observer: std::sync::OnceLock::new(),
            append_observer: parking_lot::Mutex::new(None),
            origin: parking_lot::RwLock::new(RunOrigin::default()),
        })
    }

    /// Sets the origin (build revision, process start) this handle stamps onto
    /// new runs. Built once per process by the caller; passed down as data.
    pub fn set_origin(&self, origin: RunOrigin) {
        *self.origin.write() = origin;
    }

    /// The epoch ms at which the process that owns this handle started (spec 217: "since start" counts).
    #[must_use]
    pub fn process_start_ms(&self) -> u64 {
        self.origin.read().process_start_ms
    }

    /// Stamps `header` with this handle's origin and the acceptance time.
    pub fn stamp_origin(&self, header: &mut RunHeader, created_ms: u64) {
        let origin = self.origin.read().clone();
        header.revision = origin.revision;
        header.process_start_ms = origin.process_start_ms;
        header.created_ms = created_ms;
    }

    /// Visits every run header in one pass over the run table, without
    /// materialising them all.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb or decode failure.
    pub fn for_each_run_header(&self, mut f: impl FnMut(RunHeader)) -> Result<(), PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::store_op("begin_read", "db", e))?;
        let table = read
            .open_table(RUN)
            .map_err(|e| PiiError::store_op("open", "run", e))?;
        let iter = table
            .iter()
            .map_err(|e| PiiError::store_op("iter", "run", e))?;
        for item in iter {
            let (_, v) = item.map_err(|e| PiiError::store_op("read", "run", e))?;
            f(from_bytes(v.value())?);
        }
        Ok(())
    }

    /// Cumulative bytes written by the session durability path (D3/D6, 026
    /// report 5.0.4). **Test instrumentation only** — not a public API
    /// promise.
    #[must_use]
    pub fn session_bytes_written(&self) -> u64 {
        self.session_bytes_written.load(Ordering::Relaxed)
    }

    /// Opens a (counted) write transaction. A transaction is opened and
    /// committed inside one method body — never returned to a caller or held
    /// across a node dispatch (single-writer rule).
    /// Allocates the next run id as `r{n:012}` (strictly monotonic, unique
    /// across restarts).
    ///
    /// Ids are reserved in blocks of [`RUN_ID_BLOCK`]: one durable write
    /// transaction moves the persisted counter to the end of the block, and the
    /// ids inside it are then handed out from memory. A crash or restart skips
    /// the unissued rest of the block, so ids may have gaps but are NEVER
    /// reused; nothing about a run's journal or header is less durable (spec
    /// 116 F3: the per-run counter commit was ~20% of the bytes a run wrote).
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn next_run_id(&self) -> Result<RunId, PiiError> {
        let n = {
            let mut ids = self.run_ids.lock();
            if ids.0 >= ids.1 {
                *ids = self.reserve_run_ids()?;
            }
            ids.0 += 1;
            ids.0
        };
        Ok(RunId::from(format!("r{n:012}")))
    }

    /// Persists a new ceiling [`RUN_ID_BLOCK`] ids above the old one and
    /// returns `(last issued, ceiling)` for the fresh block.
    fn reserve_run_ids(&self) -> Result<(u64, u64), PiiError> {
        let write = self.begin_write_counted()?;
        let cur;
        {
            let mut table = write
                .open_table(META)
                .map_err(|e| PiiError::store_op("open", "meta", e))?;
            cur = table
                .get(RUN_COUNTER_KEY)
                .map_err(|e| PiiError::store_op("read", "counter", e))?
                .map_or(0, |v| v.value());
            table
                .insert(RUN_COUNTER_KEY, cur + RUN_ID_BLOCK)
                .map_err(|e| PiiError::store_op("write", "counter", e))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::store_op("commit", "counter", e))?;
        Ok((cur, cur + RUN_ID_BLOCK))
    }

    /// Stores a graph body under its blake3 hash; returns the hash key.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn put_graph(&self, spec_json: &[u8]) -> Result<String, PiiError> {
        let key = hash(spec_json);
        // Spec 116 F3: a body is addressed by its own hash and never changes, so
        // one already stored is already durable; committing it again only
        // rewrites pages (every run stored its graph and input afresh).
        if self.has_graph(&key)? {
            return Ok(key);
        }
        let write = self.begin_write_counted()?;
        {
            let mut table = write
                .open_table(GRAPH_BODY)
                .map_err(|e| PiiError::store_op("open", "graph_body", e))?;
            table
                .insert(key.as_str(), spec_json)
                .map_err(|e| PiiError::store_op("put", "graph", e))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::store_op("commit", "graph", e))?;
        Ok(key)
    }

    /// Whether a body is stored under `key` (no fault injection: a write path).
    fn has_graph(&self, key: &str) -> Result<bool, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::store_op("begin_read", "db", e))?;
        let table = read
            .open_table(GRAPH_BODY)
            .map_err(|e| PiiError::store_op("open", "graph_body", e))?;
        Ok(table
            .get(key)
            .map_err(|e| PiiError::store_op("read", "graph", e))?
            .is_some())
    }

    /// Fetches a stored graph body by hash.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn get_graph(&self, sha_or_hash: &str) -> Result<Option<Vec<u8>>, PiiError> {
        if let Some(e) = self.injected_get_graph_error() {
            return Err(e);
        }
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::store_op("begin_read", "db", e))?;
        let table = read
            .open_table(GRAPH_BODY)
            .map_err(|e| PiiError::store_op("open", "graph_body", e))?;
        let got = table
            .get(sha_or_hash)
            .map_err(|e| PiiError::store_op("get", "graph", e))?;
        Ok(got.map(|v| v.value().to_vec()))
    }

    /// Lists every stored graph body key (the content hashes), in key order.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn all_graphs(&self) -> Result<Vec<String>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::store_op("begin_read", "db", e))?;
        let table = read
            .open_table(GRAPH_BODY)
            .map_err(|e| PiiError::store_op("open", "graph_body", e))?;
        let mut out = Vec::new();
        let iter = table
            .iter()
            .map_err(|e| PiiError::store_op("iter", "graph_body", e))?;
        for item in iter {
            let (key, _) = item.map_err(|e| PiiError::store_op("read", "graph_body", e))?;
            out.push(key.value().to_owned());
        }
        Ok(out)
    }

    /// Appends `event` at `header.seq` and advances `header` in ONE redb write
    /// transaction, committed before returning. Also repoints `run_by_status`.
    ///
    /// Returns the seq the event was written at. Thin wrapper over
    /// [`Self::append_events`] — byte-identical observable behaviour to the
    /// pre-batching single-event path (spec 11 J1).
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn append_event(&self, header: &mut RunHeader, event: &RunEvent) -> Result<u64, PiiError> {
        self.append_events(header, std::slice::from_ref(&event))
    }

    /// Appends `events` at consecutive seqs beginning at `header.seq`, then
    /// advances `header`, in **ONE** redb write transaction (committed before
    /// returning). Inserts every event, persists the advanced header once, and
    /// repoints `run_by_status` once — this is the fix behind spec 11's
    /// transaction-count target (N events in one txn, not N).
    ///
    /// The caller is expected to have already set the terminal status on
    /// `header.status` before calling (as with the single-event path): the
    /// persisted header carries whatever status the header reports, advanced to
    /// `header.seq + events.len()`.
    ///
    /// Returns the seq of the first event written.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb or serialisation failure.
    pub fn append_events(
        &self,
        header: &mut RunHeader,
        events: &[&RunEvent],
    ) -> Result<u64, PiiError> {
        let start = header.seq;
        if let Some(e) = self.injected_append_events_error() {
            return Err(e);
        }
        let write = self.begin_write_counted()?;
        {
            let mut journal = write
                .open_table(RUN_JOURNAL)
                .map_err(|e| PiiError::store_op("open", "journal", e))?;
            for (i, ev) in events.iter().enumerate() {
                let seq = start + i as u64;
                let bytes = to_bytes(*ev)?;
                journal
                    .insert((header.run.as_str(), seq), bytes.as_slice())
                    .map_err(|e| PiiError::store_op("insert", "event", e))?;
            }

            let mut run_tbl = write
                .open_table(RUN)
                .map_err(|e| PiiError::store_op("open", "run", e))?;
            let mut next_header = header.clone();
            next_header.seq = start + events.len() as u64;
            let header_bytes = to_bytes(&next_header)?;
            run_tbl
                .insert(header.run.as_str(), header_bytes.as_slice())
                .map_err(|e| PiiError::store_op("put", "run header", e))?;

            let mut status_tbl = write
                .open_table(RUN_BY_STATUS)
                .map_err(|e| PiiError::store_op("open", "status", e))?;
            for st in [
                RunStatus::Running,
                RunStatus::Suspended,
                RunStatus::Succeeded,
                RunStatus::Failed,
                RunStatus::Refused,
            ] {
                status_tbl
                    .remove((status_name(st), header.run.as_str()))
                    .map_err(|e| PiiError::store_op("remove", "status", e))?;
            }
            status_tbl
                .insert((status_name(header.status), header.run.as_str()), ())
                .map_err(|e| PiiError::store_op("insert", "status", e))?;
        }
        // Spec 370: the health record moves with the run, in this same transaction. The observer runs after the
        // tables above are closed, so it may open the run table itself.
        if let Some(observe) = self.append_observer() {
            let mut next_header = header.clone();
            next_header.seq = start + events.len() as u64;
            observe(self, &write, &next_header, events)?;
        }
        write
            .commit()
            .map_err(|e| PiiError::store_op("commit", "events", e))?;
        header.seq += events.len() as u64;
        Ok(start)
    }

    /// Fetches a run header if present.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn get_run(&self, run: &RunId) -> Result<Option<RunHeader>, PiiError> {
        if let Some(e) = self.injected_get_run_error() {
            return Err(e);
        }
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::store_op("begin_read", "db", e))?;
        let table = read
            .open_table(RUN)
            .map_err(|e| PiiError::store_op("open", "run", e))?;
        let got = table
            .get(run.as_str())
            .map_err(|e| PiiError::store_op("get", "run", e))?;
        match got {
            Some(v) => Ok(Some(from_bytes(v.value())?)),
            None => Ok(None),
        }
    }

    /// Reads a run's full event journal in seq order.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn read_journal(&self, run: &RunId) -> Result<Vec<(u64, RunEvent)>, PiiError> {
        if let Some(e) = self.injected_read_journal_error() {
            return Err(e);
        }
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::store_op("begin_read", "db", e))?;
        let table = read
            .open_table(RUN_JOURNAL)
            .map_err(|e| PiiError::store_op("open", "journal", e))?;
        let mut out = Vec::new();
        let range = table
            .range((run.as_str(), 0)..(run.as_str(), u64::MAX))
            .map_err(|e| PiiError::store_op("range", "journal", e))?;
        for item in range {
            let (key, value) = item.map_err(|e| PiiError::store_op("read", "journal item", e))?;
            let (_, seq) = key.value();
            out.push((seq, from_bytes(value.value())?));
        }
        Ok(out)
    }

    /// Stores a blackboard snapshot at a journal seq.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn put_snapshot(&self, run: &RunId, seq: u64, bytes: &[u8]) -> Result<(), PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::store_op("begin_write", "db", e))?;
        {
            let mut table = write
                .open_table(RUN_SNAPSHOT)
                .map_err(|e| PiiError::store_op("open", "snapshot", e))?;
            table
                .insert((run.as_str(), seq), bytes)
                .map_err(|e| PiiError::store_op("put", "snapshot", e))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::store_op("commit", "snapshot", e))?;
        Ok(())
    }

    /// Returns the highest-seq snapshot for a run, if any.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn latest_snapshot(&self, run: &RunId) -> Result<Option<(u64, Vec<u8>)>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::store_op("begin_read", "db", e))?;
        let table = read
            .open_table(RUN_SNAPSHOT)
            .map_err(|e| PiiError::store_op("open", "snapshot", e))?;
        let mut latest: Option<(u64, Vec<u8>)> = None;
        let range = table
            .range((run.as_str(), 0)..=(run.as_str(), u64::MAX))
            .map_err(|e| PiiError::store_op("range", "snapshot", e))?;
        for item in range {
            let (key, value) = item.map_err(|e| PiiError::store_op("read", "snapshot", e))?;
            let (_, seq) = key.value();
            latest = Some((seq, value.value().to_vec()));
        }
        Ok(latest)
    }

    /// Lists runs by lifecycle status.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn runs_by_status(&self, status: RunStatus) -> Result<Vec<RunId>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::store_op("begin_read", "db", e))?;
        let table = read
            .open_table(RUN_BY_STATUS)
            .map_err(|e| PiiError::store_op("open", "status", e))?;
        let mut out = Vec::new();
        let s = status_name(status);
        let range = table
            .range((s, "")..(s, "\u{10ffff}"))
            .map_err(|e| PiiError::store_op("range", "status", e))?;
        for item in range {
            let (key, _) = item.map_err(|e| PiiError::store_op("read", "status", e))?;
            let (_, run) = key.value();
            out.push(RunId::from(run.to_owned()));
        }
        Ok(out)
    }
}

/// Canonical byte key for a run status (redb keys must be `&str` sorts).
#[must_use]
fn status_name(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "Running",
        RunStatus::Suspended => "Suspended",
        RunStatus::Succeeded => "Succeeded",
        RunStatus::Failed => "Failed",
        RunStatus::Refused => "Refused",
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use pii_abi::{RunEvent, RunId, RunStatus, Terminal};

    use crate::Store;

    fn tmp_store(name: &str) -> Store {
        // N8: all test artifacts live under the crate-local `_tmp` (CARGO_MANIFEST_DIR-relative).
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/pi-store-test"))
            .to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}-{}.redb", name, std::process::id()));
        let _ = std::fs::remove_file(&path);
        Store::open(&path).unwrap()
    }

    #[test]
    fn next_run_id_monotonic() {
        let store = tmp_store("runs");
        let a = store.next_run_id().unwrap();
        let b = store.next_run_id().unwrap();
        assert_ne!(a, b);
        assert!(a.as_str().starts_with('r'));
        assert!(b.as_str().starts_with('r'));
    }

    /// Spec 116 F3: a block of ids costs one commit, not one per id.
    #[test]
    fn run_ids_are_reserved_in_blocks_not_committed_one_by_one() {
        let store = tmp_store("run-id-blocks");
        let before = store.write_txn_count();
        for _ in 0..100 {
            store.next_run_id().unwrap();
        }
        let commits = store.write_txn_count() - before;
        // 100 ids at 32 per block: 4 commits, where one-per-id would be 100.
        assert_eq!(commits, 4);
    }

    /// The price of batching is gaps after a restart; the guarantee that stays
    /// is that an id is never issued twice.
    #[test]
    fn run_ids_are_never_reused_across_a_reopen() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/pi-store-test"))
            .join(format!("reopen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.redb");
        let _ = std::fs::remove_file(&path);
        let first: Vec<_> = {
            let s = Store::open(&path).unwrap();
            (0..5).map(|_| s.next_run_id().unwrap()).collect()
        };
        let s = Store::open(&path).unwrap();
        let after = s.next_run_id().unwrap();
        assert!(
            first.iter().all(|id| id.as_str() < after.as_str()),
            "{first:?} then {after:?}"
        );
        assert_eq!(after.as_str(), format!("r{:012}", super::RUN_ID_BLOCK + 1));
    }

    /// Spec 116 F3: bodies are content-addressed and immutable, so storing one
    /// that is already there must not commit again (every run used to commit
    /// its graph and its input, identical to the last run's).
    #[test]
    fn storing_a_body_that_is_already_stored_commits_nothing() {
        let store = tmp_store("graph-dedup");
        let before = store.write_txn_count();
        let k1 = store.put_graph(b"{ \"graph\": \"dedup-a\" }").unwrap();
        assert_eq!(store.write_txn_count() - before, 1, "new body: one commit");
        let k2 = store.put_graph(b"{ \"graph\": \"dedup-a\" }").unwrap();
        assert_eq!(k1, k2);
        assert_eq!(store.write_txn_count() - before, 1, "same body: none");
        store.put_graph(b"{ \"graph\": \"dedup-b\" }").unwrap();
        assert_eq!(
            store.write_txn_count() - before,
            2,
            "another body: one more"
        );
        assert_eq!(
            store.get_graph(&k1).unwrap().unwrap(),
            b"{ \"graph\": \"dedup-a\" }"
        );
    }

    #[test]
    fn graph_put_get_roundtrip() {
        let store = tmp_store("graph");
        let body = br#"{ "graph": "g1" }"#;
        let key = store.put_graph(body).unwrap();
        let got = store.get_graph(&key).unwrap().unwrap();
        assert_eq!(got, body);
        assert!(store.get_graph("nope").unwrap().is_none());
    }

    #[test]
    fn append_event_transactions_seq_and_status() {
        let store = tmp_store("append");
        let run = RunId::from("r000000000001");
        let mut header = pii_abi::RunHeader {
            run: run.clone(),
            graph: "g".to_string(),
            status: RunStatus::Running,
            seq: 0,
            dur_ms: 0,
            revision: String::new(),
            process_start_ms: 0,
            created_ms: 0,
        };
        let e1 = RunEvent::Started {
            graph: "g".to_string(),
            input_hash: "h1".to_string(),
        };
        let s0 = store.append_event(&mut header, &e1).unwrap();
        assert_eq!(s0, 0);
        assert_eq!(header.seq, 1);

        // Append a terminal event and flip status.
        header.status = RunStatus::Succeeded;
        let e2 = RunEvent::Finished {
            terminal: Terminal::Success {
                output: serde_json::json!("ok"),
            },
            output_hash: "h2".to_string(),
        };
        let s1 = store.append_event(&mut header, &e2).unwrap();
        assert_eq!(s1, 1);
        assert_eq!(header.seq, 2);

        let journal = store.read_journal(&run).unwrap();
        assert_eq!(journal.len(), 2);
        assert_eq!(journal[0].0, 0);
        assert_eq!(journal[1].0, 1);

        let loaded = store.get_run(&run).unwrap().unwrap();
        assert_eq!(loaded.status, RunStatus::Succeeded);
        assert_eq!(loaded.seq, 2);

        let running = store.runs_by_status(RunStatus::Succeeded).unwrap();
        assert!(running.contains(&run));
    }

    #[test]
    fn snapshot_latest() {
        let store = tmp_store("snap");
        let run = RunId::from("r000000000002");
        store.put_snapshot(&run, 4, b"s4").unwrap();
        store.put_snapshot(&run, 8, b"s8").unwrap();
        let (seq, bytes) = store.latest_snapshot(&run).unwrap().unwrap();
        assert_eq!(seq, 8);
        assert_eq!(bytes, b"s8");
    }

    #[test]
    fn open_failure_renders_structured_store_op() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
        std::fs::create_dir_all(&dir).unwrap();
        // A directory is not a database file, so `open` must fail.
        let Err(err) = Store::open(&dir) else {
            panic!("opening a directory must fail");
        };
        assert!(matches!(err, pii_abi::PiiError::StoreOp { .. }));
        let msg = err.to_string();
        assert!(msg.starts_with("store op open on "), "{msg}");
        // The resource at this site is the FULL database path (not a table
        // name): `Store::open` has only the path to name the thing it failed
        // to open.
        assert!(
            msg.starts_with(&format!("store op open on {}: ", dir.display())),
            "{msg}"
        );
        assert!(
            msg.ends_with(": I/O error: Is a directory (os error 21)"),
            "{msg}"
        );
    }

    /// Spec 92 R4: creates a database whose table `name` already exists with
    /// a DIFFERENT type, so `Store::open` fails at that table's `open_table`.
    fn open_with_mismatched_table(tag: &str, name: &'static str) -> String {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{tag}-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let db = redb::Database::create(&path).unwrap();
            let write = db.begin_write().unwrap();
            let wrong: redb::TableDefinition<'_, u8, u8> = redb::TableDefinition::new(name);
            write.open_table(wrong).unwrap();
            write.commit().unwrap();
        }
        let Err(err) = Store::open(&path) else {
            panic!("opening a store with a mistyped `{name}` table must fail");
        };
        err.to_string()
    }

    /// Pins the FULL message (op, resource, detail) at table-open sites so a
    /// swapped op/resource pair at any of them fails.
    #[test]
    fn open_table_failures_render_op_resource_and_detail() {
        for name in ["meta", "graph_body", "run"] {
            let msg = open_with_mismatched_table(&format!("mismatch-{name}"), name);
            assert_eq!(
                msg,
                format!("store op open on {name}: {name} is of type Table<u8, u8>")
            );
        }
    }
}

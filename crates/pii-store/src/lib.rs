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
pub mod channel;
mod codec;
pub mod mem;
pub mod timer;
pub use mem::MemStore;
mod tables;

use std::path::Path;

use pii_abi::{PiiError, ResumeKey, RunEvent, RunHeader, RunId, RunStatus};
use redb::{Database, ReadableDatabase, ReadableTable};
use tables::{GRAPH_BODY, META, RESUME_INDEX, RUN, RUN_BY_STATUS, RUN_JOURNAL, RUN_SNAPSHOT};

pub use codec::{from_bytes, hash, to_bytes};

/// Persistence handle over one redb database file.
pub struct Store {
    db: Database,
}

const RUN_COUNTER_KEY: &str = "run_counter";

impl Store {
    /// Opens (creating if needed) the store at `path`.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] if the database cannot be opened or its
    /// tables cannot be defined.
    pub fn open(path: &Path) -> Result<Self, PiiError> {
        let db = Database::create(path)
            .map_err(|e| PiiError::Internal(format!("store: open {}: {e}", path.display())))?;
        // Define tables up front so reads on first use never race a write.
        {
            let write = db
                .begin_write()
                .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
            {
                let mut table = write
                    .open_table(META)
                    .map_err(|e| PiiError::Internal(format!("store: open meta: {e}")))?;
                if table
                    .get(RUN_COUNTER_KEY)
                    .map_err(|e| PiiError::Internal(format!("store: read run_counter: {e}")))?
                    .is_none()
                {
                    table
                        .insert(RUN_COUNTER_KEY, 0)
                        .map_err(|e| PiiError::Internal(format!("store: init run_counter: {e}")))?;
                }
            }
            write
                .open_table(GRAPH_BODY)
                .map_err(|e| PiiError::Internal(format!("store: open graph_body: {e}")))?;
            write
                .open_table(RUN)
                .map_err(|e| PiiError::Internal(format!("store: open run: {e}")))?;
            write
                .open_table(RUN_JOURNAL)
                .map_err(|e| PiiError::Internal(format!("store: open run_journal: {e}")))?;
            write
                .open_table(RUN_SNAPSHOT)
                .map_err(|e| PiiError::Internal(format!("store: open run_snapshot: {e}")))?;
            write
                .open_table(RUN_BY_STATUS)
                .map_err(|e| PiiError::Internal(format!("store: open run_by_status: {e}")))?;
            write
                .open_table(RESUME_INDEX)
                .map_err(|e| PiiError::Internal(format!("store: open resume_index: {e}")))?;
            write
                .commit()
                .map_err(|e| PiiError::Internal(format!("store: commit tables: {e}")))?;
        }
        Ok(Self { db })
    }

    /// Allocates the next run id as `r{n:012}` (monotonic, in one write txn).
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn next_run_id(&self) -> Result<RunId, PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        let next;
        {
            let mut table = write
                .open_table(META)
                .map_err(|e| PiiError::Internal(format!("store: open meta: {e}")))?;
            let cur = table
                .get(RUN_COUNTER_KEY)
                .map_err(|e| PiiError::Internal(format!("store: read counter: {e}")))?
                .map_or(0, |v| v.value());
            next = cur + 1;
            table
                .insert(RUN_COUNTER_KEY, next)
                .map_err(|e| PiiError::Internal(format!("store: write counter: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit counter: {e}")))?;
        Ok(RunId::from(format!("r{next:012}")))
    }

    /// Stores a graph body under its blake3 hash; returns the hash key.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn put_graph(&self, spec_json: &[u8]) -> Result<String, PiiError> {
        let key = hash(spec_json);
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        {
            let mut table = write
                .open_table(GRAPH_BODY)
                .map_err(|e| PiiError::Internal(format!("store: open graph_body: {e}")))?;
            table
                .insert(key.as_str(), spec_json)
                .map_err(|e| PiiError::Internal(format!("store: put graph: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit graph: {e}")))?;
        Ok(key)
    }

    /// Fetches a stored graph body by hash.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn get_graph(&self, sha_or_hash: &str) -> Result<Option<Vec<u8>>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(GRAPH_BODY)
            .map_err(|e| PiiError::Internal(format!("store: open graph_body: {e}")))?;
        let got = table
            .get(sha_or_hash)
            .map_err(|e| PiiError::Internal(format!("store: get graph: {e}")))?;
        Ok(got.map(|v| v.value().to_vec()))
    }

    /// Appends `event` at `header.seq` and advances `header` in ONE redb write
    /// transaction, committed before returning. Also repoints `run_by_status`.
    ///
    /// Returns the seq the event was written at.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn append_event(&self, header: &mut RunHeader, event: &RunEvent) -> Result<u64, PiiError> {
        let seq = header.seq;
        // The caller bumps status before/after via header mutation; here we
        // store whatever status the header currently reports.
        let bytes = to_bytes(event)?;
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        {
            let mut journal = write
                .open_table(RUN_JOURNAL)
                .map_err(|e| PiiError::Internal(format!("store: open journal: {e}")))?;
            journal
                .insert((header.run.as_str(), seq), bytes.as_slice())
                .map_err(|e| PiiError::Internal(format!("store: insert event: {e}")))?;

            let mut run_tbl = write
                .open_table(RUN)
                .map_err(|e| PiiError::Internal(format!("store: open run: {e}")))?;
            // Persist the header with seq already advanced (the next slot to
            // write), so a reloaded header matches `header.seq` post-append.
            let mut next_header = header.clone();
            next_header.seq = seq + 1;
            let header_bytes = to_bytes(&next_header)?;
            run_tbl
                .insert(header.run.as_str(), header_bytes.as_slice())
                .map_err(|e| PiiError::Internal(format!("store: put run header: {e}")))?;

            let mut status_tbl = write
                .open_table(RUN_BY_STATUS)
                .map_err(|e| PiiError::Internal(format!("store: open status: {e}")))?;
            // Repoint, don't append: drop the run from every status bucket,
            // then list it under its current status. (Removing only the
            // previous key would leave a stale Running row behind forever.)
            for st in [
                RunStatus::Running,
                RunStatus::Suspended,
                RunStatus::Succeeded,
                RunStatus::Failed,
            ] {
                status_tbl
                    .remove((status_name(st), header.run.as_str()))
                    .map_err(|e| PiiError::Internal(format!("store: remove status: {e}")))?;
            }
            status_tbl
                .insert((status_name(header.status), header.run.as_str()), ())
                .map_err(|e| PiiError::Internal(format!("store: insert status: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit event: {e}")))?;
        header.seq += 1;
        Ok(seq)
    }

    /// Fetches a run header if present.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn get_run(&self, run: &RunId) -> Result<Option<RunHeader>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(RUN)
            .map_err(|e| PiiError::Internal(format!("store: open run: {e}")))?;
        let got = table
            .get(run.as_str())
            .map_err(|e| PiiError::Internal(format!("store: get run: {e}")))?;
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
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(RUN_JOURNAL)
            .map_err(|e| PiiError::Internal(format!("store: open journal: {e}")))?;
        let mut out = Vec::new();
        let range = table
            .range((run.as_str(), 0)..(run.as_str(), u64::MAX))
            .map_err(|e| PiiError::Internal(format!("store: range journal: {e}")))?;
        for item in range {
            let (key, value) =
                item.map_err(|e| PiiError::Internal(format!("store: read journal item: {e}")))?;
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
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        {
            let mut table = write
                .open_table(RUN_SNAPSHOT)
                .map_err(|e| PiiError::Internal(format!("store: open snapshot: {e}")))?;
            table
                .insert((run.as_str(), seq), bytes)
                .map_err(|e| PiiError::Internal(format!("store: put snapshot: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit snapshot: {e}")))?;
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
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(RUN_SNAPSHOT)
            .map_err(|e| PiiError::Internal(format!("store: open snapshot: {e}")))?;
        let mut latest: Option<(u64, Vec<u8>)> = None;
        let range = table
            .range((run.as_str(), 0)..=(run.as_str(), u64::MAX))
            .map_err(|e| PiiError::Internal(format!("store: range snapshot: {e}")))?;
        for item in range {
            let (key, value) =
                item.map_err(|e| PiiError::Internal(format!("store: read snapshot: {e}")))?;
            let (_, seq) = key.value();
            latest = Some((seq, value.value().to_vec()));
        }
        Ok(latest)
    }

    /// Maps a durable resume key to a run id.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn put_resume_key(&self, key: &ResumeKey, run: &RunId) -> Result<(), PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        {
            let mut table = write
                .open_table(RESUME_INDEX)
                .map_err(|e| PiiError::Internal(format!("store: open resume: {e}")))?;
            table
                .insert(key.0.as_str(), run.as_str())
                .map_err(|e| PiiError::Internal(format!("store: put resume key: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit resume: {e}")))?;
        Ok(())
    }

    /// Lists runs by lifecycle status.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn runs_by_status(&self, status: RunStatus) -> Result<Vec<RunId>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(RUN_BY_STATUS)
            .map_err(|e| PiiError::Internal(format!("store: open status: {e}")))?;
        let mut out = Vec::new();
        let s = status_name(status);
        let range = table
            .range((s, "")..(s, "\u{10ffff}"))
            .map_err(|e| PiiError::Internal(format!("store: range status: {e}")))?;
        for item in range {
            let (key, _) =
                item.map_err(|e| PiiError::Internal(format!("store: read status: {e}")))?;
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
    use redb::ReadableDatabase;

    use crate::Store;

    fn tmp_store(name: &str) -> Store {
        // N8: all test artifacts live under /projects/pi-improved/_tmp.
        let dir = std::path::Path::new("/projects/pi-improved/_tmp/pi-store-test").to_path_buf();
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
    fn resume_key_index() {
        let store = tmp_store("resume");
        let run = RunId::from("r000000000003");
        let key = pii_abi::ResumeKey("k1".to_string());
        store.put_resume_key(&key, &run).unwrap();
        let read = store
            .db
            .begin_read()
            .unwrap()
            .open_table(crate::tables::RESUME_INDEX)
            .unwrap();
        let got = read.get("k1").unwrap().unwrap();
        assert_eq!(got.value(), run.as_str());
    }
}

//! Store identity (spec 99): what a redb file actually is — path, size,
//! persisted format version, per-table entry counts, last write — and the
//! [`Health`] reports built from it.
//!
//! The format version lives in the `meta` table under
//! [`FORMAT_VERSION_KEY`]. It is written on first open and a store whose
//! recorded version is NEWER than [`FORMAT_VERSION`] is refused, so an old
//! binary never writes into a layout it does not understand.

// reason: see tables.rs — `pub(crate)` items in a private module; clippy
// `redundant_pub_crate` conflicts with rustc `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_abi::PiiError;
use pii_observe::{Event, Health, Level, Observable, Observed, Observer, SubsystemHealth, names};
use redb::{ReadableDatabase, ReadableTable, ReadableTableMetadata};

use crate::tables::{
    GRAPH_BODY, META, RESUME_INDEX, RUN, RUN_JOURNAL, RUN_SNAPSHOT, SESSION, SESSION_MSG,
};
use crate::{MemStore, Store};

/// The on-disk layout version this binary reads and writes.
pub const FORMAT_VERSION: u64 = 1;
pub(crate) const FORMAT_VERSION_KEY: &str = "store.format_version";

/// What a store file reports about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreInfo {
    /// The file the store was opened from.
    pub path: PathBuf,
    /// File size in bytes.
    pub bytes: u64,
    /// The format version persisted in the file's `meta` table.
    pub format_version: u64,
    /// Entry count per table.
    pub tables: BTreeMap<String, u64>,
    /// File modification time (ms since the Unix epoch), when readable.
    pub last_write_ms: Option<u64>,
}

/// File size and modification time (ms since the epoch) of `path`; `(0, None)`
/// when it cannot be read.
#[must_use]
pub fn file_stats(path: &Path) -> (u64, Option<u64>) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (0, None);
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| u64::try_from(d.as_millis()).ok());
    (meta.len(), mtime)
}

fn read_err(what: &str, e: impl std::fmt::Display) -> PiiError {
    PiiError::Internal(format!("store identity: {what}: {e}"))
}

impl Store {
    /// The file this store was opened from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reports the store's identity: path, size, persisted format version
    /// and per-table entry counts.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn info(&self) -> Result<StoreInfo, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| read_err("begin_read", e))?;
        let format_version = read
            .open_table(META)
            .map_err(|e| read_err("meta", e))?
            .get(FORMAT_VERSION_KEY)
            .map_err(|e| read_err("format_version", e))?
            .map_or(0, |v| v.value());
        let mut tables = BTreeMap::new();
        macro_rules! count {
            ($name:literal, $def:expr) => {
                tables.insert(
                    $name.to_owned(),
                    read.open_table($def)
                        .map_err(|e| read_err($name, e))?
                        .len()
                        .map_err(|e| read_err($name, e))?,
                );
            };
        }
        count!("graph_body", GRAPH_BODY);
        count!("resume_index", RESUME_INDEX);
        count!("run", RUN);
        count!("run_journal", RUN_JOURNAL);
        count!("run_snapshot", RUN_SNAPSHOT);
        count!("session", SESSION);
        count!("session_msg", SESSION_MSG);
        let (bytes, last_write_ms) = file_stats(&self.path);
        Ok(StoreInfo {
            path: self.path.clone(),
            bytes,
            format_version,
            tables,
            last_write_ms,
        })
    }
}

/// Stamps a fresh (or pre-versioning) store with [`FORMAT_VERSION`], and
/// refuses one whose recorded version is NEWER than this binary understands,
/// so an old binary never writes into a layout it does not know.
pub(crate) fn ensure_format_version(
    meta: &mut redb::Table<'_, &str, u64>,
    path: &Path,
) -> Result<(), PiiError> {
    let recorded = meta
        .get(FORMAT_VERSION_KEY)
        .map_err(|e| PiiError::store_op("read", "format_version", e))?
        .map(|v| v.value());
    match recorded {
        Some(v) if v > FORMAT_VERSION => Err(PiiError::Invalid(format!(
            "store '{}' has format version {v}, which is newer than the \
             version {FORMAT_VERSION} this binary understands; upgrade pii or \
             use a different store path",
            path.display()
        ))),
        Some(_) => Ok(()),
        None => meta
            .insert(FORMAT_VERSION_KEY, FORMAT_VERSION)
            .map(|_| ())
            .map_err(|e| PiiError::store_op("init", "format_version", e)),
    }
}

/// Stamps `db` (an auxiliary store: approvals, heal, memory, channel, timer)
/// with [`FORMAT_VERSION`] in its own `meta` table, refusing a newer one.
pub(crate) fn stamp(db: &redb::Database, path: &Path) -> Result<(), PiiError> {
    let write = db
        .begin_write()
        .map_err(|e| PiiError::store_op("begin_write", "format_version", e))?;
    {
        let mut meta = write
            .open_table(META)
            .map_err(|e| PiiError::store_op("open", "meta", e))?;
        ensure_format_version(&mut meta, path)?;
    }
    write
        .commit()
        .map_err(|e| PiiError::store_op("commit", "format_version", e))
}

/// The version recorded in `db`'s `meta` table (0 when unstamped).
pub(crate) fn read_stamp(db: &redb::Database) -> Result<u64, PiiError> {
    let read = db.begin_read().map_err(|e| read_err("begin_read", e))?;
    let meta = match read.open_table(META) {
        Ok(t) => t,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(0),
        Err(e) => return Err(read_err("meta", e)),
    };
    Ok(meta
        .get(FORMAT_VERSION_KEY)
        .map_err(|e| read_err("format_version", e))?
        .map_or(0, |v| v.value()))
}

impl Health for Store {
    fn health(&self) -> SubsystemHealth {
        let mut h = SubsystemHealth::ok("store");
        match self.info() {
            Ok(i) => {
                h.identity
                    .insert("path".into(), i.path.display().to_string());
                h.identity.insert("bytes".into(), i.bytes.to_string());
                h.identity
                    .insert("format_version".into(), i.format_version.to_string());
                for (name, n) in i.tables {
                    h.identity.insert(format!("entries.{name}"), n.to_string());
                }
            }
            Err(e) => {
                h.state = pii_observe::State::Down;
                h.detail = Some(e.to_string());
                h.identity
                    .insert("path".into(), self.path.display().to_string());
            }
        }
        h
    }
}

impl Health for MemStore {
    fn health(&self) -> SubsystemHealth {
        let mut h = SubsystemHealth::ok("memory");
        h.identity
            .insert("path".into(), self.path().display().to_string());
        let (bytes, _) = file_stats(self.path());
        h.identity.insert("bytes".into(), bytes.to_string());
        match self.n_docs() {
            Ok(n) => {
                h.identity.insert("docs".into(), n.to_string());
            }
            Err(e) => {
                h.state = pii_observe::State::Down;
                h.detail = Some(e.to_string());
            }
        }
        h
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
impl Store {
    /// Overwrites the persisted format version (simulates a newer binary).
    pub(crate) fn set_format_version_for_test(&self, v: u64) {
        let write = self.db.begin_write().unwrap();
        write
            .open_table(META)
            .unwrap()
            .insert(FORMAT_VERSION_KEY, v)
            .unwrap();
        write.commit().unwrap();
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

    use crate::{FORMAT_VERSION, Store};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/identity-test"))
            .to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn fresh_store_persists_and_reports_format_version() {
        let path = scratch("fmt");
        let store = Store::open(&path).unwrap();
        let info = store.info().unwrap();
        assert_eq!(info.format_version, FORMAT_VERSION);
        assert_eq!(info.path, path);
        assert!(info.bytes > 0, "a created redb file has a size");
        drop(store);
        // Persisted, not a printed constant: a reopen reads the same entry back.
        assert_eq!(
            Store::open(&path).unwrap().info().unwrap().format_version,
            FORMAT_VERSION
        );
    }

    #[test]
    fn newer_format_version_is_refused_with_a_clear_message() {
        let path = scratch("newer");
        let store = Store::open(&path).unwrap();
        store.set_format_version_for_test(FORMAT_VERSION + 1);
        drop(store);
        let err = Store::open(&path).err().expect("must refuse a newer store");
        let msg = err.to_string();
        assert_eq!(
            msg,
            format!(
                "invalid: store '{}' has format version {}, which is newer than the \
                 version {FORMAT_VERSION} this binary understands; upgrade pii or \
                 use a different store path",
                path.display(),
                FORMAT_VERSION + 1
            )
        );
    }

    #[test]
    fn counts_reflect_stored_runs_and_events() {
        let store = Store::open(&scratch("counts")).unwrap();
        let before = store.info().unwrap();
        assert_eq!((before.tables["run"], before.tables["run_journal"]), (0, 0));
        let mut header = pii_abi::RunHeader {
            run: store.next_run_id().unwrap(),
            graph: "g".to_owned(),
            status: pii_abi::RunStatus::Running,
            seq: 0,
            dur_ms: 0,
            revision: String::new(),
            process_start_ms: 0,
            created_ms: 0,
        };
        let ev = pii_abi::RunEvent::Started {
            graph: "g".to_owned(),
            input_hash: "h".to_owned(),
        };
        store.append_event(&mut header, &ev).unwrap();
        let after = store.info().unwrap();
        assert_eq!((after.tables["run"], after.tables["run_journal"]), (1, 1));
    }
}
// ---- observability seam (spec 100) ----

impl Observable for Store {
    fn observer(&self) -> Option<&dyn Observer> {
        self.observer.get().map(AsRef::as_ref)
    }
}

impl Store {
    /// Attaches the observer (first call wins; later calls are ignored).
    ///
    /// Set after construction rather than injected into [`Store::open`]: the
    /// store is opened at many call sites (and shared as an `Arc`) before any
    /// observer exists.
    pub fn set_observer(&self, observer: Arc<dyn Observer>) {
        let _ = self.observer.set(observer);
    }

    /// Passes `r` through unchanged, reporting a failure of write `op`.
    pub(crate) fn report_write<T>(
        &self,
        op: &'static str,
        r: Result<T, PiiError>,
    ) -> Result<T, PiiError> {
        if let Err(e) = &r {
            self.emit(
                &Event::new("store", Level::Error, names::STORE_PERSIST_ERROR)
                    .field("op", op)
                    .field("path", self.path.display().to_string())
                    .field("error", e.to_string()),
            );
        }
        r
    }
}

#[cfg(test)]
mod observe_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use pii_observe::Recorder;

    use super::*;

    #[test]
    fn persist_failure_reaches_the_observer_and_is_still_returned() {
        let dir =
            crate::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "store-observe-").unwrap();
        let store = Store::open(&dir.path().join("s.redb")).unwrap();
        let rec = Arc::new(Recorder::new());
        store.set_observer(rec.clone());
        store.set_fail_session_writes_for_test(true);
        assert!(store.append_session_message("s1", b"x").is_err());
        let ev = rec.events_named(names::STORE_PERSIST_ERROR);
        assert_eq!(ev.len(), 1);
        assert_eq!(
            ev[0].fields.get("op"),
            Some(&"append_session_message".into())
        );
        store.set_fail_session_writes_for_test(false);
        store.append_session_message("s1", b"x").unwrap();
        assert_eq!(rec.events_named(names::STORE_PERSIST_ERROR).len(), 1);
    }
}

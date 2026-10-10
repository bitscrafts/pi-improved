//! The entity store: one canonical file, written atomically under a lock.
//!
//! `<dir>/memory/graph.json` holds only the records (sorted maps, compact JSON),
//! so the same writes in any order leave the same bytes. The counters live in
//! `counters.json` beside it, because how many writes *changed* something does
//! depend on order. A refused call changes neither the records nor their bytes.

use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_core::Clock as _;
use pii_observe::{Event, Level, NullObserver, Observer, names};
use serde::{Deserialize, Serialize};

use super::types::{Entity, EntityInput, GraphError, Relationship, RelationshipInput};
use super::upsert::{upsert_entity, upsert_relationship};

/// Largest graph or counters file this store will read.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const LOCK_TRIES: u32 = 300;
const LOCK_STALE: Duration = Duration::from_secs(30);

/// The bounds on what the store holds and what a query may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphConfig {
    /// Most entities. A write that would add one more is refused (`entity_cap`).
    pub max_entities: usize,
    /// Most relationships (`relationship_cap`).
    pub max_relationships: usize,
    /// Most provenance sources on one record. A write that would exceed it is refused
    /// (`provenance_cap`) and the record is left as it was; nothing is evicted.
    pub max_provenance: usize,
    /// Most attributes on one record.
    pub max_attributes: usize,
    /// Deepest `neighbours` walk (`depth_cap`).
    pub max_depth: u32,
    /// The depth a `neighbours` call uses when it names none.
    pub default_depth: u32,
    /// Most results a query may return; more matches are refused (`result_cap`), never truncated.
    pub max_results: usize,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            max_entities: 10_000,
            max_relationships: 50_000,
            max_provenance: 32,
            max_attributes: 32,
            max_depth: 3,
            default_depth: 1,
            max_results: 200,
        }
    }
}

/// How a successful write changed the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// A record that did not exist.
    Created,
    /// An existing record gained provenance or attributes.
    Merged,
    /// Nothing new.
    Unchanged,
}

impl Written {
    /// The word used in events.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Merged => "merged",
            Self::Unchanged => "unchanged",
        }
    }
}

/// A successful write: the record as stored and what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Put<T> {
    /// The record as stored.
    pub record: T,
    /// What the write did.
    pub written: Written,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct State {
    #[serde(default = "one")]
    version: u32,
    #[serde(default)]
    pub(super) entities: BTreeMap<String, Entity>,
    #[serde(default)]
    pub(super) relationships: BTreeMap<String, Relationship>,
}

const fn one() -> u32 {
    1
}

/// Counters persisted beside the graph (not part of its bytes).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    /// Entity writes that created or merged.
    #[serde(default)]
    pub entities_written: u64,
    /// Relationship writes that created or merged.
    #[serde(default)]
    pub relationships_written: u64,
    /// Refused writes by reason.
    #[serde(default)]
    pub write_refusals: BTreeMap<String, u64>,
    /// Refused queries by reason.
    #[serde(default)]
    pub query_refusals: BTreeMap<String, u64>,
}

/// What `pii health` and `pii metrics` report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    /// Entities now stored.
    pub entities: u64,
    /// Relationships now stored.
    pub relationships: u64,
    /// Entity writes that changed the graph.
    pub entities_written: u64,
    /// Relationship writes that changed the graph.
    pub relationships_written: u64,
    /// Refused writes by reason.
    pub write_refusals: BTreeMap<String, u64>,
    /// Refused queries by reason.
    pub query_refusals: BTreeMap<String, u64>,
}

/// What a compaction measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactReport {
    /// Bytes of the graph file before.
    pub bytes_before: u64,
    /// Bytes after.
    pub bytes_after: u64,
}

/// The entity store under one directory. Cheap to clone-by-`Arc`; holds no file open.
pub struct EntityStore {
    area: PathBuf,
    cfg: GraphConfig,
    observer: Arc<dyn Observer>,
}

struct LockGuard(PathBuf);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn io_err(what: &str, path: &Path, e: impl std::fmt::Display) -> GraphError {
    GraphError::Store(format!("{what} {}: {e}", path.display()))
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T, GraphError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(T::default()),
        Err(e) => return Err(io_err("stat", path, e)),
    };
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err(io_err(
            "read",
            path,
            "not a regular file within the size bound",
        ));
    }
    let mut buf = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE_BYTES).read_to_end(&mut buf))
        .map_err(|e| io_err("read", path, e))?;
    serde_json::from_slice(&buf).map_err(|e| io_err("parse", path, e))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), GraphError> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|e| io_err("write", &tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| io_err("rename", path, e))
}

fn bump(map: &mut BTreeMap<String, u64>, key: &str) {
    let n = map.entry(key.to_owned()).or_insert(0);
    *n = n.saturating_add(1);
}

impl EntityStore {
    /// Opens (creating the area) the store under `dir`. Nothing is read yet.
    ///
    /// # Errors
    /// [`GraphError::Store`] when the area cannot be created.
    pub fn open(dir: &Path, cfg: GraphConfig) -> Result<Self, GraphError> {
        let area = dir.join("memory");
        fs::create_dir_all(&area).map_err(|e| io_err("create", &area, e))?;
        Ok(Self {
            area,
            cfg,
            observer: Arc::new(NullObserver),
        })
    }

    /// Reports writes and refusals to `observer`.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = observer;
        self
    }

    /// The bounds in force.
    #[must_use]
    pub const fn config(&self) -> &GraphConfig {
        &self.cfg
    }

    /// The canonical graph file.
    #[must_use]
    pub fn graph_path(&self) -> PathBuf {
        self.area.join("graph.json")
    }

    fn counters_path(&self) -> PathBuf {
        self.area.join("counters.json")
    }

    fn lock_path(&self) -> PathBuf {
        self.area.join("graph.lock")
    }

    pub(super) fn load(&self) -> Result<State, GraphError> {
        read_json(&self.graph_path())
    }

    fn lock(&self) -> Result<LockGuard, GraphError> {
        let path = self.lock_path();
        for _ in 0..LOCK_TRIES {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    let _ = write!(f, "{}", std::process::id());
                    return Ok(LockGuard(path));
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    if lock_is_stale(&path) {
                        let _ = fs::remove_file(&path);
                    } else {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                Err(e) => return Err(io_err("lock", &path, e)),
            }
        }
        Err(io_err(
            "lock",
            &path,
            "held by another process for too long",
        ))
    }

    /// Runs `f` on the loaded state under the lock; saves the graph when `f` says it changed,
    /// and the counters when they did. A refusal from `f` is counted here, once.
    fn txn<T>(
        &self,
        query: bool,
        f: impl FnOnce(&mut State, &mut Counters) -> Result<(T, bool), GraphError>,
    ) -> Result<T, GraphError> {
        let _guard = self.lock()?;
        let mut state = self.load()?;
        let mut counters: Counters = read_json(&self.counters_path())?;
        let before = counters.clone();
        let outcome = f(&mut state, &mut counters);
        let result = match outcome {
            Ok((v, changed)) => {
                if changed {
                    let bytes =
                        serde_json::to_vec(&state).map_err(|e| GraphError::Store(e.to_string()))?;
                    write_atomic(&self.graph_path(), &bytes)?;
                }
                Ok(v)
            }
            Err(GraphError::Refused(r, why)) => {
                let map = if query {
                    &mut counters.query_refusals
                } else {
                    &mut counters.write_refusals
                };
                bump(map, r.as_str());
                Err(GraphError::Refused(r, why))
            }
            Err(e) => Err(e),
        };
        if counters != before {
            let bytes =
                serde_json::to_vec(&counters).map_err(|e| GraphError::Store(e.to_string()))?;
            write_atomic(&self.counters_path(), &bytes)?;
        }
        result
    }

    /// Records a refused query that was decided outside a transaction.
    pub(super) fn refuse_query(&self, op: &'static str, e: GraphError) -> GraphError {
        if let GraphError::Refused(r, _) = &e {
            let r = *r;
            let counted = self.txn::<()>(true, |_, _| Err(GraphError::Refused(r, String::new())));
            drop(counted);
            self.observer.event(
                &Event::new("memory", Level::Warn, names::MEMORY_QUERY_REFUSED)
                    .field("op", op)
                    .field("reason", r.as_str())
                    .field("error", e.to_string()),
            );
        }
        e
    }

    fn refused_write(&self, what: &'static str, e: GraphError) -> GraphError {
        if let GraphError::Refused(r, _) = &e {
            self.observer.event(
                &Event::new("memory", Level::Warn, names::MEMORY_WRITE_REFUSED)
                    .field("what", what)
                    .field("reason", r.as_str())
                    .field("error", e.to_string()),
            );
        }
        e
    }

    /// Describes an entity: creates it, or merges attributes and provenance into the one
    /// that has the same kind and name.
    ///
    /// # Errors
    /// A named refusal (`no_provenance`, `bad_provenance`, `invalid`, `too_large`,
    /// `entity_cap`, `provenance_cap`), counted and reported; or a store failure.
    pub fn put_entity(&self, input: EntityInput) -> Result<Put<Entity>, GraphError> {
        let cfg = self.cfg;
        let res = self.txn(false, move |state, counters| {
            let put = upsert_entity(state, &cfg, input)?;
            let changed = put.written != Written::Unchanged;
            if changed {
                counters.entities_written = counters.entities_written.saturating_add(1);
            }
            Ok((put, changed))
        });
        match res {
            Ok(put) => {
                self.observer.event(
                    &Event::new("memory", Level::Info, names::MEMORY_ENTITY_WRITTEN)
                        .field("id", put.record.id.clone())
                        .field("kind", put.record.kind.clone())
                        .field("outcome", put.written.as_str())
                        .field("provenance", put.record.provenance.len() as u64),
                );
                Ok(put)
            }
            Err(e) => Err(self.refused_write("entity", e)),
        }
    }

    /// Describes a relationship between two entities that exist. A relationship naming an
    /// unknown entity is **refused** (`unknown_entity`); no stub is created.
    ///
    /// # Errors
    /// As [`EntityStore::put_entity`], plus `unknown_entity` and `relationship_cap`.
    pub fn put_relationship(
        &self,
        input: RelationshipInput,
    ) -> Result<Put<Relationship>, GraphError> {
        let cfg = self.cfg;
        let res = self.txn(false, move |state, counters| {
            let put = upsert_relationship(state, &cfg, input)?;
            let changed = put.written != Written::Unchanged;
            if changed {
                counters.relationships_written = counters.relationships_written.saturating_add(1);
            }
            Ok((put, changed))
        });
        match res {
            Ok(put) => {
                self.observer.event(
                    &Event::new("memory", Level::Info, names::MEMORY_RELATIONSHIP_WRITTEN)
                        .field("id", put.record.id.clone())
                        .field("kind", put.record.kind.clone())
                        .field("from", put.record.from.clone())
                        .field("to", put.record.to.clone())
                        .field("outcome", put.written.as_str())
                        .field("provenance", put.record.provenance.len() as u64),
                );
                Ok(put)
            }
            Err(e) => Err(self.refused_write("relationship", e)),
        }
    }

    /// The current sizes and counters.
    ///
    /// # Errors
    /// [`GraphError::Store`] when a file cannot be read.
    pub fn counts(&self) -> Result<Counts, GraphError> {
        let state = self.load()?;
        let c: Counters = read_json(&self.counters_path())?;
        Ok(Counts {
            entities: state.entities.len() as u64,
            relationships: state.relationships.len() as u64,
            entities_written: c.entities_written,
            relationships_written: c.relationships_written,
            write_refusals: c.write_refusals,
            query_refusals: c.query_refusals,
        })
    }

    /// Rewrites the graph file in its canonical form and removes leftovers of an interrupted
    /// write (`graph.json.tmp`, a stale lock). It never drops a record: there is no eviction.
    /// The caller (`pii memory compact`) is responsible for refusing while a daemon runs.
    ///
    /// # Errors
    /// [`GraphError::Store`] when a file cannot be read or written.
    pub fn compact(&self) -> Result<CompactReport, GraphError> {
        let _guard = self.lock()?;
        let size = |p: &Path| fs::metadata(p).map_or(0, |m| m.len());
        let bytes_before = size(&self.graph_path());
        let state = self.load()?;
        let bytes = serde_json::to_vec(&state).map_err(|e| GraphError::Store(e.to_string()))?;
        write_atomic(&self.graph_path(), &bytes)?;
        for leftover in ["graph.json.tmp", "counters.json.tmp"] {
            let p = self.area.join(leftover);
            if p.is_file() {
                let _ = fs::remove_file(&p);
            }
        }
        Ok(CompactReport {
            bytes_before,
            bytes_after: size(&self.graph_path()),
        })
    }
}

fn lock_is_stale(path: &Path) -> bool {
    let modified_ms = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis());
    let now_ms = u128::from(pii_core::SystemClock.now_ms());
    if modified_ms.is_some_and(|m| now_ms.saturating_sub(m) > LOCK_STALE.as_millis()) {
        return true;
    }
    let pid = fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    pid.is_some_and(|p| !Path::new(&format!("/proc/{p}")).exists())
}

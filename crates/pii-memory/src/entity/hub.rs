//! The slot the engine's nodes share, and the health seam over the stored counts.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::PiiError;
use pii_observe::{Health, State, SubsystemHealth};

use super::store::{EntityStore, GraphConfig};

/// Holds the daemon's [`EntityStore`]. Starts empty; `Daemon::build` fills it, and the
/// `remember` and `recall` nodes fail loudly when it is empty.
#[derive(Default)]
pub struct EntityHub {
    slot: Mutex<Option<Arc<EntityStore>>>,
}

impl EntityHub {
    /// Installs the store (replacing any earlier one).
    pub fn set(&self, store: Arc<EntityStore>) {
        *self.slot.lock() = Some(store);
    }

    /// The installed store, if any.
    #[must_use]
    pub fn get(&self) -> Option<Arc<EntityStore>> {
        self.slot.lock().clone()
    }

    /// The installed store.
    ///
    /// # Errors
    /// [`PiiError::Denied`] when the daemon never installed one.
    pub fn require(&self) -> Result<Arc<EntityStore>, PiiError> {
        self.get().ok_or_else(|| {
            PiiError::Denied(String::from(
                "memory: the entity store is not installed (it exists only under a daemon)",
            ))
        })
    }
}

/// Read-only health over `<dir>/memory`: reports the counts a `pii health` reader wants.
pub struct EntityHealth {
    dir: PathBuf,
    cfg: GraphConfig,
}

impl EntityHealth {
    /// Health over the store directory `dir` (the one holding `memory/`).
    #[must_use]
    pub const fn new(dir: PathBuf, cfg: GraphConfig) -> Self {
        Self { dir, cfg }
    }

    /// The stored counts; `None` when the area was never written or cannot be read.
    #[must_use]
    pub fn counts(&self) -> Option<super::store::Counts> {
        let area = self.dir.join("memory");
        if !area.join("graph.json").is_file() && !area.join("counters.json").is_file() {
            return None;
        }
        let store = EntityStore::open(&self.dir, self.cfg).ok()?;
        // Report transitions that time alone caused, so the counters move without a read.
        let _ = store.sweep();
        store.counts().ok()
    }
}

impl Health for EntityHealth {
    fn health(&self) -> SubsystemHealth {
        let mut h = SubsystemHealth::ok("memory_entities");
        match EntityStore::open(&self.dir, self.cfg).and_then(|s| s.counts()) {
            Ok(c) => {
                let put = |h: &mut SubsystemHealth, k: &str, v: u64| {
                    h.identity.insert(k.into(), v.to_string());
                };
                put(&mut h, "entities", c.entities);
                put(&mut h, "relationships", c.relationships);
                put(&mut h, "entity_cap", self.cfg.max_entities as u64);
                put(
                    &mut h,
                    "relationship_cap",
                    self.cfg.max_relationships as u64,
                );
                put(&mut h, "bytes", c.bytes);
                put(&mut h, "max_bytes", self.cfg.max_bytes);
                for (tier, n) in &c.tiers {
                    put(&mut h, &format!("tier_{tier}"), *n);
                }
                put(&mut h, "write_refusals", c.write_refusals.values().sum());
                put(&mut h, "query_refusals", c.query_refusals.values().sum());
            }
            Err(e) => {
                h.state = State::Degraded;
                h.detail = Some(e.to_string());
            }
        }
        h
    }
}

//! Spec 217: agent-call observations, recorded into the run journal.
//!
//! A node reports its `coder.spawned`, `coder.exited` and `worker.start.refused` through its context. The engine
//! records them with [`CallRecorder`] before the node's own `NodeFinished`. Every count here is derived from those
//! journal events, never from a node's returned value.

use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{NodeId, PiiError, RunEvent, RunStatus};
use pii_core::{Metric, ObsEvent, Observer};
use pii_store::Store;

/// Records one node's agent-call observations while it runs, and forwards every observation unchanged.
pub struct CallRecorder {
    node: NodeId,
    inner: Arc<dyn Observer>,
    seen: Mutex<Vec<RunEvent>>,
}

impl CallRecorder {
    /// A recorder for `node` that forwards to `inner`.
    #[must_use]
    pub fn new(node: NodeId, inner: Arc<dyn Observer>) -> Self {
        Self {
            node,
            inner,
            seen: Mutex::new(Vec::new()),
        }
    }

    /// The observations recorded so far, in the order the node made them.
    #[must_use]
    pub fn take(&self) -> Vec<RunEvent> {
        std::mem::take(&mut *self.seen.lock())
    }
}

impl Observer for CallRecorder {
    fn event(&self, e: &ObsEvent) {
        if let ObsEvent::NodeObserved { kind, fields } = e {
            self.seen.lock().push(RunEvent::NodeObserved {
                node: self.node.clone(),
                kind: kind.clone(),
                fields: fields.clone(),
            });
        }
        self.inner.event(e);
    }

    fn metric(&self, m: &Metric) {
        self.inner.metric(m);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// The pid carried by an observation's fields, when it has one.
fn pid_of(fields: &serde_json::Value) -> Option<i64> {
    fields.get("pid").and_then(serde_json::Value::as_i64)
}

/// The pids a journal spawned and never exited, in spawn order. Each `coder.exited` closes one spawn of its pid.
#[must_use]
pub fn unmatched_spawns(journal: &[(u64, RunEvent)]) -> Vec<i64> {
    let mut open: Vec<i64> = Vec::new();
    for (_, event) in journal {
        let RunEvent::NodeObserved { kind, fields, .. } = event else {
            continue;
        };
        let Some(pid) = pid_of(fields) else {
            continue;
        };
        match kind.as_str() {
            "coder.spawned" => open.push(pid),
            "coder.exited" => {
                if let Some(at) = open.iter().position(|p| *p == pid) {
                    open.remove(at);
                }
            }
            _ => {}
        }
    }
    open
}

/// Counts over every run in the store (spec 217 R8–R10). All of them are derived from journal events.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Totals {
    /// Agent calls ever spawned (`coder.spawned`).
    pub dispatches_total: u64,
    /// Agent calls refused before any spawn (`worker.start.refused`).
    pub refusals_total: u64,
    /// Agent calls whose criteria did not all pass.
    pub criteria_failures_total: u64,
    /// Spawns with no exit in a run that reached a terminal state.
    pub orphans_total: u64,
    /// The orphans above that belong to runs accepted by the current process.
    pub orphans_since_start: u64,
    /// Spawns with no exit in a run that has not reached a terminal state.
    pub agents_in_flight: u64,
}

/// Walks every run's journal and derives [`Totals`]. `process_start_ms` is the current process's start (the origin
/// stamped on every run it accepts).
///
/// # Errors
/// Returns [`PiiError`] when the store cannot be read.
pub fn store_totals(store: &Store, process_start_ms: u64) -> Result<Totals, PiiError> {
    let mut headers = Vec::new();
    store.for_each_run_header(|h| headers.push(h))?;
    let mut totals = Totals::default();
    for header in headers {
        let journal = store.read_journal(&header.run)?;
        for (_, event) in &journal {
            let RunEvent::NodeObserved { kind, fields, .. } = event else {
                continue;
            };
            match kind.as_str() {
                "coder.spawned" => totals.dispatches_total += 1,
                "worker.start.refused" => totals.refusals_total += 1,
                "coder.exited"
                    if fields
                        .get("criteria_failed")
                        .and_then(serde_json::Value::as_i64)
                        .is_some_and(|n| n > 0) =>
                {
                    totals.criteria_failures_total += 1;
                }
                _ => {}
            }
        }
        let open = u64::try_from(unmatched_spawns(&journal).len()).unwrap_or(u64::MAX);
        match header.status {
            RunStatus::Succeeded | RunStatus::Failed => {
                totals.orphans_total += open;
                if header.process_start_ms == process_start_ms {
                    totals.orphans_since_start += open;
                }
            }
            RunStatus::Running | RunStatus::Suspended => totals.agents_in_flight += open,
        }
    }
    Ok(totals)
}

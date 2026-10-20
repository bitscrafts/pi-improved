//! Spec 217: agent-call observations, recorded into the run journal.
//!
//! A node reports its `coder.spawned`, `coder.exited` and `worker.start.refused` through its context. The engine
//! records them with [`CallRecorder`] before the node's own `NodeFinished`. Every count here is derived from those
//! journal events, never from a node's returned value.

use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{NodeId, PiiError, RunEvent, RunStatus, Terminal};
use pii_core::{Metric, ObsEvent, Observer};
use pii_store::Store;

use crate::spec::GraphSpec;

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

/// One coder node's agent-call record, folded from its journal events alone (spec 221).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CallSummary {
    /// The node reached a terminal event (`NodeFinished` or `NodeFailed`) in this journal.
    pub executed: bool,
    /// The last pid the node spawned, if any.
    pub spawned: Option<i64>,
    /// The exit code of a `coder.exited` that closes a `coder.spawned` of the same pid.
    pub exit_rc: Option<i64>,
    /// True when a spawn of some pid is closed by an exit of that same pid (spec 221 R1).
    pub matched: bool,
    /// The reason of a `worker.start.refused` the node journalled; a refusal carries no pid (spec 221 R2).
    pub refused: Option<String>,
    /// The node's own last outcome was a failure that will not retry (spec 224 R4).
    pub failed: bool,
}

/// Folds `node`'s agent-call observations out of `journal`. Nothing the node returned is consulted.
#[must_use]
pub fn call_summary(journal: &[(u64, RunEvent)], node: &NodeId) -> CallSummary {
    let mut out = CallSummary::default();
    let mut spawned_pids: Vec<i64> = Vec::new();
    for (_, event) in journal {
        match event {
            RunEvent::NodeFinished { node: n, .. } if n == node => {
                out.executed = true;
                out.failed = false;
            }
            RunEvent::NodeFailed {
                node: n, retryable, ..
            } if n == node => {
                out.executed = true;
                out.failed = !retryable;
            }
            RunEvent::NodeObserved {
                node: n,
                kind,
                fields,
            } if n == node => match kind.as_str() {
                "coder.spawned" => {
                    if let Some(pid) = pid_of(fields) {
                        spawned_pids.push(pid);
                        out.spawned = Some(pid);
                    }
                }
                "coder.exited" => {
                    if let Some(pid) = pid_of(fields)
                        && spawned_pids.contains(&pid)
                    {
                        out.matched = true;
                        out.exit_rc = fields.get("rc").and_then(serde_json::Value::as_i64);
                    }
                }
                "worker.start.refused" => {
                    out.refused = Some(
                        fields
                            .get("reason")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unspecified")
                            .to_owned(),
                    );
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

/// Spec 221 R1–R5: the terminal and status a finished run is journalled with, given what its coder calls show.
///
/// A refused call makes the run `refused`; a coder node that ran with no matched spawn and exit makes a success
/// `failed` with `coder.no_call_events`; a node that says `expect_spawn = false` and made no call makes it `refused`.
#[must_use]
pub fn call_verdict(
    spec: &GraphSpec,
    journal: &[(u64, RunEvent)],
    terminal: Terminal,
) -> (Terminal, RunStatus) {
    if let Terminal::Suspended { .. } = terminal {
        return (terminal, RunStatus::Suspended);
    }
    let mut refused = false;
    let mut silent: Option<&NodeId> = None;
    let mut expected_none = false;
    for node in spec.nodes.iter().filter(|n| n.kind == "coder") {
        let calls = call_summary(journal, &node.id);
        if !calls.executed {
            continue;
        }
        if calls.refused.is_some() {
            refused = true;
        } else if !calls.matched {
            let expects = node
                .cfg
                .get("expect_spawn")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            if !expects {
                expected_none = true;
            } else if !calls.failed {
                // Spec 224 R4: a node that failed on its own account keeps its reason; only a success is judged here.
                silent.get_or_insert(&node.id);
            }
        }
    }
    if refused || expected_none {
        return (terminal, RunStatus::Refused);
    }
    if let (Some(node), Terminal::Success { .. }) = (silent, &terminal) {
        return (
            Terminal::Failure {
                error: format!(
                    "coder.no_call_events: node '{node}' finished with no coder.spawned and coder.exited for one pid"
                ),
            },
            RunStatus::Failed,
        );
    }
    let status = crate::run_helpers::terminal_status(&terminal);
    (terminal, status)
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
            RunStatus::Succeeded | RunStatus::Failed | RunStatus::Refused => {
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

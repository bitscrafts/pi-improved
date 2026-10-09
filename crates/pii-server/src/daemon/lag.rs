//! Reactive-loop lag recovery (spec 79 / 043 D6).
//!
//! The daemon's reactive loop subscribes to the bus; when the receiver lags,
//! broadcast drops events — including CONTROL events such as `RunSuspended`,
//! which arms the timer that resumes a suspended run. Dropping one used to
//! mean a suspended run never resumed, silently.
//!
//! This module holds the two helpers that make recovery possible: the stderr
//! log line (the crate has no tracing dependency) and the rescan that re-applies
//! the latest suspension of every `Suspended` run. Kept out of `daemon/mod.rs`
//! so that file stays inside the workspace's effective-line gate.

use std::sync::Arc;

use pii_abi::{RunEvent, RunStatus};

use crate::Engine;
use crate::daemon::on_suspended;
use crate::daemon::scheduler::Scheduler;

/// Daemon stderr log line (no tracing dep wired; stderr is the module's channel).
pub(super) fn log_warn(msg: &str) {
    #[allow(clippy::print_stderr)]
    {
        eprintln!("pii: {msg}");
    }
}

/// Lag recovery (spec 79): re-reacts to the latest suspension of every
/// `Suspended` run. Idempotent — the wheel is keyed by (fire time, run), so
/// re-arming an already-armed timer is a no-op.
pub(super) fn recover_suspended(engine: &Arc<Engine>, scheduler: &Arc<Scheduler>) {
    let runs = match engine.store.runs_by_status(RunStatus::Suspended) {
        Ok(runs) => runs,
        Err(e) => {
            log_warn(&format!("lag recovery: cannot list suspended runs: {e}"));
            return;
        }
    };
    for run in runs {
        let Ok(journal) = engine.store.read_journal(&run) else {
            continue;
        };
        let key = journal.iter().rev().find_map(|(_, e)| match e {
            RunEvent::Suspended { key, .. } => Some(key.0.clone()),
            _ => None,
        });
        if let Some(key) = key {
            on_suspended(engine, scheduler, run.as_str(), &key);
        }
    }
}

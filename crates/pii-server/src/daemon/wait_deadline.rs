//! Spec 397: a run suspended past its gate's `deadline_ms` fails with `wait_deadline_exceeded`, on the daemon's
//! own tick, so a killed client cannot leave a deadline unenforced.

use std::sync::Arc;
use std::time::Duration;

use pii_abi::{RunId, RunStatus};
use pii_core::Clock as _;
use pii_graph::spec::GraphSpec;

use crate::Engine;
use crate::daemon::lag::log_warn;

const TICK: Duration = Duration::from_millis(50);

/// Spawns the sweep: every tick, each pending approval is checked against its gate's declared deadline.
pub(super) fn spawn(engine: Arc<Engine>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            let engine = Arc::clone(&engine);
            let swept = tokio::task::spawn_blocking(move || sweep(&engine)).await;
            if let Err(e) = swept {
                log_warn(&format!("wait deadline sweep failed: {e}"));
            }
        }
    })
}

fn sweep(engine: &Engine) {
    let Ok(pending) = engine.approvals.pending_approvals() else {
        return;
    };
    let now = pii_core::SystemClock.now_ms();
    for a in pending {
        let run = RunId::from(a.run_id.as_str());
        let Ok(Some(header)) = engine.store.get_run(&run) else {
            continue;
        };
        if header.status != RunStatus::Suspended {
            continue;
        }
        let Some(deadline) = deadline_of(engine, &header.graph, &a.node) else {
            continue;
        };
        if now.saturating_sub(a.requested_at_ms) <= deadline {
            continue;
        }
        if let Err(e) = crate::engine::runs::fail_wait_deadline(engine, &run, &a.node, deadline) {
            log_warn(&format!("wait deadline for run {run}: {e}"));
        }
    }
}

/// The `deadline_ms` the gate `node` of `graph` declares, if any.
pub fn deadline_of(engine: &Engine, graph: &str, node: &str) -> Option<u64> {
    let bytes = engine.store.get_graph(graph).ok().flatten()?;
    let spec = GraphSpec::from_json(&bytes).ok()?;
    spec.nodes
        .iter()
        .find(|n| n.id.as_str() == node)
        .and_then(|n| n.cfg.get("deadline_ms"))
        .and_then(serde_json::Value::as_u64)
}

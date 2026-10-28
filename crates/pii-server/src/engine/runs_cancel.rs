//! Run cancellation: the refusal while a node executes (spec 189) and the explicit kill (spec 243).
//!
//! Split from `runs.rs` so neither file approaches the 450-line gate; `runs` re-exports the public names.

use std::time::Duration;

use pii_abi::{PiiError, RunId};

use super::runs::close_failed;
use crate::Engine;

/// How long a `kill` cancel waits for the killed node to leave flight (spec 243 R3).
pub const KILL_WAIT: Duration = Duration::from_secs(30);

/// Cancels a run that is not executing (spec 189): closes it as `Failed` with
/// `cancelled: <reason>` on the record and emits `run.cancelled`.
///
/// A run with a node executing is refused (`run.cancel_refused` names the node):
/// cancellation never abandons work half-done. The spec 243 kill accepts that, see
/// [`cancel_run_with`].
///
/// # Errors
/// [`PiiError::NotFound`] for an unknown run, [`PiiError::Invalid`] for a run that
/// already finished or whose node is executing, store errors from journaling.
pub fn cancel_run(engine: &Engine, run_id: &RunId, reason: &str) -> Result<(), PiiError> {
    cancel_run_with(engine, run_id, reason, false, KILL_WAIT)
}

/// [`cancel_run`] with the spec 243 kill explicit.
///
/// With `kill`, the executing node is stopped and the run is awaited for up to `wait`,
/// then closed as `cancelled: <reason> (node '<node>' abandoned)`. A node still running
/// after `wait` leaves the run open: it is never closed while its agent may still run.
///
/// # Errors
/// As [`cancel_run`], and [`PiiError::Invalid`] `did not stop` when a killed node is
/// still in flight after `wait`.
pub fn cancel_run_with(
    engine: &Engine,
    run_id: &RunId,
    reason: &str,
    kill: bool,
    wait: Duration,
) -> Result<(), PiiError> {
    let header = engine
        .store
        .get_run(run_id)?
        .ok_or_else(|| PiiError::NotFound(format!("run '{run_id}' not found")))?;
    if matches!(
        header.status,
        pii_abi::RunStatus::Succeeded | pii_abi::RunStatus::Failed | pii_abi::RunStatus::Refused
    ) {
        return Err(PiiError::Invalid(format!(
            "run '{run_id}' already finished"
        )));
    }
    let mut error = format!("cancelled: {reason}");
    let mut close = true;
    if engine.cancel_queued(run_id.as_str()) {
        // Spec 308 R3: queued in the run pool, so no node is open and nothing can be refused. The
        // claim keeps its thread from executing it; the handle goes with the cancel.
        engine.cancel(run_id.as_str());
    } else if engine.is_in_flight(run_id.as_str()) {
        let node = executing_node(engine, run_id)?;
        if !kill {
            engine.log().event(
                &pii_observe::Event::new(
                    "engine",
                    pii_observe::Level::Warn,
                    pii_observe::names::RUN_CANCEL_REFUSED,
                )
                .field("run", run_id.as_str())
                .field("node", node.as_str())
                .in_run(pii_observe::RunId(run_id.to_string())),
            );
            return Err(PiiError::Invalid(format!(
                "run '{run_id}' is executing node '{node}'; cancel refused"
            )));
        }
        // The node's own thread may close the run once it stops; then it stays as it closed.
        close = stop_executing(engine, run_id, reason, &node, wait)?;
        error = format!("cancelled: {reason} (node '{node}' abandoned)");
    }
    if close {
        close_failed(engine, run_id, error, "refused")?;
    }
    // Spec 196: a cancelled run's schedule dies with it; zero is normal (most runs have no timer).
    let disarmed = match engine.daemon_ctx() {
        Some(ctx) => ctx.timer_store.remove_run(run_id.as_str())?,
        None => 0,
    };
    engine.log().event(
        &pii_observe::Event::new(
            "engine",
            pii_observe::Level::Info,
            pii_observe::names::RUN_CANCELLED,
        )
        .field("run", run_id.as_str())
        .field("reason", reason)
        .field("disarmed", disarmed)
        .in_run(pii_observe::RunId(run_id.to_string())),
    );
    Ok(())
}

/// Trips the cancel token of the node a kill names and waits for it to leave flight
/// (spec 243 R2/R3). Returns `Ok(false)` when the run was already closed by its own
/// thread during the wait, so the kill must not append a second terminal.
fn stop_executing(
    engine: &Engine,
    run_id: &RunId,
    reason: &str,
    node: &str,
    wait: Duration,
) -> Result<bool, PiiError> {
    let event = |name: &'static str| {
        pii_observe::Event::new("engine", pii_observe::Level::Warn, name)
            .field("run", run_id.as_str())
            .field("node", node)
            .in_run(pii_observe::RunId(run_id.to_string()))
    };
    engine
        .log()
        .event(&event(pii_observe::names::RUN_CANCEL_KILLING).field("reason", reason));
    // `trip_cancel` keeps the handle: `Engine::cancel` would drop it and look stopped at once.
    engine.trip_cancel(run_id.as_str());
    let end = std::time::Instant::now() + wait;
    while engine.is_in_flight(run_id.as_str()) {
        if std::time::Instant::now() >= end {
            engine
                .log()
                .event(&event(pii_observe::names::RUN_CANCEL_KILL_TIMEOUT));
            return Err(PiiError::Invalid(format!(
                "run '{run_id}' did not stop within {wait:?} after kill"
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    engine.cancel(run_id.as_str());
    let closed_by_node = engine.store.get_run(run_id)?.is_some_and(|h| {
        matches!(
            h.status,
            pii_abi::RunStatus::Succeeded
                | pii_abi::RunStatus::Failed
                | pii_abi::RunStatus::Refused
        )
    });
    Ok(!closed_by_node)
}

/// The node a run's journal shows started and not yet ended (empty if none).
fn executing_node(engine: &Engine, run_id: &RunId) -> Result<String, PiiError> {
    use pii_abi::RunEvent;
    let mut open: Vec<String> = Vec::new();
    for (_, ev) in engine.store.read_journal(run_id)? {
        match ev {
            RunEvent::NodeStarted { node, .. } => open.push(node.to_string()),
            RunEvent::NodeFinished { node, .. }
            | RunEvent::NodeFailed { node, .. }
            | RunEvent::NodeCancelled { node }
            | RunEvent::NodeDead { node } => open.retain(|n| *n != node.to_string()),
            _ => {}
        }
    }
    Ok(open.pop().unwrap_or_default())
}

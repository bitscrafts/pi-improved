//! Store-backed run entry points (spec F4.8, F4.11) and the scheduler tick.
//!
//! One scheduler tick (pass-3 §4.3): propagate dead to a fixpoint, fail
//! loudly on any unsatisfiable join, then drain **all** ready nodes into one
//! batch (never `.pop()`), partition by parallel safety, and apply results as
//! they arrive. Every branch of the loop returns — never a hang.
//!
//! `run` journals every event to the [`pii_store::Store`] and snapshots the
//! blackboard every 32 events (see [`crate::journal`]). `resume` replays the
//! whole journal to rebuild the state — a node with `NodeFinished` is never
//! re-invoked (its value is in the journal); a node with `NodeStarted` and no
//! terminal event **is** re-run, which is the documented Phase-1 boundary.
//! Cyclic execution also lands in Phase 2: `run`/`resume` reject any cycle
//! with [`PiiError::Invalid`] even when `loop_budget` makes it lint-clean.

mod engine;
mod resume;

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;

use pii_abi::{NodeId, PiiError, ResumeKey, RunEvent, RunId, Terminal};
use pii_core::Ctx;
use pii_store::Store;

use crate::blackboard::Blackboard;
use crate::journal::Journaler;
use crate::node::NodeRegistry;
use crate::run::engine::Engine;
use crate::run_helpers::{build_nodes, fresh_tokens, validate_for_exec};
use crate::sched::state::RunState;
use crate::spec::GraphSpec;

pub use resume::resume;

/// Runs a graph to a terminal disposition, journaling every event to the
/// store under `run_id`.
///
/// # Errors
/// Returns [`PiiError::Invalid`] for cyclic specs (Phase-1 boundary) or node
/// build failures, [`PiiError::Cancelled`] when the root context is torn down
/// mid-run, and store errors from journaling.
pub fn run(
    store: &Store,
    spec: &GraphSpec,
    reg: &NodeRegistry,
    ctx: &Ctx,
    run_id: &RunId,
    input: serde_json::Value,
) -> Result<Terminal, PiiError> {
    run_with_validate(store, spec, reg, ctx, run_id, input, true)
}

/// [`run`] with an explicit validation switch.
///
/// `validate` defaults to `true` (the library always validates unless a caller
/// opts out). The CLI's `--no-validate` passes `false` so the runtime
/// `UnsatisfiableJoin` path (pass-3 Test C) can be exercised on a graph that
/// V6 rejects at load time.
///
/// # Errors
/// Returns [`PiiError::Invalid`] for cyclic specs (Phase-1 boundary), node
/// build failures, or (when `validate` is true) load-time validation errors;
/// [`PiiError::Cancelled`] when the root context is torn down mid-run; and
/// store errors from journaling.
pub fn run_with_validate(
    store: &Store,
    spec: &GraphSpec,
    reg: &NodeRegistry,
    ctx: &Ctx,
    run_id: &RunId,
    input: serde_json::Value,
    validate: bool,
) -> Result<Terminal, PiiError> {
    if validate {
        validate_for_exec(spec, reg)?;
    }
    let built = build_nodes(spec, reg)?;
    let spec_json = spec.to_json()?;
    let graph_hash = store.put_graph(&spec_json)?;
    // The run input is stored content-addressed; `Started.input_hash` is the
    // key resume uses to recover it without reading snapshots.
    let input_bytes =
        serde_json::to_vec(&input).map_err(|e| PiiError::Internal(format!("input serde: {e}")))?;
    let input_hash = store.put_graph(&input_bytes)?;

    let bb = Arc::new(RwLock::new(Blackboard::new(input)));
    let jr = Journaler::fresh(
        store,
        ctx.observer().clone(),
        run_id.clone(),
        graph_hash,
        Arc::clone(&bb),
    );
    jr.append(&RunEvent::Started {
        graph: spec.name.clone(),
        input_hash,
    })?;
    // Spec 154: the accepted run (header + `Started`) is durable before the
    // first node executes — one synchronous write per run, not per event.
    jr.flush()?;

    let tokens = fresh_tokens(ctx, spec);
    let mut st = RunState::from_spec(spec);
    let mut fin = Vec::new();
    let mut failure = None;
    let no_resume_keys: BTreeMap<NodeId, ResumeKey> = BTreeMap::new();
    let engine = Engine {
        spec,
        reg,
        root: ctx,
        run_id,
        bb: &bb,
        jr: &jr,
        built: &built,
        tokens: &tokens,
        resume_keys: &no_resume_keys,
    };
    engine.tick_loop(&mut st, &mut fin, &mut failure)
}

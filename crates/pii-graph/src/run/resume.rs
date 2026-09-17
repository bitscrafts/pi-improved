//! Journal-replay resume for store-backed runs (spec F4.11).
//!
//! `resume` replays the whole journal to rebuild the state — a node with
//! `NodeFinished` is never re-invoked (its value is in the journal); a node
//! with `NodeStarted` and no terminal event **is** re-run, which is the
//! documented Phase-1 boundary.

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;

use pii_abi::{NodeId, PiiError, ResumeKey, RunEvent, RunId, Terminal};
use pii_core::Ctx;

use crate::blackboard::Blackboard;
use crate::journal::Journaler;
use crate::node::NodeRegistry;
use crate::run::Engine;
use crate::run_helpers::{build_nodes, fresh_tokens, interrupted_nodes, replay, validate_for_exec};
use crate::sched::dispatch::FinishedNode;
use crate::sched::state::{RunState, propagate_dead};
use crate::spec::GraphSpec;

/// Resumes a run by replaying its journal, then continuing the tick loop.
///
/// `resume_key` is the specific key the caller knows just fired (e.g. the
/// daemon scheduler's due timer wheel entry); `None` for a bare/manual
/// resume (CLI, approval resolve) that has no single known key. Either way,
/// the node(s) the journal shows as still-suspended are dispatched through
/// [`crate::node::GraphNode::on_resume`] (spec 30 D1) rather than a fresh
/// `run()` — a fired timer resumes to `Next` instead of re-suspending.
///
/// # Errors
/// Returns [`PiiError::NotFound`] for an unknown run or graph body,
/// [`PiiError::Invalid`] for a cyclic spec, and store errors from journaling.
pub fn resume(
    store: &pii_store::Store,
    reg: &NodeRegistry,
    ctx: &Ctx,
    run_id: &RunId,
    resume_key: Option<&ResumeKey>,
) -> Result<Terminal, PiiError> {
    let header = store
        .get_run(run_id)?
        .ok_or_else(|| PiiError::NotFound(format!("run '{run_id}' not found")))?;
    let spec_bytes = store
        .get_graph(&header.graph)?
        .ok_or_else(|| PiiError::NotFound(format!("graph body '{}'", header.graph)))?;
    let spec = GraphSpec::from_json(&spec_bytes)?;
    validate_for_exec(&spec, reg)?;
    let journal = store.read_journal(run_id)?;

    // Recover the run input from the content-addressed store.
    let input = journal
        .iter()
        .find_map(|(_, ev)| match ev {
            RunEvent::Started { input_hash, .. } => store
                .get_graph(input_hash)
                .ok()
                .flatten()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok()),
            _ => None,
        })
        .unwrap_or(serde_json::Value::Null);

    let bb = Arc::new(RwLock::new(Blackboard::new(input.clone())));
    let jr = Journaler::adopt(store, ctx.observer().clone(), header, Arc::clone(&bb));

    // Replay the journal: rebuild state, blackboard, and completion order.
    let mut st = RunState::from_spec(&spec);
    let mut fin: Vec<FinishedNode> = Vec::new();
    let mut failure: Option<String> = None;
    let mut suspended: BTreeMap<NodeId, ResumeKey> = BTreeMap::new();
    replay(
        &journal,
        &spec,
        &mut st,
        &bb,
        &mut fin,
        &mut failure,
        &mut suspended,
    );

    // Spec 11 J3 — interrupted-turn closer: a node whose last relevant event
    // is `NodeStarted` with no later terminal event (`NodeFinished`/
    // `NodeFailed`/`NodeCancelled`/`NodeDead`) was left mid-flight by a crash
    // or cancel. Append a synthetic `NodeInterrupted` marker for each (in the
    // seq order of their last `NodeStarted`) so the journal records the
    // previous attempt's fate instead of showing an unexplained duplicate
    // `NodeStarted` after `Resumed`. Purely an audit marker: `replay` treats
    // it as a no-op and the node still re-runs, exactly as before.
    let interrupted = interrupted_nodes(&journal);
    for node in &interrupted {
        jr.append(&RunEvent::NodeInterrupted { node: node.clone() })?;
    }

    let payload_bytes =
        serde_json::to_vec(&input).map_err(|e| PiiError::Internal(format!("input serde: {e}")))?;
    // The Resumed event's key is the caller-supplied key when known (e.g.
    // the scheduler's due timer), else whichever key the journal shows as
    // still-suspended (at most one — a suspend halts the whole tick loop),
    // else empty (a bare resume of a run that was not actually suspended).
    let fired_key = resume_key
        .cloned()
        .or_else(|| suspended.values().next().cloned())
        .unwrap_or_else(|| ResumeKey(String::new()));
    jr.append(&RunEvent::Resumed {
        key: fired_key,
        payload_hash: pii_store::hash(&payload_bytes),
    })?;

    // Nodes killed by the final (unjournalled) propagation before the crash.
    let newly = propagate_dead(&spec, &mut st);
    for id in &newly {
        jr.append(&RunEvent::NodeDead { node: id.clone() })?;
    }

    let built = build_nodes(&spec, reg)?;
    let tokens = fresh_tokens(ctx, &spec);
    let engine = Engine {
        spec: &spec,
        reg,
        root: ctx,
        run_id,
        bb: &bb,
        jr: &jr,
        built: &built,
        tokens: &tokens,
        resume_keys: &suspended,
    };
    engine.tick_loop(&mut st, &mut fin, &mut failure)
}

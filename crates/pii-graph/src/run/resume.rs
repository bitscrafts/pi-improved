//! Journal-replay resume for store-backed runs (spec F4.11).
//!
//! `resume` replays the whole journal to rebuild the state — a node with
//! `NodeFinished` is never re-invoked (its value is in the journal); a node
//! with `NodeStarted` and no terminal event **is** re-run, which is the
//! documented Phase-1 boundary.

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;

use pii_abi::{NodeId, PiiError, ResumeKey, RunEvent, RunId, RunStatus, Terminal};
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
/// Spec 186: a journal holding an effect started and never confirmed is
/// REFUSED (`effect.unknown`) before anything is appended; a confirmed effect
/// whose `NodeFinished` never landed is completed from the journal, not re-run.
///
/// # Errors
/// Returns [`PiiError::NotFound`] for an unknown run or graph body,
/// [`PiiError::Invalid`] for a cyclic spec or an unknown effect, and store
/// errors from journaling.
pub fn resume(
    store: &pii_store::Store,
    reg: &NodeRegistry,
    ctx: &Ctx,
    run_id: &RunId,
    resume_key: Option<&ResumeKey>,
) -> Result<Terminal, PiiError> {
    resume_with(store, reg, ctx, run_id, resume_key, None)
}

/// Re-runs ONE node of a recorded run (spec 186, `pii run retry --node`).
///
/// Its earlier events become history, its previous output stays on the
/// blackboard as input, every other node is replayed from the journal
/// untouched. This is the operator's explicit acceptance of an `effect.unknown`.
///
/// # Errors
/// As [`resume`], plus [`PiiError::NotFound`] when the graph has no such node.
pub fn retry(
    store: &pii_store::Store,
    reg: &NodeRegistry,
    ctx: &Ctx,
    run_id: &RunId,
    node: &NodeId,
) -> Result<Terminal, PiiError> {
    resume_with(store, reg, ctx, run_id, None, Some(node))
}

/// Closes a run as failed with the operator's reason (spec 186,
/// `pii run abandon`). Appends; never rewrites. Idempotent callers get a
/// second `Finished` event, which is the record of the second call.
///
/// # Errors
/// [`PiiError::NotFound`] for an unknown run; store errors from journaling.
pub fn abandon(
    store: &pii_store::Store,
    ctx: &Ctx,
    run_id: &RunId,
    reason: &str,
) -> Result<(), PiiError> {
    let header = store
        .get_run(run_id)?
        .ok_or_else(|| PiiError::NotFound(format!("run '{run_id}' not found")))?;
    let bb = Arc::new(RwLock::new(Blackboard::new(serde_json::Value::Null)));
    let jr = Journaler::adopt(store, ctx.observer().clone(), header, bb);
    let terminal = Terminal::Failure {
        error: format!("abandoned: {reason}"),
    };
    let bytes = serde_json::to_vec(&terminal)
        .map_err(|e| PiiError::Internal(format!("terminal serde: {e}")))?;
    jr.append(&RunEvent::Finished {
        terminal,
        output_hash: pii_store::hash(&bytes),
    })?;
    jr.set_status(RunStatus::Failed);
    jr.flush()?;
    Ok(())
}

fn resume_with(
    store: &pii_store::Store,
    reg: &NodeRegistry,
    ctx: &Ctx,
    run_id: &RunId,
    resume_key: Option<&ResumeKey>,
    retry_node: Option<&NodeId>,
) -> Result<Terminal, PiiError> {
    let header = store
        .get_run(run_id)?
        .ok_or_else(|| PiiError::NotFound(format!("run '{run_id}' not found")))?;
    let spec_bytes = store
        .get_graph(&header.graph)?
        .ok_or_else(|| PiiError::NotFound(format!("graph body '{}'", header.graph)))?;
    let spec = GraphSpec::from_json(&spec_bytes)?;
    validate_for_exec(&spec, reg)?;
    let mut journal = store.read_journal(run_id)?;
    let (ledger, prev_output) = preflight(&spec, &journal, retry_node)?;

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
    append_prelude(&jr, &ledger, retry_node)?;
    jr.flush()?;
    journal = crate::effect::effective(&store.read_journal(run_id)?);

    // Replay the journal: rebuild state, blackboard, and completion order.
    let mut st = RunState::from_spec(&spec);
    let mut fin: Vec<FinishedNode> = Vec::new();
    let mut failure: Option<String> = None;
    let mut suspended: BTreeMap<NodeId, ResumeKey> = BTreeMap::new();
    if let (Some(n), Some(v)) = (retry_node, prev_output) {
        bb.write().record(n.as_str(), v);
    }
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
    // Spec 399 R9: the resumed run admits the programs its graph declares, as the run it resumes did.
    let root = ctx.with_proc_programs(&crate::sched::dispatch::graph_declared_programs(
        reg, &spec, &input,
    ));
    let engine = Engine {
        spec: &spec,
        reg,
        root: &root,
        run_id,
        bb: &bb,
        jr: &jr,
        built: &built,
        tokens: &tokens,
        resume_keys: &suspended,
    };
    engine.tick_loop(&mut st, &mut fin, &mut failure)
}

/// Checks a resume/retry against the journal BEFORE anything is appended
/// (spec 186): an unknown retry node, or an effect started and never
/// confirmed, is refused. Returns the effect ledger and, for a retry, the
/// node's previous output.
fn preflight(
    spec: &GraphSpec,
    journal: &[(u64, RunEvent)],
    retry_node: Option<&NodeId>,
) -> Result<(crate::effect::Ledger, Option<serde_json::Value>), PiiError> {
    if let Some(n) = retry_node
        && !spec.nodes.iter().any(|s| &s.id == n)
    {
        return Err(PiiError::NotFound(format!("node '{n}' not in graph")));
    }
    let ledger = crate::effect::scan(journal);
    if retry_node.is_none()
        && let Some(refused) = crate::effect::refusal(&ledger)
    {
        return Err(refused);
    }
    let prev_output = retry_node.and_then(|n| {
        journal.iter().rev().find_map(|(_, ev)| match ev {
            RunEvent::NodeFinished { node, value, .. } if node == n => Some(value.clone()),
            _ => None,
        })
    });
    Ok((ledger, prev_output))
}

/// Appends the replay's own record: the retry marker, or the `NodeFinished`
/// of every effect that was confirmed but never finished (reused, not re-run).
fn append_prelude(
    jr: &Journaler<'_>,
    ledger: &crate::effect::Ledger,
    retry_node: Option<&NodeId>,
) -> Result<(), PiiError> {
    if let Some(n) = retry_node {
        jr.append(&RunEvent::NodeRetried { node: n.clone() })?;
        return Ok(());
    }
    for (node, (port, value)) in &ledger.orphans {
        let value_hash = pii_store::hash(&serde_json::to_vec(value).unwrap_or_default());
        jr.append(&RunEvent::NodeFinished {
            node: node.clone(),
            port: port.clone(),
            value: value.clone(),
            value_hash,
            at_ms: 0,
            dur_ms: 0,
        })?;
    }
    Ok(())
}

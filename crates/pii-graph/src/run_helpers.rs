//! Free helper functions for the run engine (kept out of `run.rs` so that
//! file stays under the 450-line budget).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use parking_lot::RwLock;

use pii_abi::{NodeId, PiiError, ResumeKey, RunEvent, RunStatus, Terminal};
use pii_core::Ctx;

use crate::blackboard::Blackboard;
use crate::node::{GraphNode, NodeRegistry};
use crate::sched::dispatch::FinishedNode;
use crate::sched::state::{Readiness, RunState, readiness};
use crate::spec::{EdgeKind, GraphSpec};
use crate::validate::validate;

/// Runs `validate()` and maps any errors to a single `PiiError::Invalid`.
///
/// The engine must never store or execute a graph that fails load-time
/// validation (pass-1 §3.2: *"`validate()` runs before a graph can be stored or
/// executed"*). The CLI's `--no-validate` skips its own pre-flight only; the
/// library entry points always validate.
pub(crate) fn validate_for_exec(spec: &GraphSpec, reg: &NodeRegistry) -> Result<(), PiiError> {
    if let Err(errors) = validate(spec, reg) {
        return Err(PiiError::Invalid(format!(
            "graph failed validation: {}",
            errors.join("; ")
        )));
    }
    Ok(())
}

/// Builds every node instance up front (build errors surface before journalling).
pub(crate) fn build_nodes(
    spec: &GraphSpec,
    reg: &NodeRegistry,
) -> Result<BTreeMap<NodeId, Box<dyn GraphNode>>, PiiError> {
    let mut built = BTreeMap::new();
    for n in &spec.nodes {
        let factory = reg.factory(&n.kind)?;
        let node = factory.build(&n.cfg)?;
        built.insert(n.id.clone(), node);
    }
    Ok(built)
}

/// A per-node cancel token, all children of the run's root token.
pub(crate) type CancelTokenSet = BTreeMap<NodeId, pii_core::CancelToken>;

pub(crate) fn fresh_tokens(ctx: &Ctx, spec: &GraphSpec) -> CancelTokenSet {
    spec.nodes
        .iter()
        .map(|n| (n.id.clone(), ctx.cancel_token().child()))
        .collect()
}

/// Detects nodes left mid-flight by a crash/cancel (spec 11 J3): a node with
/// a `NodeStarted` but no later terminal event (`NodeFinished`/`NodeFailed`/
/// `NodeCancelled`/`NodeDead`) anywhere in the journal. Returns the nodes in
/// the seq order of their LAST `NodeStarted` (deterministic). The caller
/// appends a synthetic `NodeInterrupted` marker for each on resume.
#[must_use]
pub(crate) fn interrupted_nodes(journal: &[(u64, RunEvent)]) -> Vec<NodeId> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut terminal: BTreeSet<NodeId> = BTreeSet::new();
    let mut last_start: BTreeMap<NodeId, u64> = BTreeMap::new();
    for (seq, ev) in journal {
        match ev {
            RunEvent::NodeStarted { node, .. } => {
                last_start.insert(node.clone(), *seq);
            }
            RunEvent::NodeFinished { node, .. }
            | RunEvent::NodeFailed { node, .. }
            | RunEvent::NodeCancelled { node }
            | RunEvent::NodeDead { node } => {
                terminal.insert(node.clone());
            }
            _ => {}
        }
    }
    let mut dangling: Vec<(u64, NodeId)> = last_start
        .into_iter()
        .filter(|(id, _)| !terminal.contains(id))
        .map(|(id, seq)| (seq, id))
        .collect();
    dangling.sort_by_key(|(seq, _)| *seq);
    dangling.into_iter().map(|(_, id)| id).collect()
}

/// Replays a journal into the state (spec F4.11). Nodes that eventually
/// finished have their failed attempts skipped; a `NodeStarted` without a
/// terminal event is simply ignored — that node re-runs.
///
/// `suspended` collects, per node, the key of its LATEST `Suspended` event
/// among those with no later terminal event (spec 30 D1): the tick loop
/// dispatches these nodes through [`crate::node::GraphNode::on_resume`]
/// instead of a fresh `run()`, so a fired timer resumes to `Next` rather
/// than re-suspending with a new key.
pub(crate) fn replay(
    journal: &[(u64, RunEvent)],
    spec: &GraphSpec,
    st: &mut RunState,
    bb: &Arc<RwLock<Blackboard>>,
    fin: &mut Vec<FinishedNode>,
    failure: &mut Option<String>,
    suspended: &mut BTreeMap<NodeId, ResumeKey>,
) {
    let finished: BTreeSet<NodeId> = journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::NodeFinished { node, .. } => Some(node.clone()),
            _ => None,
        })
        .collect();
    for (_, ev) in journal {
        match ev {
            RunEvent::NodeFinished {
                node, port, value, ..
            } => {
                // Replay of an already-journaled completion: the activation
                // outcome is irrelevant, only the state mutation matters.

                #[allow(clippy::let_underscore_must_use)] // replay: activation outcome irrelevant
                let _ = crate::sched::state::apply_completion(spec, st, node, port, value);
                bb.write().record(node.as_str(), value.clone());
                fin.push(FinishedNode {
                    node: node.clone(),
                    port: port.clone(),
                    value: value.clone(),
                });
                suspended.remove(node);
            }
            RunEvent::NodeDead { node } => {
                st.dead.insert(node.clone());
                crate::sched::state::apply_failed_or_cancelled(spec, st, node);
                suspended.remove(node);
            }
            RunEvent::NodeCancelled { node } => {
                st.cancelled.insert(node.clone());
                crate::sched::state::apply_failed_or_cancelled(spec, st, node);
                suspended.remove(node);
            }
            RunEvent::NodeFailed {
                node,
                error,
                retryable: false,
                ..
            } if !finished.contains(node) && failure.is_none() => {
                // A resolved node's failed attempts are history, not state.
                st.failed.insert(node.clone());
                crate::sched::state::apply_failed_or_cancelled(spec, st, node);
                suspended.remove(node);
                *failure = Some(error.clone());
            }
            RunEvent::Suspended { node, key, .. } => {
                suspended.insert(node.clone(), key.clone());
            }
            _ => {}
        }
    }
}

/// Terminal disposition when no node is ready (spec F4.8).
pub(crate) fn terminal_determination(
    spec: &GraphSpec,
    st: &RunState,
    fin: &[FinishedNode],
    failure: Option<&str>,
) -> Terminal {
    if let Some(err) = failure {
        return Terminal::Failure {
            error: err.to_string(),
        };
    }
    if let Some(n) = spec.nodes.iter().find(|n| {
        !st.executed.contains(&n.id)
            && !st.dead.contains(&n.id)
            && !st.failed.contains(&n.id)
            && !st.cancelled.contains(&n.id)
            && readiness(spec, st, &n.id) == Readiness::NotReady
    }) {
        return Terminal::Failure {
            error: format!("no progress: node '{}' never became ready", n.id),
        };
    }
    // Output = the latest finished node with no outbound Data edge on the
    // port it took (latest by seq if several).
    for f in fin.iter().rev() {
        let has_out = spec.outbound(&f.node).iter().any(|&ei| {
            spec.edges
                .get(ei)
                .is_some_and(|e| e.kind == EdgeKind::Data && e.port == f.port)
        });
        if !has_out {
            return Terminal::Success {
                output: f.value.clone(),
            };
        }
    }
    if let Some(last) = fin.last() {
        return Terminal::Success {
            output: last.value.clone(),
        };
    }
    Terminal::Success {
        output: serde_json::Value::Null,
    }
}

pub(crate) fn terminal_status(terminal: &Terminal) -> RunStatus {
    match terminal {
        Terminal::Success { .. } => RunStatus::Succeeded,
        Terminal::Failure { .. } => RunStatus::Failed,
        Terminal::Suspended { .. } => RunStatus::Suspended,
    }
}

/// True when the spec contains a cycle (SCC with >1 node, or a self-loop).
#[must_use]
pub fn spec_has_cycle(spec: &GraphSpec) -> bool {
    let mut g = petgraph::graph::DiGraph::new();
    let mut idx: BTreeMap<NodeId, petgraph::graph::NodeIndex> = BTreeMap::new();
    for n in &spec.nodes {
        idx.insert(n.id.clone(), g.add_node(()));
    }
    for e in &spec.edges {
        if let (Some(&a), Some(&b)) = (idx.get(&e.from), idx.get(&e.to)) {
            g.add_edge(a, b, ());
        }
    }
    petgraph::algo::tarjan_scc(&g)
        .iter()
        .any(|scc| scc.len() > 1 || scc.first().is_some_and(|&n| g.find_edge(n, n).is_some()))
}

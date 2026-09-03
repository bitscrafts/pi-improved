//! Node dispatch (spec F4.9): worker execution, retry, cancellation, and
//! `cancel_losers`.
//!
//! Borrow split: [`Env`] is shared `&`-only with worker threads. The
//! [`RunState`] is mutated **only** by the receiving loop and is therefore
//! never captured by a thread. Journal events are appended by whichever
//! thread holds them at that moment through the shared
//! [`Journaler`](crate::journal::Journaler) (a brief redb txn per event —
//! never held across a dispatch).

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;

use pii_abi::{NodeId, NodeInput, Outcome, PortName, ResumeKey, RunEvent, RunId};
use pii_core::{BudgetShare, CancelToken, CapKind, CapSet, Ctx, Principal};

use crate::blackboard::Blackboard;
use crate::journal::Journaler;
use crate::sched::state::{
    EdgeState, Readiness, RunState, apply_completion, apply_failed_or_cancelled, readiness,
};
use crate::spec::{EdgeKind, GraphSpec, JoinPolicy, NodeSpec};

/// Everything a dispatch needs, shared `&`-only across worker threads.
pub struct Env<'a> {
    /// The graph being run.
    pub spec: &'a GraphSpec,
    /// The node registry (factory lookups).
    pub reg: &'a crate::node::NodeRegistry,
    /// Built node instances keyed by id.
    pub built: &'a BTreeMap<NodeId, Box<dyn crate::node::GraphNode>>,
    /// The run's root context.
    pub root: &'a Ctx,
    /// Per-node cancel tokens (children of the root token).
    pub tokens: &'a BTreeMap<NodeId, CancelToken>,
    /// Per-node inputs, built by the scheduler from the tick's state.
    pub inputs: &'a BTreeMap<NodeId, NodeInput>,
    /// The journaler (shared, mutex-guarded header inside).
    pub jr: &'a Journaler<'a>,
    /// The run id.
    pub run: &'a RunId,
    /// The blackboard behind a [`RwLock`].
    pub bb: &'a Arc<RwLock<Blackboard>>,
}

/// A node's dispatch outcome, sent from a worker to the applying loop.
#[derive(Debug)]
pub enum DispatchOutcome {
    /// Completed on a port with a value (`NodeFinished` journalled).
    Next {
        /// The port taken.
        port: PortName,
        /// The value produced.
        value: serde_json::Value,
    },
    /// Failed terminally (non-retryable) — `NodeFailed` journalled.
    Failed {
        /// The failure description.
        error: String,
    },
    /// Cancelled as a race loser — `NodeCancelled` journalled.
    Cancelled,
    /// Suspended the run — `Suspended` journalled + snapshot + resume key.
    Suspended {
        /// The resume key.
        key: ResumeKey,
    },
    /// The run was torn down at the root (crash simulation): only
    /// `NodeStarted` is in the journal for this node — resume re-runs it.
    Aborted,
}

/// A completed node, kept in completion order for terminal-output selection.
#[derive(Debug, Clone)]
pub struct FinishedNode {
    /// The node id.
    pub node: NodeId,
    /// The port it took.
    pub port: PortName,
    /// The value it produced.
    pub value: serde_json::Value,
}

/// Node capability set: the run root's kinds narrowed by the graph's declared
/// caps and this node's bits, keeping the root's path/string scopes.
///
/// F5.4: the graph's caps are intersected with the root, never unioned. Specs
/// carry bits only; the CLI grants scopes at run time, and scopes must survive
/// node attenuation.
#[must_use]
pub fn node_capset(root_caps: &CapSet, graph_bits: u32, node_bits: u32) -> CapSet {
    let mut cs = CapSet::empty();
    cs.kinds = root_caps.kinds
        & CapKind::from_bits_truncate(graph_bits)
        & CapKind::from_bits_truncate(node_bits);
    cs.fs_read = root_caps.fs_read.clone();
    cs.fs_write = root_caps.fs_write.clone();
    cs.net_hosts = root_caps.net_hosts.clone();
    cs.proc_argv0 = root_caps.proc_argv0.clone();
    cs.secrets = root_caps.secrets.clone();
    cs
}

/// Builds the join payload for a node: an object keyed by source node id
/// holding the value of every `Fired` inbound Data edge. A node with no
/// inbound Data edges gets the run input.
#[must_use]
pub fn build_node_input(
    spec: &GraphSpec,
    st: &RunState,
    bb: &Blackboard,
    id: &NodeId,
) -> NodeInput {
    let in_data = spec.inbound(id, EdgeKind::Data);
    let payload = if in_data.is_empty() {
        bb.input.clone()
    } else {
        let mut obj = serde_json::Map::new();
        for &ei in &in_data {
            if let (Some(edge), Some(EdgeState::Fired { value })) =
                (spec.edges.get(ei), st.edges.get(ei))
            {
                obj.insert(edge.from.as_str().to_string(), value.clone());
            }
        }
        serde_json::Value::Object(obj)
    };
    NodeInput {
        input: bb.input.clone(),
        payload,
        nodes: serde_json::to_value(&bb.nodes).unwrap_or(serde_json::Value::Null),
    }
}

/// Runs one node (with retries) on the calling thread, journaling every
/// attempt. Used both by parallel workers and the sequential partition.
// Kept as one function for reviewability of the retry/backoff and the four
// outcome arms; splitting would cost more than the line budget saves.
#[allow(clippy::too_many_lines)]
pub fn dispatch(env: &Env<'_>, ns: &NodeSpec) -> DispatchOutcome {
    let Some(node) = env.built.get(&ns.id) else {
        return DispatchOutcome::Failed {
            error: format!("node '{}' not built", ns.id),
        };
    };
    let Some(token) = env.tokens.get(&ns.id) else {
        return DispatchOutcome::Failed {
            error: format!("node '{}' has no cancel token", ns.id),
        };
    };
    let Some(input) = env.inputs.get(&ns.id) else {
        return DispatchOutcome::Failed {
            error: format!("node '{}' has no input", ns.id),
        };
    };
    let caps = node_capset(env.root.caps(), env.spec.caps, ns.caps);
    let ctx = env.root.child_with_token(
        Principal::GraphNode {
            run: env.run.clone(),
            node: ns.id.clone(),
        },
        &caps,
        // Same-process nodes share the run's budget (opus review F2): carving
        // `All` per node would drain the parent on the first dispatch and
        // starve every later node. Carving is for subgraph/worker boundaries.
        &BudgetShare::Shared,
        token.clone(),
    );

    let max_retries = ns.retry.max;
    let mut attempt: u32 = 1;
    loop {
        // Journal NodeStarted BEFORE running: a node interrupted mid-flight
        // is visible as "may have executed" (spec F4.9).
        let at_ms = ctx.elapsed_ms();
        if let Err(e) = env.jr.append(&RunEvent::NodeStarted {
            node: ns.id.clone(),
            at_ms,
        }) {
            return DispatchOutcome::Failed {
                error: e.to_string(),
            };
        }
        let result = node.run(&ctx, input);
        let dur_ms = ctx.elapsed_ms().saturating_sub(at_ms);
        match result {
            Ok(Outcome::Next { port, value }) => {
                let value_hash = hash_json(&value);
                if let Err(e) = env.jr.append(&RunEvent::NodeFinished {
                    node: ns.id.clone(),
                    port: port.clone(),
                    value: value.clone(),
                    value_hash,
                    at_ms,
                    dur_ms,
                }) {
                    return DispatchOutcome::Failed {
                        error: e.to_string(),
                    };
                }
                return DispatchOutcome::Next { port, value };
            }
            Ok(Outcome::Suspend { reason, key }) => {
                if let Err(e) = env.jr.append(&RunEvent::Suspended {
                    node: ns.id.clone(),
                    reason,
                    key: key.clone(),
                }) {
                    return DispatchOutcome::Failed {
                        error: e.to_string(),
                    };
                }
                if let Err(e) = env.jr.store().put_resume_key(&key, env.run) {
                    return DispatchOutcome::Failed {
                        error: e.to_string(),
                    };
                }
                // Snapshot unconditionally at the Suspended seq.
                let _ = env.jr.snapshot_now();
                return DispatchOutcome::Suspended { key };
            }
            Ok(Outcome::Fail { error }) => {
                return journal_failure(env, &ns.id, &error, attempt, false);
            }
            Err(e) if e.is_cancelled() => {
                if env.root.cancelled() {
                    // Run-level teardown: the run is being aborted, so the
                    // node stays "started but unfinished" — resume replays
                    // and re-runs it. Nothing is journalled here.
                    return DispatchOutcome::Aborted;
                }
                // Race-loser cancellation (not a failure, never retried).
                if let Err(e) = env.jr.append(&RunEvent::NodeCancelled {
                    node: ns.id.clone(),
                }) {
                    return DispatchOutcome::Failed {
                        error: e.to_string(),
                    };
                }
                return DispatchOutcome::Cancelled;
            }
            Err(e) => {
                // Retry only genuinely transient errors, while attempts remain.
                let retryable = e.is_retryable() && attempt <= max_retries;
                let outcome = journal_failure(env, &ns.id, &e.to_string(), attempt, retryable);
                if retryable {
                    attempt += 1;
                    continue;
                }
                return outcome;
            }
        }
    }
}

fn journal_failure(
    env: &Env<'_>,
    id: &NodeId,
    error: &str,
    attempt: u32,
    retryable: bool,
) -> DispatchOutcome {
    if let Err(e) = env.jr.append(&RunEvent::NodeFailed {
        node: id.clone(),
        error: error.to_string(),
        retryable,
        attempt,
    }) {
        return DispatchOutcome::Failed {
            error: e.to_string(),
        };
    }
    DispatchOutcome::Failed {
        error: error.to_string(),
    }
}

/// Applies a dispatch outcome to the state (receiving loop only).
/// Returns the resume key when the node suspended the run.
pub fn apply(
    env: &Env<'_>,
    st: &mut RunState,
    fin: &mut Vec<FinishedNode>,
    failure: &mut Option<String>,
    node: &NodeId,
    out: DispatchOutcome,
) -> Option<ResumeKey> {
    match out {
        DispatchOutcome::Next { port, value } => {
            env.bb.write().record(node.as_str(), value.clone());
            apply_completion(env.spec, st, node, &port, &value);
            fin.push(FinishedNode {
                node: node.clone(),
                port,
                value,
            });
            None
        }
        DispatchOutcome::Failed { error } => {
            apply_failed_or_cancelled(env.spec, st, node);
            st.failed.insert(node.clone());
            if failure.is_none() {
                *failure = Some(error);
            }
            None
        }
        DispatchOutcome::Cancelled => {
            st.cancelled.insert(node.clone());
            apply_failed_or_cancelled(env.spec, st, node);
            None
        }
        DispatchOutcome::Suspended { key } => Some(key),
        DispatchOutcome::Aborted => None,
    }
}

/// Cancels the still-pending feeders of every ready `Any:cancel` / `Quorum:
/// cancel` joiner. Called AFTER every single applied result (spec F4.9).
pub fn cancel_losers(env: &Env<'_>, st: &RunState) {
    for j in &env.spec.nodes {
        let wants_cancel = matches!(
            j.join,
            JoinPolicy::Any {
                cancel_losers: true
            } | JoinPolicy::Quorum {
                cancel_losers: true,
                ..
            }
        );
        if !wants_cancel
            || st.executed.contains(&j.id)
            || st.dead.contains(&j.id)
            || readiness(env.spec, st, &j.id) != Readiness::Ready
        {
            continue;
        }
        for &ei in &env.spec.inbound(&j.id, EdgeKind::Data) {
            let Some(edge) = env.spec.edges.get(ei) else {
                continue;
            };
            let pending = st.edges.get(ei).is_none_or(|s| *s == EdgeState::Pending);
            if pending && let Some(tok) = env.tokens.get(&edge.from) {
                tok.cancel();
            }
        }
    }
}

/// blake3 hash of a JSON value's serialisation (journal provenance).
fn hash_json(v: &serde_json::Value) -> String {
    serde_json::to_vec(v).map_or_else(|_| String::new(), |bytes| pii_store::hash(&bytes))
}

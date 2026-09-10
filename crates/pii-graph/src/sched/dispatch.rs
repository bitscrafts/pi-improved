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
    /// Nodes the journal shows as still-suspended, keyed to their latest
    /// resume key (spec 30 D1). Empty for a fresh run; populated by
    /// `run::resume`'s journal replay.
    pub resume_keys: &'a BTreeMap<NodeId, ResumeKey>,
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
        // Collect the fired inbound values. A node with exactly ONE inbound
        // Data edge in the SPEC receives that value directly as `$.payload`
        // — `route`/`transform` nodes fed by a single source address it as
        // `$.payload.exit_code` etc. (spec F6.2's gate shape, every starter
        // graph, Phase-2's verify-fix all rely on this). A node whose SPEC
        // declares several inbound edges is a join: its payload stays the
        // object keyed by source node id so each arm is addressable — even
        // when only one arm fired (race/any:cancel semantics, diamond.rs).
        let mut fired: Vec<(&NodeId, &serde_json::Value)> = Vec::new();
        for &ei in &in_data {
            if let (Some(edge), Some(EdgeState::Fired { value })) =
                (spec.edges.get(ei), st.edges.get(ei))
            {
                fired.push((&edge.from, value));
            }
        }
        if in_data.len() == 1 {
            fired
                .first()
                .map_or(serde_json::Value::Null, |(_from, value)| (*value).clone())
        } else {
            let mut obj = serde_json::Map::new();
            for (from, value) in fired {
                obj.insert(from.as_str().to_string(), value.clone());
            }
            serde_json::Value::Object(obj)
        }
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

    // Spec 30 D1: a node the journal shows as still-suspended gets one
    // `on_resume` check before falling back to a fresh `run()`. A timer
    // whose wheel entry has fired completes here (`Next{"fired"}`) instead
    // of re-suspending with a brand-new key.
    if let Some(key) = env.resume_keys.get(&ns.id)
        && let Some(outcome) = node.on_resume(&ctx, input, key)
    {
        let at_ms = ctx.elapsed_ms();
        if let Err(e) = env.jr.append(&RunEvent::NodeStarted {
            node: ns.id.clone(),
            at_ms,
        }) {
            return DispatchOutcome::Failed {
                error: e.to_string(),
            };
        }
        return match outcome {
            Outcome::Fail { error } => journal_failure(env, &ns.id, &error, 1, false),
            other => journal_next_or_suspend(env, node.as_ref(), &ns.id, other, at_ms, 0),
        };
    }

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
            Ok(outcome @ (Outcome::Next { .. } | Outcome::Suspend { .. })) => {
                return journal_next_or_suspend(env, node.as_ref(), &ns.id, outcome, at_ms, dur_ms);
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

/// Journals a `Next` or `Suspend` outcome, shared by the normal `run()` path
/// and the spec-30 `on_resume` path.
fn journal_next_or_suspend(
    env: &Env<'_>,
    node: &dyn crate::node::GraphNode,
    id: &NodeId,
    outcome: Outcome,
    at_ms: u32,
    dur_ms: u32,
) -> DispatchOutcome {
    match outcome {
        Outcome::Next { port, value } => {
            let value_hash = hash_json(&value);
            if let Err(e) = env.jr.append(&RunEvent::NodeFinished {
                node: id.clone(),
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
            // Spec 32 R1: extra journal events a node wants alongside its own
            // NodeFinished (e.g. `spawn`'s parent→child `RunSpawned` link),
            // appended through the SAME journaler so they share its seq
            // counter rather than racing a second writer.
            for extra in node.link_events(&value) {
                if let Err(e) = env.jr.append(&extra) {
                    return DispatchOutcome::Failed {
                        error: e.to_string(),
                    };
                }
            }
            DispatchOutcome::Next { port, value }
        }
        Outcome::Suspend { reason, key } => {
            if let Err(e) = env.jr.append(&RunEvent::Suspended {
                node: id.clone(),
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
            DispatchOutcome::Suspended { key }
        }
        Outcome::Fail { error } => journal_failure(env, id, &error, 1, false),
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
            if let Err(e) = apply_completion(env.spec, st, node, &port, &value) {
                // F4.5 loop-budget violation: a terminal failure.
                apply_failed_or_cancelled(env.spec, st, node);
                st.failed.insert(node.clone());
                if failure.is_none() {
                    *failure = Some(e);
                }
                return None;
            }
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
///
/// Cancellation is transitive within the losing arm (opus review F8): when a
/// feeder is still pending, its own data-ancestors are cancelled too, so a
/// multi-node losing arm (`fanout -> {slowA -> slowB, fast} -> use`) stops
/// paying for the whole arm, not just the node directly feeding the joiner.
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
            if pending {
                cancel_arm(env, st, &edge.from);
            }
        }
    }
}

/// Cancels `node`'s token and, transitively, every data-ancestor of `node`
/// that is still pending (part of the same losing arm). Stops at nodes that
/// have already fired or are dead/executed.
fn cancel_arm(env: &Env<'_>, st: &RunState, node: &NodeId) {
    if let Some(tok) = env.tokens.get(node) {
        tok.cancel();
    }
    for &ei in &env.spec.inbound(node, EdgeKind::Data) {
        let Some(edge) = env.spec.edges.get(ei) else {
            continue;
        };
        let pending = st.edges.get(ei).is_none_or(|s| *s == EdgeState::Pending);
        if pending {
            cancel_arm(env, st, &edge.from);
        }
    }
}

/// blake3 hash of a JSON value's serialisation (journal provenance).
fn hash_json(v: &serde_json::Value) -> String {
    serde_json::to_vec(v).map_or_else(|_| String::new(), |bytes| pii_store::hash(&bytes))
}

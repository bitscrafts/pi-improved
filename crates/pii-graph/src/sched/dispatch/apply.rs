//! Applying a dispatch outcome to the run state and cancelling losing arms (split from `dispatch.rs` for the 450-line gate).

use pii_abi::{NodeId, ResumeKey};

use super::{DispatchOutcome, Env, FinishedNode};
use crate::sched::state::{
    EdgeState, Readiness, RunState, apply_completion, apply_failed_or_cancelled, readiness,
};
use crate::spec::{EdgeKind, JoinPolicy};

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

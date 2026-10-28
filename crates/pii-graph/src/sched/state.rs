//! Scheduler run-state machine (spec F4.7, pass-3 §4.3).

use std::collections::{BTreeMap, BTreeSet};

use pii_abi::NodeId;
use serde::{Deserialize, Serialize};

use crate::spec::{EdgeKind, GraphSpec, JoinPolicy};

/// State of a single edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeState {
    /// Not yet settled.
    Pending,
    /// Produced a value on its port.
    Fired {
        /// The value carried.
        value: serde_json::Value,
    },
    /// Dead (branch never fires; data edges are killed by dead propagation).
    Dead,
}

/// Readiness verdict for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// Not ready yet (waiting on more inbound edges).
    NotReady,
    /// Ready to run now.
    Ready,
    /// Dead — will never run.
    DeadNode,
    /// A join that can never be satisfied.
    UnsatisfiableJoin,
}

/// Durable run-scheduling state.
#[derive(Debug, Clone, Default)]
pub struct RunState {
    /// Per-edge state, parallel to `spec.edges`.
    pub edges: Vec<EdgeState>,
    /// Nodes that have run to completion.
    pub executed: BTreeSet<NodeId>,
    /// Nodes marked dead.
    pub dead: BTreeSet<NodeId>,
    /// Nodes cancelled.
    pub cancelled: BTreeSet<NodeId>,
    /// Nodes that failed terminally.
    pub failed: BTreeSet<NodeId>,
    /// Per-node loop re-entry count (F4.5), keyed by node id.
    pub iters: BTreeMap<NodeId, u32>,
    /// Completed entries of each call head `<call>__in` (spec 306), keyed by head id; cumulative for the run.
    pub entries: BTreeMap<NodeId, u32>,
}

impl RunState {
    /// Builds a fresh state with all edges pending.
    #[must_use]
    pub fn from_spec(spec: &GraphSpec) -> Self {
        Self {
            edges: vec![EdgeState::Pending; spec.edges.len()],
            executed: BTreeSet::new(),
            dead: BTreeSet::new(),
            cancelled: BTreeSet::new(),
            failed: BTreeSet::new(),
            iters: BTreeMap::new(),
            entries: BTreeMap::new(),
        }
    }
}

/// Spec 306 R2: the refusal for entering call head `n` once more, when its completed entries already reach the
/// `max_entries` its node carries. `None` for a node with no limit or one that may still be entered.
#[must_use]
pub fn entry_refusal(spec: &GraphSpec, st: &RunState, n: &NodeId) -> Option<String> {
    let max = spec.node(n)?.cfg.get("max_entries")?.as_u64()?;
    let done = u64::from(st.entries.get(n).copied().unwrap_or(0));
    if done < max {
        return None;
    }
    let call = n.as_str().strip_suffix("__in").unwrap_or(n.as_str());
    Some(format!(
        "call '{call}' entered more than max_entries ({max}) times"
    ))
}

fn is_fired(e: &EdgeState) -> bool {
    matches!(e, EdgeState::Fired { .. })
}

/// Computes the total readiness of a node (spec F4.7).
///
/// Order matters:
///
/// 1. If any inbound **Order** edge is `Pending` → `NotReady`.
/// 2. If the node has zero inbound Data edges → `Ready` (entry node!).
///    This must be written explicitly FIRST among the join arms — `all()`
///    over an empty slice is `true`, which would otherwise report the entry
///    as dead.
/// 3. `All`: any `Pending` → `NotReady`; all `Dead` → `DeadNode`; any
///    `Dead` → `UnsatisfiableJoin`; else `Ready`.
/// 4. `Any`: any `Fired` → `Ready`; all `Dead` → `DeadNode`; else `NotReady`.
/// 5. `Quorum{k}`: `Fired` count ≥ k → `Ready`; else if `Fired`+`Pending` < k
///    → `UnsatisfiableJoin`; else `NotReady`.
/// 6. `AllSettled`: any `Pending` → `NotReady`; all `Dead` → `DeadNode`;
///    else `Ready`.
#[must_use]
pub fn readiness(spec: &GraphSpec, st: &RunState, n: &NodeId) -> Readiness {
    // The entry node is ready on its first pass even when it has inbound
    // edges (a loop back-edge from a downstream node is Pending initially).
    // This is what lets a cyclic graph's entry start the run (F4.5).
    if n == &spec.entry && !st.executed.contains(n) {
        return Readiness::Ready;
    }

    // Order edges gate readiness first: wait for settle.
    for (i, e) in spec.edges.iter().enumerate() {
        if e.to == *n
            && e.kind == EdgeKind::Order
            && st.edges.get(i).is_none_or(|s| *s == EdgeState::Pending)
        {
            return Readiness::NotReady;
        }
    }

    let in_data = spec.inbound(n, EdgeKind::Data);
    // Zero inbound Data edges → always ready. Must precede join arms.
    if in_data.is_empty() {
        return Readiness::Ready;
    }

    let Some(node) = spec.node(n) else {
        return Readiness::DeadNode;
    };

    let fired = |i: usize| st.edges.get(i).is_some_and(is_fired);
    let pending = |i: usize| st.edges.get(i).is_none_or(|e| *e == EdgeState::Pending);
    let dead_edge = |i: usize| st.edges.get(i).is_some_and(|e| *e == EdgeState::Dead);

    match &node.join {
        JoinPolicy::All => {
            if in_data.iter().any(|&i| pending(i)) {
                Readiness::NotReady
            } else if in_data.iter().all(|&i| dead_edge(i)) {
                Readiness::DeadNode
            } else if in_data.iter().any(|&i| dead_edge(i)) {
                Readiness::UnsatisfiableJoin
            } else {
                Readiness::Ready
            }
        }
        JoinPolicy::Any { .. } => {
            if in_data.iter().any(|&i| fired(i)) {
                Readiness::Ready
            } else if in_data.iter().all(|&i| dead_edge(i)) {
                Readiness::DeadNode
            } else {
                Readiness::NotReady
            }
        }
        JoinPolicy::Quorum { k, .. } => {
            let fired_count = in_data.iter().filter(|&&i| fired(i)).count();
            let pending_count = in_data.iter().filter(|&&i| pending(i)).count();
            let k = usize::from(*k);
            if fired_count >= k {
                Readiness::Ready
            } else if fired_count + pending_count < k {
                Readiness::UnsatisfiableJoin
            } else {
                Readiness::NotReady
            }
        }
        JoinPolicy::AllSettled => {
            if in_data.iter().any(|&i| pending(i)) {
                Readiness::NotReady
            } else if in_data.iter().all(|&i| dead_edge(i)) {
                Readiness::DeadNode
            } else {
                Readiness::Ready
            }
        }
    }
}

/// Applies the edge transitions when node `n` completes on port `P` with `value`.
///
/// F4.5 loop re-entry: if an outbound edge targets a node already in
/// `executed`, that target must have a `loop_budget`. The budget is
/// incremented; exceeding it is a terminal failure. Otherwise the target and
/// its downstream reachable set are reset (removed from `executed`/`dead`,
/// their outbound edges back to `Pending`) so the cycle can iterate.
///
/// # Errors
/// Returns `Err` on re-entry into a node without a `loop_budget`, or when a
/// `loop_budget` is exceeded.
pub fn apply_completion(
    spec: &GraphSpec,
    st: &mut RunState,
    n: &NodeId,
    port: &pii_abi::PortName,
    value: &serde_json::Value,
) -> Result<(), String> {
    // Mark `n` executed before the re-entry reset below: a reset that walks back round a cycle to `n` must clear it
    // (spec 306: otherwise `n` stays executed, and its next pass re-enters the loop head once more).
    st.executed.insert(n.clone());
    // Detect re-entry BEFORE firing any edge: an outbound edge on the taken
    // port whose target is already executed.
    for (i, e) in spec.edges.iter().enumerate() {
        if &e.from != n {
            continue;
        }
        let fires = e.kind == EdgeKind::Order || &e.port == port;
        if !fires || !st.executed.contains(&e.to) {
            continue;
        }
        let budget = spec.node(&e.to).and_then(|t| t.loop_budget);
        if budget.is_none() {
            // Re-entry into a non-budgeted node is only an error when it is
            // NOT part of a cycle (V3 makes that unreachable via lint, so
            // this is a defence). A non-budgeted loop member (e.g. `retry`)
            // is allowed to re-run; reset it so it can, without touching any
            // budget counter.
            if !crate::sched::loop_helpers::is_in_cycle(spec, &e.to) {
                return Err(format!("re-entry into node '{}' without loop_budget", e.to));
            }
            crate::sched::loop_helpers::reset_loop(spec, st, &e.to);
            continue;
        }
        // Re-entry into a budgeted node: reset it so it can re-run. The
        // budget itself is enforced at execution time (see below), so a
        // non-budgeted loop member resetting the head downstream cannot
        // bypass it.
        crate::sched::loop_helpers::reset_loop(spec, st, &e.to);
        let _ = i;
    }

    // Spec 306: a call head counts its completed entries here, so a replay on resume rebuilds the count.
    if spec
        .node(n)
        .is_some_and(|t| t.cfg.get("max_entries").is_some())
    {
        *st.entries.entry(n.clone()).or_insert(0) += 1;
    }
    // Count every execution of a budgeted node; the (budget+1)th is a
    // terminal failure (F4.5: budget=3 → exactly 3 runs). This is checked on
    // execution, not re-entry, so a non-budgeted loop member resetting the
    // head downstream cannot bypass the budget.
    if let Some(b) = spec.node(n).and_then(|t| t.loop_budget) {
        let it = st.iters.get(n).copied().unwrap_or(0) + 1;
        if it > b {
            return Err(format!("loop_budget exceeded for node '{n}'"));
        }
        st.iters.insert(n.clone(), it);
        // The budget-th execution completed: the run is now terminal. The
        // (budget+1)th execution is what `it > b` catches above, but nodes
        // downstream of this one (e.g. the loop body's gate/retry) must not
        // run again either — they only exist to drive the budgeted node back
        // around the loop, and it cannot run again. Terminate by failing this
        // node now (the caller records the failure and the run ends), so the
        // tick loop never dispatches another batch after the budget is spent.
        if it == b {
            return Err(format!("loop_budget exceeded for node '{n}'"));
        }
    }
    for (i, e) in spec.edges.iter().enumerate() {
        if &e.from != n {
            continue;
        }
        let new = if e.kind == EdgeKind::Order || &e.port == port {
            EdgeState::Fired {
                value: value.clone(),
            }
        } else {
            EdgeState::Dead
        };
        if let Some(slot) = st.edges.get_mut(i) {
            *slot = new;
        }
    }
    Ok(())
}

/// Applies transitions when a node failed or was cancelled: all outbound Data
/// edges -> Dead, all outbound Order edges -> Fired { Null }.
pub fn apply_failed_or_cancelled(spec: &GraphSpec, st: &mut RunState, n: &NodeId) {
    for (i, e) in spec.edges.iter().enumerate() {
        if &e.from != n {
            continue;
        }
        let new = if e.kind == EdgeKind::Order {
            EdgeState::Fired {
                value: serde_json::Value::Null,
            }
        } else {
            EdgeState::Dead
        };
        if let Some(slot) = st.edges.get_mut(i) {
            *slot = new;
        }
    }
}

/// Propagates dead-ness to a fixpoint (spec F4.7). Returns the newly-dead ids.
pub fn propagate_dead(spec: &GraphSpec, st: &mut RunState) -> BTreeSet<NodeId> {
    let mut newly = BTreeSet::new();
    loop {
        let mut changed = false;
        for node in &spec.nodes {
            if st.executed.contains(&node.id) || st.dead.contains(&node.id) {
                continue;
            }
            let in_data = spec.inbound(&node.id, EdgeKind::Data);
            if in_data.is_empty() {
                continue; // entry never becomes dead by propagation
            }
            let all_dead = in_data
                .iter()
                .all(|&i| st.edges.get(i).is_some_and(|e| *e == EdgeState::Dead));
            if all_dead {
                st.dead.insert(node.id.clone());
                newly.insert(node.id.clone());
                apply_failed_or_cancelled(spec, st, &node.id);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    newly
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use pii_abi::{NodeId, PortName};

    use crate::spec::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec};

    use super::{
        Readiness, RunState, apply_completion, apply_failed_or_cancelled, propagate_dead, readiness,
    };

    fn node(id: &str, join: JoinPolicy) -> NodeSpec {
        NodeSpec {
            id: NodeId::from(id),
            kind: "transform".into(),
            join,
            loop_budget: None,
            retry: crate::spec::RetryPolicy::default(),
            caps: 0,
            cfg: serde_json::json!({}),
        }
    }

    fn data(from: &str, to: &str) -> Edge {
        Edge {
            from: NodeId::from(from),
            port: PortName::from("ok"),
            to: NodeId::from(to),
            kind: EdgeKind::Data,
        }
    }

    fn spec(nodes: Vec<NodeSpec>, edges: Vec<Edge>, entry: &str) -> GraphSpec {
        GraphSpec {
            name: "t".into(),
            caps: 0,
            nodes,
            edges,
            entry: NodeId::from(entry),
            limits: GraphLimits::default(),
        }
    }

    /// entry -> a (All join) -> b
    /// Spec EC requires the `readiness_` name so the readiness family is
    /// greppable under one prefix (part 2 exit-criterion 6).
    #[test]
    fn readiness_entry_node_with_no_inbound_is_ready() {
        let g = spec(
            vec![node("entry", JoinPolicy::All), node("a", JoinPolicy::All)],
            vec![data("entry", "a")],
            "entry",
        );
        let st = RunState::from_spec(&g);
        assert_eq!(readiness(&g, &st, &NodeId::from("entry")), Readiness::Ready);
        assert_eq!(readiness(&g, &st, &NodeId::from("a")), Readiness::NotReady);
    }

    #[test]
    fn all_join_waits_for_all_fired() {
        let g = spec(
            vec![
                node("entry", JoinPolicy::All),
                node("a", JoinPolicy::All),
                node("b", JoinPolicy::All),
                node("join", JoinPolicy::All),
            ],
            vec![
                data("entry", "a"),
                data("entry", "b"),
                data("a", "join"),
                data("b", "join"),
            ],
            "entry",
        );
        let mut st = RunState::from_spec(&g);
        apply_completion(
            &g,
            &mut st,
            &NodeId::from("entry"),
            &PortName::from("ok"),
            &serde_json::json!(1),
        )
        .unwrap();
        // only a fired
        apply_completion(
            &g,
            &mut st,
            &NodeId::from("a"),
            &PortName::from("ok"),
            &serde_json::json!("a"),
        )
        .unwrap();
        assert_eq!(
            readiness(&g, &st, &NodeId::from("join")),
            Readiness::NotReady
        );
        apply_completion(
            &g,
            &mut st,
            &NodeId::from("b"),
            &PortName::from("ok"),
            &serde_json::json!("b"),
        )
        .unwrap();
        assert_eq!(readiness(&g, &st, &NodeId::from("join")), Readiness::Ready);
    }

    #[test]
    fn any_join_ready_on_first_fired() {
        let g = spec(
            vec![
                node("entry", JoinPolicy::All),
                node("slow", JoinPolicy::All),
                node("fast", JoinPolicy::All),
                node(
                    "use",
                    JoinPolicy::Any {
                        cancel_losers: false,
                    },
                ),
            ],
            vec![
                data("entry", "slow"),
                data("entry", "fast"),
                data("slow", "use"),
                data("fast", "use"),
            ],
            "entry",
        );
        let mut st = RunState::from_spec(&g);
        apply_completion(
            &g,
            &mut st,
            &NodeId::from("entry"),
            &PortName::from("ok"),
            &serde_json::json!(1),
        )
        .unwrap();
        apply_completion(
            &g,
            &mut st,
            &NodeId::from("fast"),
            &PortName::from("ok"),
            &serde_json::json!("fast"),
        )
        .unwrap();
        assert_eq!(readiness(&g, &st, &NodeId::from("use")), Readiness::Ready);
    }

    #[test]
    fn dead_propagation_marks_fanout_loser_dead() {
        // entry -> {a,b}; a Dead -> b still fires -> no dead propagation to use
        // but a dead with an AllSettled join downstream is fine; here test that
        // a node fed only by a dead branch is dead.
        let g = spec(
            vec![
                node("entry", JoinPolicy::All),
                node("a", JoinPolicy::All),
                node("use", JoinPolicy::All),
            ],
            vec![data("entry", "a"), data("a", "use")],
            "entry",
        );
        let mut st = RunState::from_spec(&g);
        // entry fires on "ok", but 'use' edges from 'a' are data; simulate
        // entry took a different port so a->use stays pending/dead.
        apply_completion(
            &g,
            &mut st,
            &NodeId::from("entry"),
            &PortName::from("ok"),
            &serde_json::json!(1),
        )
        .unwrap();
        apply_failed_or_cancelled(&g, &mut st, &NodeId::from("a"));
        let _newly = propagate_dead(&g, &mut st);
        assert!(st.dead.contains(&NodeId::from("use")));
    }

    #[test]
    fn dead_fanout_marks_join_unsatisfiable_all() {
        let g = spec(
            vec![
                node("entry", JoinPolicy::All),
                node("a", JoinPolicy::All),
                node("b", JoinPolicy::All),
                node("join", JoinPolicy::All),
            ],
            vec![
                data("entry", "a"),
                data("entry", "b"),
                data("a", "join"),
                data("b", "join"),
            ],
            "entry",
        );
        let mut st = RunState::from_spec(&g);
        apply_completion(
            &g,
            &mut st,
            &NodeId::from("entry"),
            &PortName::from("ok"),
            &serde_json::json!(1),
        )
        .unwrap();
        apply_completion(
            &g,
            &mut st,
            &NodeId::from("a"),
            &PortName::from("ok"),
            &serde_json::json!("a"),
        )
        .unwrap();
        apply_failed_or_cancelled(&g, &mut st, &NodeId::from("b"));
        assert_eq!(
            readiness(&g, &st, &NodeId::from("join")),
            Readiness::UnsatisfiableJoin
        );
    }
}

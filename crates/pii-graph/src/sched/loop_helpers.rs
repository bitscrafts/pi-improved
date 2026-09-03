//! Loop re-entry helpers (F4.5): cycle membership and reachable-set reset.

use std::collections::BTreeMap;

use pii_abi::NodeId;

use crate::sched::state::{EdgeState, RunState};
use crate::spec::GraphSpec;

/// True when `id` is a member of a cycle (SCC with >1 node, or a self-loop).
#[must_use]
pub fn is_in_cycle(spec: &GraphSpec, id: &NodeId) -> bool {
    let mut g = petgraph::graph::DiGraph::<(), ()>::new();
    let mut idx: BTreeMap<&str, _> = BTreeMap::new();
    for n in &spec.nodes {
        idx.insert(n.id.as_str(), g.add_node(()));
    }
    for e in &spec.edges {
        if let (Some(&a), Some(&b)) = (idx.get(e.from.as_str()), idx.get(e.to.as_str())) {
            g.add_edge(a, b, ());
        }
    }
    for scc in petgraph::algo::tarjan_scc(&g) {
        let is_cycle = scc.len() > 1
            || (scc.len() == 1 && scc.first().is_some_and(|&n| g.find_edge(n, n).is_some()));
        if is_cycle
            && scc.iter().any(|&i| {
                spec.nodes
                    .iter()
                    .any(|n| n.id == *id && idx.get(n.id.as_str()) == Some(&i))
            })
        {
            return true;
        }
    }
    false
}

/// BFS from `start` over all outbound edges (Data and Order): removes every
/// reachable node from `executed`/`dead` and resets every edge whose `from`
/// is in the set to `Pending` (F4.5).
pub fn reset_loop(spec: &GraphSpec, st: &mut RunState, start: &NodeId) {
    let mut queue = vec![start.clone()];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(id) = queue.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        st.executed.remove(&id);
        st.dead.remove(&id);
        for (i, e) in spec.edges.iter().enumerate() {
            if e.from == id {
                if let Some(slot) = st.edges.get_mut(i) {
                    *slot = EdgeState::Pending;
                }
                queue.push(e.to.clone());
            }
        }
    }
}

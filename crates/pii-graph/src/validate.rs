//! Static graph validation (spec F4.6).
//!
//! `Ok(warnings)` / `Err(errors)`. Every message starts with its rule code so
//! it is greppable. V4 (schema compatibility) is deferred — do not implement.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use crate::node::NodeRegistry;
use crate::spec::{EdgeKind, GraphSpec, JoinPolicy, NodeSpec};
use pii_abi::NodeId;
use pii_core::CapKind;

/// Validates a graph against the registry.
///
/// Returns `Ok(warnings)` when the graph is valid (possibly with non-fatal
/// warnings), or `Err(errors)` when it is invalid.
///
/// # Errors
/// Returns `Err(errors)` whenever any V-code rule fails. Each error string
/// starts with its rule code (e.g. `V0`, `V1`) for greppability.
#[allow(clippy::too_many_lines)]
pub fn validate(spec: &GraphSpec, reg: &NodeRegistry) -> Result<Vec<String>, Vec<String>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // V0 structural
    let mut seen = HashSet::new();
    for n in &spec.nodes {
        if !seen.insert(n.id.as_str()) {
            errors.push(format!("V0: duplicate node id '{}'", n.id));
        }
        if reg.factory(&n.kind).is_err() {
            errors.push(format!("V0: node '{}' has unknown kind '{}'", n.id, n.kind));
        }
    }
    let known: BTreeSet<&str> = spec.nodes.iter().map(|n| n.id.as_str()).collect();
    for e in &spec.edges {
        if !known.contains(e.from.as_str()) {
            errors.push(format!("V0: edge references unknown node '{}'", e.from));
        }
        if !known.contains(e.to.as_str()) {
            errors.push(format!("V0: edge references unknown node '{}'", e.to));
        }
    }
    if !known.contains(spec.entry.as_str()) {
        errors.push(format!("V0: entry '{}' is not a declared node", spec.entry));
    }

    // V1 port existence, V9 multi-arm join arity — need factory ports per node.
    let mut node_ports: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for n in &spec.nodes {
        if let Ok(f) = reg.factory(&n.kind) {
            match f.ports(&n.cfg) {
                Ok(ps) => {
                    node_ports
                        .entry(n.id.as_str())
                        .or_default()
                        .extend(ps.iter().map(|p| p.as_str().to_string()));
                }
                Err(msg) => errors.push(format!("V0: node '{}': {msg}", n.id)),
            }
        }
    }
    for e in &spec.edges {
        if e.kind == EdgeKind::Data || e.kind == EdgeKind::Order {
            let ports = node_ports.get(e.from.as_str());
            let allowed = |p: &str| ports.is_some_and(|s| s.contains(p));
            if !allowed(e.port.as_str()) {
                errors.push(format!("V1: node '{}' has no port '{}'", e.from, e.port));
            }
        }
    }

    // V2 cap subset
    let spec_kinds = CapKind::from_bits_truncate(spec.caps);
    for n in &spec.nodes {
        let required = match reg.factory(&n.kind) {
            Ok(f) => f.required_caps(),
            Err(_) => continue,
        };
        let node_caps = CapKind::from_bits_truncate(n.caps);
        let needed = required | node_caps;
        if needed & !spec_kinds != CapKind::empty() {
            errors.push(format!(
                "V2: node '{}' requires capability '{}' not granted by the graph",
                n.id,
                missing(needed & !spec_kinds)
            ));
        }
    }

    // V3 bounded cycles
    let cycle = find_cycle_error(spec);
    if let Some(msg) = cycle {
        errors.push(msg);
    }

    // V9 multi-arm join arity (skip if V3 found a cycle is fine to still run)
    for n in &spec.nodes {
        let in_data = spec.inbound(&n.id, EdgeKind::Data).len();
        match &n.join {
            JoinPolicy::Any { .. } | JoinPolicy::AllSettled if in_data < 2 => {
                errors.push(format!(
                    "V9: node '{}' uses a multi-arm join with {in_data} inbound data edges",
                    n.id
                ));
            }
            JoinPolicy::Quorum { k, .. } => {
                if *k < 1 {
                    errors.push(format!("V9: node '{}' quorum k must be >= 1", n.id));
                } else if in_data < *k as usize {
                    errors.push(format!(
                        "V9: node '{}' quorum k={} exceeds its {} inbound data edges",
                        n.id, k, in_data
                    ));
                }
            }
            _ => {}
        }
    }

    // V5 unreachable from entry (warning) — simple BFS over outbound edges.
    let idx: BTreeMap<&str, &NodeSpec> = spec.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut reachable = BTreeSet::new();
    let mut stack = vec![spec.entry.as_str()];
    while let Some(id) = stack.pop() {
        if !reachable.insert(id) {
            continue;
        }
        let from = NodeId::from(id);
        for e in spec.outbound(&from) {
            let e = spec.edges.get(e);
            if let Some(e) = e {
                let tid = idx.get(e.to.as_str()).map_or("", |n| n.id.as_str());
                if !reachable.contains(tid) {
                    stack.push(e.to.as_str());
                }
            }
        }
    }
    for n in &spec.nodes {
        if !reachable.contains(n.id.as_str()) {
            warnings.push(format!("V5: node '{}' is unreachable from entry", n.id));
        }
    }

    if errors.is_empty() {
        Ok(warnings)
    } else {
        Err(errors)
    }
}

fn find_cycle_error(spec: &GraphSpec) -> Option<String> {
    let mut g: DiGraph<(), ()> = DiGraph::new();
    let mut indices: BTreeMap<&str, _> = BTreeMap::new();
    for n in &spec.nodes {
        let i = g.add_node(());
        indices.insert(n.id.as_str(), i);
    }
    for e in &spec.edges {
        if let (Some(&a), Some(&b)) = (indices.get(e.from.as_str()), indices.get(e.to.as_str())) {
            g.add_edge(a, b, ());
        }
    }
    for scc in tarjan_scc(&g) {
        let nodes: Vec<&str> = scc
            .iter()
            .map(|&i| {
                spec.nodes
                    .iter()
                    .find(|n| indices.get(n.id.as_str()) == Some(&i))
                    .map_or("?", |n| n.id.as_str())
            })
            .collect();
        let is_cycle = scc.len() > 1
            || (scc.len() == 1
                && scc.first().is_some_and(|&n| {
                    g.edges_directed(n, petgraph::Direction::Outgoing).count() > 0
                }));
        if !is_cycle {
            continue;
        }
        // A cycle is allowed only if at least one member has a loop_budget.
        let has_budget = nodes.iter().any(|id| {
            spec.nodes
                .iter()
                .find(|n| n.id.as_str() == *id)
                .and_then(|n| n.loop_budget)
                .is_some()
        });
        if !has_budget {
            return Some(format!(
                "V3: cycle [{}] has no loop_budget",
                nodes.join(", ")
            ));
        }
    }
    None
}

fn missing(kinds: CapKind) -> String {
    let mut names = Vec::new();
    for (bit, name) in [
        (CapKind::FS_READ, "fs_read"),
        (CapKind::FS_WRITE, "fs_write"),
        (CapKind::NET, "net"),
        (CapKind::PROC, "proc"),
        (CapKind::HW, "hw"),
        (CapKind::SECRETS, "secrets"),
        (CapKind::MEMORY_WRITE, "memory_write"),
        (CapKind::SPAWN, "spawn"),
    ] {
        if kinds.contains(bit) {
            names.push(name);
        }
    }
    if names.is_empty() {
        "?".to_string()
    } else {
        names.join(",")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use crate::node::NodeRegistry;
    use crate::spec::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy};
    use pii_abi::NodeId;
    use pii_abi::PortName;

    use super::validate;

    fn spec(nodes: Vec<(&str, &str, JoinPolicy, u32)>, entry: &str) -> GraphSpec {
        // (id, kind, join, caps_bits)
        let nodes = nodes
            .into_iter()
            .map(|(id, kind, join, caps)| NodeSpec {
                id: NodeId::from(id),
                kind: kind.to_string(),
                join,
                loop_budget: None,
                retry: RetryPolicy::default(),
                caps,
                cfg: serde_json::json!({}),
            })
            .collect();
        GraphSpec {
            name: "t".into(),
            caps: 0,
            nodes,
            edges: vec![],
            entry: NodeId::from(entry),
            limits: GraphLimits::default(),
        }
    }

    #[test]
    fn lint_bad_port_detects_v1() {
        let mut g = spec(vec![("a", "wait", JoinPolicy::All, 0)], "a");
        g.edges.push(Edge {
            from: NodeId::from("a"),
            port: PortName::from("nope"),
            to: NodeId::from("a"),
            kind: EdgeKind::Data,
        });
        let reg = NodeRegistry::builtin();
        assert!(validate(&g, &reg).is_err());
    }

    #[test]
    fn lint_cap_escalation_detects_v2() {
        // emit requires FS_WRITE; graph has none.
        let g = spec(vec![("w", "emit", JoinPolicy::All, 0)], "w");
        let reg = NodeRegistry::builtin();
        let r = validate(&g, &reg);
        assert!(r.is_err());
    }

    #[test]
    fn lint_one_armed_race_detects_v9() {
        // single inbound data edge + any join
        let mut g = spec(
            vec![
                ("x", "transform", JoinPolicy::All, 0),
                (
                    "y",
                    "transform",
                    JoinPolicy::Any {
                        cancel_losers: false,
                    },
                    0,
                ),
            ],
            "x",
        );
        g.edges.push(Edge {
            from: NodeId::from("x"),
            port: PortName::from("ok"),
            to: NodeId::from("y"),
            kind: EdgeKind::Data,
        });
        let reg = NodeRegistry::builtin();
        assert!(validate(&g, &reg).is_err());
    }
}

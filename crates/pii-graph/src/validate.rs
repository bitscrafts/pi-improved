//! Static graph validation (spec F4.6).
//!
//! `Ok(warnings)` / `Err(errors)`. Every message starts with its rule code so
//! it is greppable. V4 (schema compatibility) is deferred — do not implement.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use crate::expr::{PROMPT_CAP_BYTES, placeholders};
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

    // V10 a node that can block on a process must bound itself (spec 162). Spec 247: the `coder`
    // kind is judged by its KIND, not its effects — it spawns an agent for as long as it likes while
    // its factory declares only `MUTATES_EXTERNAL`, and declaring `SPAWNS_PROC` would make every
    // coder node not parallel-safe.
    for n in &spec.nodes {
        let spawns = n.kind == "coder"
            || reg
                .factory(&n.kind)
                .is_ok_and(|f| f.effects().contains(pii_abi::Effects::SPAWNS_PROC));
        if spawns
            && n.cfg
                .get("timeout_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                == 0
        {
            errors.push(format!(
                "V10: node '{}' spawns a process and requires a positive 'timeout_ms'",
                n.id
            ));
        }
    }

    // V12 (spec 386): a node's `prompt` is checked at load. Only an agent node carries one; it does not share a node with
    // `brief`; it is bounded; and each placeholder names a path the graph can supply.
    for n in &spec.nodes {
        let Some(prompt) = n.cfg.get("prompt") else {
            continue;
        };
        // A prompt needs a CONSUMER. A model node turns it into instructions (`coder`, `agent`);
        // a `gate` shows it to a human as the question being asked (starter/boss-watch's `escalate`).
        // Refusing those three refused the repository's own graphs: measured 2026-11-05, V12 as first
        // written allowed only `coder` and broke all six boss-watch tests.
        // A prompt needs a consumer, but WHICH kinds have one is not the validator's to decide: the SOP
        // compiler attaches a prompt to every step it generates, and `gate` shows one to a human. This
        // was an error for one night and it refused both (measured 2026-11-05). It is a warning now: the
        // refusals that matter are below (prompt XOR brief, placeholders checked at load, the size cap).
        // A prompt needs a CONSUMER: a model node turns it into instructions (`coder`, `agent`), and a
        // `gate` shows it to a human as the question being asked. Any other node carrying one is a field
        // nothing reads — which is how a graph ends up looking documented while the instruction goes
        // nowhere. The SOP compiler emitted exactly that and was fixed instead of the rule being widened.
        if !matches!(n.kind.as_str(), "coder" | "agent" | "gate") {
            errors.push(format!(
                "V12: node '{}' carries a prompt; prompts belong to agent nodes",
                n.id
            ));
            continue;
        }
        if n.cfg.get("brief").is_some() {
            errors.push(format!(
                "V12: node '{}' sets both 'prompt' and 'brief'; a node says where its instruction comes from once",
                n.id
            ));
        }
        let Some(text) = prompt.as_str() else {
            errors.push(format!("V12: node '{}' prompt must be a string", n.id));
            continue;
        };
        if text.len() > PROMPT_CAP_BYTES {
            errors.push(format!(
                "V12: node '{}' prompt is {} bytes; the cap is {PROMPT_CAP_BYTES}",
                n.id,
                text.len()
            ));
        }
        match placeholders(text) {
            Err(e) => errors.push(format!("V12: node '{}': {e}", n.id)),
            Ok(paths) => {
                for path in paths {
                    let mut parts = path.split('.');
                    let supplied = match parts.next() {
                        Some("input" | "payload") => true,
                        Some("nodes") => parts.next().is_some_and(|id| known.contains(id)),
                        _ => false,
                    };
                    if !supplied {
                        errors.push(format!(
                            "V12: node '{}' placeholder '{{{{{path}}}}}' names a path the graph cannot supply",
                            n.id
                        ));
                    }
                }
            }
        }
    }

    // V3 bounded cycles
    let cycle = find_cycle_error(spec);
    if let Some(msg) = &cycle {
        errors.push(msg.clone());
    }

    // V6 statically unsatisfiable `All` join — only meaningful when the graph
    // is acyclic (spec part3 §4.6). Skip when V3 found a cycle.
    if !spec.edges.is_empty()
        && cycle.is_none()
        && let Some(vec) = v6_unsatisfiable_join(spec)
    {
        errors.extend(vec);
    }

    // V9 multi-arm join arity (skip if V3 found a cycle is fine to still run)
    for n in &spec.nodes {
        let in_data = spec.inbound(&n.id, EdgeKind::Data).len();
        match &n.join {
            // `Quorum` requires >= 2 inbound Data edges (pass-3 §4.6 V9: a one-armed
            // quorum is a typo). A one-armed `Any` is legal at runtime: it readies
            // exactly as an `all` join with that arm (fired → ready, dead → dead), and
            // exclusive-port refusals use it. So it is a warning, not an error.
            // `AllSettled` is NOT over-enforced: a single-arm AllSettled is a
            // legitimate "proceed whether or not this branch died" idiom.
            JoinPolicy::Any { .. } if in_data < 2 => {
                warnings.push(format!(
                    "V9: node '{}' uses a one-armed 'any' join with {in_data} inbound data edges; it runs as 'all'",
                    n.id
                ));
            }
            JoinPolicy::Quorum { k, .. } => {
                if *k < 1 {
                    errors.push(format!("V9: node '{}' quorum k must be >= 1", n.id));
                } else if in_data < 2 {
                    errors.push(format!(
                        "V9: node '{}' uses a multi-arm join with {in_data} inbound data edges",
                        n.id
                    ));
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

    // V5 second clause (pass-1 §3.2 rule 5): a declared port with no outgoing
    // edge is a warning — it points at a missing edge before the run reaches
    // "no progress: node ... never became ready".
    for n in &spec.nodes {
        let Ok(factory) = reg.factory(&n.kind) else {
            continue; // unknown kind is already a V0 error
        };
        let Ok(ports) = factory.ports(&n.cfg) else {
            continue; // malformed cfg is surfaced elsewhere
        };
        let used: BTreeSet<&str> = spec
            .outbound(&n.id)
            .iter()
            .filter_map(|&ei| spec.edges.get(ei))
            .map(|e| e.port.as_str())
            .collect();
        for p in &ports {
            if !used.contains(p.as_str()) {
                warnings.push(format!(
                    "V5: node '{}' declares port '{}' with no outgoing edge",
                    n.id, p
                ));
            }
        }
    }

    // V11 a coder's `err` edge that ends in a terminal `transform` finishes the
    // run as succeeded although the agent failed (spec 238). Warning only.
    for n in spec.nodes.iter().filter(|n| n.kind == "coder") {
        for &ei in &spec.outbound(&n.id) {
            let Some(e) = spec.edges.get(ei).filter(|e| e.port.as_str() == "err") else {
                continue;
            };
            let terminal_transform = idx
                .get(e.to.as_str())
                .is_some_and(|t| t.kind == "transform" && spec.outbound(&t.id).is_empty());
            if terminal_transform {
                warnings.push(format!(
                    "V11: coder node '{}' routes its err port to transform node '{}', which has no outgoing edge, so a failed agent finishes the run as succeeded",
                    n.id, e.to
                ));
            }
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
                    // Single-node SCC is a cycle only if it has a self-edge.
                    g.find_edge(n, n).is_some()
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

/// V6: statically unsatisfiable `All` join — the routed-diamond deadlock caught
/// before it runs. Requires the **transitive** formulation (spec part3 §4.6):
/// the join's edges come from `a`/`b`, not the router, so we propagate per-edge
/// router/port constraints forward and detect mutually-exclusive ports feeding
/// one `All` join. Only valid on an acyclic graph (caller skips if V3 found a
/// cycle).
#[allow(clippy::too_many_lines)]
fn v6_unsatisfiable_join(spec: &GraphSpec) -> Option<Vec<String>> {
    use pii_abi::PortName;

    // constraints[edge_index] : BTreeMap<NodeId /*router*/, BTreeSet<PortName>>
    let mut constraints: Vec<BTreeMap<NodeId, BTreeSet<PortName>>> =
        vec![BTreeMap::new(); spec.edges.len()];

    // Topological order over data edges (Kahn). Acyclic guaranteed by caller.
    let mut indegree: BTreeMap<&str, usize> =
        spec.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    for e in &spec.edges {
        if e.kind == EdgeKind::Data
            && let Some(&deg) = indegree.get(e.to.as_str())
        {
            indegree.insert(e.to.as_str(), deg + 1);
        }
    }
    let mut queue: Vec<&str> = spec
        .nodes
        .iter()
        .filter(|n| indegree.get(n.id.as_str()).copied() == Some(0))
        .map(|n| n.id.as_str())
        .collect();
    let mut ready = 0usize;
    while let Some(u) = queue.pop() {
        ready += 1;
        let uid = NodeId::from(u);
        // c_u = union_per_router(inbound data constraints), empty for entry.
        let mut c_u: BTreeMap<NodeId, BTreeSet<PortName>> = BTreeMap::new();
        for ei in spec.inbound(&uid, EdgeKind::Data) {
            if let Some(m) = constraints.get(ei) {
                for (r, ps) in m {
                    c_u.entry(r.clone()).or_default().extend(ps.iter().cloned());
                }
            }
        }
        // Branching iff outbound data edges use >1 distinct port.
        let out_data: Vec<usize> = spec
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.from == uid && e.kind == EdgeKind::Data)
            .map(|(i, _)| i)
            .collect();
        let mut distinct_ports = HashSet::new();
        for &eo in &out_data {
            if let Some(port) = spec.edges.get(eo).map(|e| e.port.as_str()) {
                distinct_ports.insert(port);
            }
        }
        let branching = distinct_ports.len() > 1;
        for &eo in &out_data {
            if let Some(slot) = constraints.get_mut(eo) {
                *slot = c_u.clone();
                if branching && let Some(port) = spec.edges.get(eo).map(|e| e.port.clone()) {
                    let mut set = BTreeSet::new();
                    set.insert(port);
                    slot.insert(uid.clone(), set); // overwrite, not union
                }
            }
        }
        // Decrement successors' indegree over data edges.
        for eo in spec.outbound(&uid) {
            if let Some(e) = spec.edges.get(eo)
                && e.kind == EdgeKind::Data
                && let Some(&deg) = indegree.get(e.to.as_str())
            {
                indegree.insert(e.to.as_str(), deg - 1);
                if deg == 1 {
                    queue.push(e.to.as_str());
                }
            }
        }
    }
    if ready != spec.nodes.len() {
        return None;
    }

    let mut errors = Vec::new();
    for j in spec.nodes.iter().filter(|n| n.join == JoinPolicy::All) {
        let in_data = spec.inbound(&j.id, EdgeKind::Data);
        for (i1, i2) in pairs(in_data.len()) {
            let (Some(ei1), Some(ei2)) = (in_data.get(i1), in_data.get(i2)) else {
                continue;
            };
            let (Some(edge1), Some(edge2)) = (spec.edges.get(*ei1), spec.edges.get(*ei2)) else {
                continue;
            };
            let (Some(c1), Some(c2)) = (constraints.get(*ei1), constraints.get(*ei2)) else {
                continue;
            };
            for r in c1.keys().filter(|r2| c2.contains_key(*r2)) {
                let s1 = c1.get(r);
                let s2 = c2.get(r);
                if let (Some(s1), Some(s2)) = (s1, s2)
                    && s1.is_disjoint(s2)
                {
                    errors.push(format!(
                        "V6: join '{}' can never be satisfied: '{}' and '{}' are on mutually exclusive ports of '{}'",
                        j.id, edge1.from, edge2.from, r
                    ));
                }
            }
        }
    }
    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Yields the unique index pairs `(0,1),(0,2),...` for `n` elements (i1 < i2).
fn pairs(n: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..n).flat_map(move |i| (i + 1..n).map(move |j| (i, j)))
}

fn missing(kinds: CapKind) -> String {
    let names = kinds.names();
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
    fn lint_one_armed_race_warns_v9() {
        // single inbound data edge + any join: legal at runtime, so a warning and not an error
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
        let warnings = validate(&g, &reg).expect("a one-armed any join is not an error");
        assert!(warnings.iter().any(|w| w.starts_with("V9: node 'y'")));
    }
}

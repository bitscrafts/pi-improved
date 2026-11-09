//! Static, deterministic ASCII graph rendering (`pii graph show`, spec 10).
//!
//! This module turns a validated [`GraphSpec`] into a human-readable plain
//! ASCII picture of its topology (nodes, branches, fan-in/fan-out joins, and
//! `loop_budget` cycles). It is deliberately a *pure* layer: it never loads,
//! never validates, and never touches a daemon or a socket — callers run
//! [`crate::validate::validate`] first (as the CLI does) and only hand already
//! valid specs here. Rendering is byte-for-byte deterministic for a given
//! spec, which is what makes future golden-file tests possible.
//!
//! ## Layout rule (loops are not lanes)
//!
//! Cycles cannot go through a git-style linear lane (that would produce the
//! diagonal-crossing mess git itself is criticised for on merge-heavy
//! histories). Instead, nodes are laid out in topological order with cycle
//! closure edges set aside, using [`topo_order`]. An edge that points
//! *backward* relative to that topological order (i.e. its target was already
//! emitted on an earlier row) is rendered as a `^-- loops back to ...`
//! annotation under its source node — not as a lane glyph.

use std::collections::HashSet;

use pii_abi::NodeId;

use crate::spec::{GraphSpec, JoinPolicy};

/// A topological order over `spec.nodes`, computed so that rows read roughly
/// down-and-to-the-right regardless of any `loop_budget` cycles.
///
/// Cycles cannot be topologically ordered as-is, so the order is derived from
/// a depth-first forest starting at [`GraphSpec::entry`] (then any node not
/// reachable from the entry, in declaration order, so every node appears
/// exactly once). Because the DFS refuses to re-enter a node it has already
/// visited, a cycle's closure edge ends up pointing to an *earlier* row —
/// which is exactly what the renderer then demotes to a loop-back annotation.
///
/// The order is fully deterministic for a given spec: it depends only on the
/// spec's declaration order, the entry id, and edge order.
#[must_use]
pub fn topo_order(spec: &GraphSpec) -> Vec<NodeId> {
    // Forest roots: the entry first, then every node not reachable from it, in
    // declaration order (so isolated nodes are not dropped when the graph is
    // not fully connected from `entry`).
    let mut seeds: Vec<NodeId> = Vec::with_capacity(spec.nodes.len());
    {
        let mut rooted = HashSet::new();
        seeds.push(spec.entry.clone());
        rooted.insert(spec.entry.as_str());
        for n in &spec.nodes {
            if rooted.insert(n.id.as_str()) {
                seeds.push(n.id.clone());
            }
        }
    }

    // `state` doubles as the "already placed" set: a seed whose component was
    // already covered by an earlier root is simply skipped.
    let mut state = HashSet::new();
    // `post` holds the DFS post-order (children first); reversing it yields
    // the topological order for the traversal tree, so forward edges point
    // strictly down the rendered page and any closure edge points back up.
    let mut post: Vec<NodeId> = Vec::with_capacity(spec.nodes.len());
    for root in seeds {
        dfs_push_post(spec, &root, &mut state, &mut post);
    }

    post.reverse();
    post
}

/// Recursively appends `node` and its (post-order) descendants to `out`.
///
/// Descendants are visited depth-first in edge declaration order. Once a node
/// has been inserted into `seen`, it is never entered again, which both
/// guarantees termination on cyclic input and naturally breaks each cycle at
/// exactly one edge (the edge that would re-enter an already-seen node). When
/// the recursion unwinds, `node` is appended after all of its descendants, so
/// `out` is a classic DFS post-order.
fn dfs_push_post(
    spec: &GraphSpec,
    node: &NodeId,
    seen: &mut HashSet<String>,
    out: &mut Vec<NodeId>,
) {
    if !seen.insert(node.as_str().to_string()) {
        // Already visited (current DFS ancestor or a finished subtree): this
        // is the cycle-closure edge — do not recurse into it again.
        return;
    }
    for e in &spec.edges {
        if e.from == *node {
            dfs_push_post(spec, &e.to, seen, out);
        }
    }
    out.push(node.clone());
}

/// Renders `spec` as a plain-ASCII multi-line picture.
///
/// The output is one contiguous string ending in a single trailing newline.
/// Each node produces one *block*: a header line, a `|-- <port> --> <to>` line
/// per forward fan-out edge, and a single `^-- loops back to "<id>" (budget=N)`
/// annotation per backward/closure edge. Blocks are separated by one blank
/// line. The result is byte-identical across repeated calls for the same
/// spec, so it is safe to compare for golden-file testing.
#[must_use]
pub fn render_graph(spec: &GraphSpec) -> String {
    let order = topo_order(spec);
    let rank = rank_of(&order);

    let mut blocks: Vec<String> = Vec::with_capacity(order.len());
    // Resolve each ordered id back to its spec node (always present in a
    // validated spec, so the filter_map never drops a row in practice).
    for id in &order {
        let Some(node) = spec.nodes.iter().find(|n| &n.id == id) else {
            continue;
        };
        blocks.push(render_block(spec, node, &rank));
    }
    let mut body = blocks.join("\n\n");
    body.push('\n');
    body
}

/// Builds a `String -> row-index` lookup so backward edges can be detected.
fn rank_of(order: &[NodeId]) -> std::collections::HashMap<String, usize> {
    let mut rank = std::collections::HashMap::with_capacity(order.len());
    for (i, id) in order.iter().enumerate() {
        rank.insert(id.as_str().to_string(), i);
    }
    rank
}

/// Renders the block (header + fan-out + loop-backs) for `node`.
fn render_block(
    spec: &GraphSpec,
    node: &crate::spec::NodeSpec,
    rank: &std::collections::HashMap<String, usize>,
) -> String {
    let mut lines = vec![render_header(spec, node)];
    let node_rank = rank.get(node.id.as_str()).copied().unwrap_or_default();

    // Fan-out and back edges, both in the graph's edge declaration order so
    // the picture mirrors the TOML the author actually wrote.
    for e in &spec.edges {
        if e.from != node.id {
            continue;
        }
        let to_rank = rank.get(e.to.as_str()).copied().unwrap_or_default();
        if to_rank <= node_rank {
            // Backward (or self) edge: the cycle closure -> annotate, don't lane.
            let budget = spec
                .node(&e.to)
                .and_then(|n| n.loop_budget)
                .map_or_else(String::new, |b| format!(" (budget={b})"));
            lines.push(format!(" ^-- loops back to \"{}\"{budget}", e.to));
        } else {
            lines.push(format!(" |-- {} --> {}", e.port, e.to));
        }
    }
    lines.join("\n")
}

/// Renders `<id> [<kind-detail>]<suffix>` for one node.
///
/// Kind-specific detail is surfaced from the node's `cfg` JSON (the same keys
/// that land there via `toml_load`) so a human can read a branch target or
/// expression source without opening the TOML:
/// - `route` shows its `on` selector
/// - `tool` shows the tool name
/// - `transform` shows its `expr`
/// - `agent` is plain; anything else shows its bare `kind`
///
/// Known suffix tokens, in a fixed order, only when present:
/// `(entry)`, `(loop_budget=N)`, then a non-default `(join: ...)` annotation.
fn render_header(spec: &GraphSpec, node: &crate::spec::NodeSpec) -> String {
    let mut header = format!("{} [{}]", node.id, desc(node));
    if spec.entry == node.id {
        header.push_str(" (entry)");
    }
    if let Some(budget) = node.loop_budget {
        use std::fmt::Write as _;
        // Writing to a `String` is infallible.

        #[allow(clippy::let_underscore_must_use)] // infallible: writing to a String
        let _ = write!(header, " (loop_budget={budget})");
    }
    if let Some(join) = join_suffix(&node.join) {
        header.push(' ');
        header.push_str(&join);
    }
    header
}

/// Builds the text inside `[ ... ]` based on a node's kind and cfg.
fn desc(node: &crate::spec::NodeSpec) -> String {
    let field = |key: &str| {
        node.cfg
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map_or_else(String::new, str::to_string)
    };
    let detail = match node.kind.as_str() {
        "route" => field("on"),
        "tool" => field("tool"),
        "transform" => field("expr"),
        // `agent` (and every unknown kind) has no upstream-keyed detail key —
        // the header stays bare, which also conveniently keeps `gate`,
        // `timer`, `wait`, `memory`, `debug_crash`, ... readable.
        _ => String::new(),
    };
    match node.kind.as_str() {
        "route" if !detail.is_empty() => format!("route: {detail}"),
        "tool" if !detail.is_empty() => format!("tool: {detail}"),
        "transform" if !detail.is_empty() => format!("transform: {detail}"),
        _ => node.kind.clone(),
    }
}

/// Renders a join policy suffix — but only when it differs from the default
/// (`All` with no cancel), so plain nodes stay annotation-free.
fn join_suffix(join: &JoinPolicy) -> Option<String> {
    match join {
        JoinPolicy::All => None,
        JoinPolicy::AllSettled => Some("(join: allsettled)".to_string()),
        JoinPolicy::Any { cancel_losers } => {
            Some(format!("(join: any, cancel_losers={cancel_losers})"))
        }
        JoinPolicy::Quorum { k, cancel_losers } => {
            Some(format!("(join: quorum:{k}, cancel_losers={cancel_losers})"))
        }
    }
}

#[cfg(test)]
mod graph_show {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::render_graph;
    use crate::render::topo_order;
    use crate::spec::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy};
    use pii_abi::NodeId;

    /// Builds a minimal `transform` node.
    fn node(id: &str) -> NodeSpec {
        NodeSpec {
            id: NodeId::from(id),
            kind: "transform".into(),
            join: JoinPolicy::All,
            loop_budget: None,
            retry: RetryPolicy::default(),
            caps: 0,
            cfg: serde_json::json!({"expr": format!("$.v.{id}")}),
        }
    }

    /// Builds a spec with the given nodes and linear edges `from -> to`.
    fn linear(pairs: &[(&str, &str)]) -> GraphSpec {
        let ids: Vec<NodeId> = {
            let mut seen = Vec::new();
            for (a, b) in pairs {
                seen.push(NodeId::from(*a));
                seen.push(NodeId::from(*b));
            }
            seen.sort();
            seen.dedup();
            seen
        };
        let nodes = ids.iter().map(|n| node(n.as_str())).collect::<Vec<_>>();
        let edges = pairs
            .iter()
            .map(|(a, b)| Edge {
                from: NodeId::from(*a),
                port: "ok".into(),
                to: NodeId::from(*b),
                kind: EdgeKind::Data,
            })
            .collect::<Vec<_>>();
        GraphSpec {
            name: "t".into(),
            caps: 0,
            nodes,
            edges,
            entry: NodeId::from(pairs[0].0),
            limits: GraphLimits::default(),
        }
    }

    #[test]
    fn show_renders_linear_graph_with_no_branches() {
        let spec = linear(&[("a", "b")]);
        let out = render_graph(&spec);
        // Two rows, `a` then `b`, each on its own line.
        let line_of =
            |needle: &str| -> Option<usize> { out.lines().position(|l| l.contains(needle)) };
        let a_row = line_of("a [transform: $.v.a]").expect("a row present");
        let b_row = line_of("b [transform: $.v.b]").expect("b row present");
        assert!(a_row < b_row, "rows out of order: {out}");
        // a's block carries exactly one forward arrow to b, and it sits above
        // b's row (b is terminal — nothing on or after its header but newline).
        assert_eq!(out.matches("|-- ok --> b").count(), 1);
        let arrow_at = out.find("|-- ok --> b").expect("the a -> b arrow");
        let b_header_at = out.find("b [transform").expect("b header");
        assert!(
            arrow_at < b_header_at,
            "b unexpectedly has an out-arrow: {out}"
        );
    }

    #[test]
    fn show_renders_route_fanout_as_multiple_arrows() {
        // verify-fix-shaped: a `route` gate whose two rules/ports fan out.
        let mut gate = node("gate");
        gate.kind = "route".into();
        gate.cfg = serde_json::json!({"on": "$.payload.exit_code", "rules": []});
        let mut done = node("done");
        done.cfg = serde_json::json!({"expr": "$.payload"});
        let mut retry = node("retry");
        retry.cfg = serde_json::json!({"expr": "$.payload"});
        let spec = GraphSpec {
            name: "t".into(),
            caps: 0,
            nodes: vec![gate, done, retry],
            edges: vec![
                Edge {
                    from: NodeId::from("gate"),
                    port: "pass".into(),
                    to: NodeId::from("done"),
                    kind: EdgeKind::Data,
                },
                Edge {
                    from: NodeId::from("gate"),
                    port: "fail".into(),
                    to: NodeId::from("retry"),
                    kind: EdgeKind::Data,
                },
            ],
            entry: NodeId::from("gate"),
            limits: GraphLimits::default(),
        };
        let out = render_graph(&spec);
        // Exactly two fan-out arrows on gate's row, one per rule/port.
        assert_eq!(out.lines().filter(|l| l.starts_with(" |-- ")).count(), 2);
        assert_eq!(out.matches("|-- pass --> done").count(), 1);
        assert_eq!(out.matches("|-- fail --> retry").count(), 1);
        assert!(out.contains("[route: $.payload.exit_code]"));
    }

    #[test]
    fn show_annotates_non_default_join_policy() {
        // A node with `join = "any"` surfaces `(join: any, cancel_losers=..)`.
        let mut fanin = node("fanin");
        fanin.join = JoinPolicy::Any {
            cancel_losers: false,
        };
        let spec = GraphSpec {
            name: "t".into(),
            caps: 0,
            nodes: vec![fanin],
            edges: vec![],
            entry: NodeId::from("fanin"),
            limits: GraphLimits::default(),
        };
        let any_out = render_graph(&spec);
        assert!(
            any_out.contains("fanin [transform: $.v.fanin]"),
            "unexpected header: {any_out}"
        );
        assert!(
            any_out.contains("(join: any, cancel_losers=false)"),
            "{any_out}"
        );

        // A plain `All`-join node (the default) shows no join annotation.
        let all_spec = linear(&[("x", "y")]);
        let all_out = render_graph(&all_spec);
        assert!(
            !all_out.contains("(join:"),
            "All-join leaked an annotation: {all_out}"
        );
    }

    #[test]
    fn show_marks_entry_node() {
        let spec = linear(&[("a", "b")]);
        let out = render_graph(&spec);
        // Only one row carries the (entry) marker, and it is the `a` row.
        assert_eq!(out.matches("(entry)").count(), 1, "output: {out}");
        let a_row = out.find("a [transform").expect("a row present");
        let b_row = out.find("b [transform").expect("b row present");
        let entry_at = out.find("(entry)").expect("entry marker present");
        assert!(
            entry_at > a_row && entry_at < b_row,
            "(entry) not on the entry row: {out}"
        );
    }

    #[test]
    fn show_renders_loop_back_reference() {
        // Load the real committed starter graph.
        let src = std::fs::read_to_string(verify_fix_path()).expect("read verify-fix.toml");
        let spec = crate::load_toml(&src).expect("parse verify-fix.toml");
        let out = render_graph(&spec);

        // The cycle closure retry->fix is a loop-back annotation, once, and
        // appears after retry's row — never as a lane-crossing `|--` arrow.
        assert_eq!(
            out.matches("^-- loops back to \"fix\" (budget=3)").count(),
            1
        );
        assert!(
            !out.contains("|-- ok --> fix"),
            "back edge leaked as a lane"
        );

        let retry_idx = out.find("retry [transform: $.payload]").expect("retry row");
        let loop_idx = out.find("^-- loops back").expect("loop annotation");
        assert!(
            loop_idx > retry_idx,
            "loop annotation before its source row: {out}"
        );

        // The whole rendered block is stable: fix/test/gate/done/retry each
        // appear exactly once (no duplication / lane-crossing mess).
        assert_eq!(out.matches("fix [agent]").count(), 1, "{out}");
        assert_eq!(out.matches("test [tool: run]").count(), 1, "{out}");
        assert_eq!(
            out.matches("gate [route: $.payload.exit_code]").count(),
            1,
            "{out}"
        );
        assert_eq!(
            out.matches("retry [transform: $.payload]").count(),
            1,
            "{out}"
        );
        assert_eq!(
            out.matches("done [transform: $.payload]").count(),
            1,
            "{out}"
        );
    }

    #[test]
    fn show_rejects_invalid_graph_like_lint_does() {
        // A V0-class error graph: an unknown node kind.
        let mut g = node("bad");
        g.kind = "definitely_not_a_kind".into();
        g.cfg = serde_json::json!({});
        let spec = GraphSpec {
            name: "t".into(),
            caps: 0,
            nodes: vec![g, node("ok")],
            edges: vec![Edge {
                from: NodeId::from("bad"),
                port: "ok".into(),
                to: NodeId::from("ok"),
                kind: EdgeKind::Data,
            }],
            entry: NodeId::from("bad"),
            limits: GraphLimits::default(),
        };
        let reg = crate::node::NodeRegistry::builtin();
        let result = crate::validate::validate(&spec, &reg);
        assert!(result.is_err(), "validation should reject the unknown kind");
        let errors = result.expect_err("is_err checked");
        let msg = errors.join("\n");
        assert!(
            msg.contains("V0: node 'bad' has unknown kind 'definitely_not_a_kind'"),
            "got: {msg}"
        );

        // A dangling edge is likewise a V0-class error, same shape.
        let dangling = {
            let mut g2 = linear(&[("a", "b")]);
            g2.edges.push(Edge {
                from: NodeId::from("a"),
                port: "ok".into(),
                to: NodeId::from("ghost"),
                kind: EdgeKind::Data,
            });
            g2
        };
        let res2 = crate::validate::validate(&dangling, &reg);
        assert!(res2.is_err());
        assert!(
            res2.expect_err("is_err checked")
                .join("\n")
                .contains("V0: edge references unknown node 'ghost'")
        );
    }

    #[test]
    fn show_output_is_deterministic() {
        let src = std::fs::read_to_string(verify_fix_path()).expect("read verify-fix.toml");
        let spec = crate::load_toml(&src).expect("parse verify-fix.toml");
        let (out1, out2) = (render_graph(&spec), render_graph(&spec));
        assert_eq!(out1, out2);
        assert_eq!(topo_order(&spec), topo_order(&spec), "topo is stable");
    }

    /// Absolute path to the committed `graphs/starter/verify-fix.toml`.
    fn verify_fix_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .expect("workspace root")
            .join("graphs/starter/verify-fix.toml")
    }
}

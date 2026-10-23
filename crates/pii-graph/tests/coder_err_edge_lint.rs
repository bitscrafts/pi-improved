//! Spec 238: the lint warns when a coder's `err` edge ends in a terminal `transform`,
//! so a failed agent finishes the run as succeeded.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_abi::{Effects, NodeId, PiiError, PortName};
use pii_core::CapKind;
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::spec::{Edge, EdgeKind};
use pii_graph::toml_load::load_toml;
use pii_graph::{GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy, validate};

/// Stand-in for a node kind with `ok` and `err` ports (the real coder lives in another crate).
struct TwoPort(&'static str);

impl NodeFactory for TwoPort {
    fn kind(&self) -> &'static str {
        self.0
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Err(PiiError::Internal("not built in this test".into()))
    }
}

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(TwoPort("coder")));
    reg.register(Arc::new(TwoPort("other")));
    reg
}

fn node(id: &str, kind: &str) -> NodeSpec {
    NodeSpec {
        id: NodeId::from(id),
        kind: kind.into(),
        join: JoinPolicy::All,
        loop_budget: None,
        retry: RetryPolicy::default(),
        caps: 0,
        cfg: serde_json::json!({"value": "v"}),
    }
}

fn edge(from: &str, port: &str, to: &str) -> Edge {
    Edge {
        from: NodeId::from(from),
        port: PortName::from(port),
        to: NodeId::from(to),
        kind: EdgeKind::Data,
    }
}

fn graph(nodes: Vec<NodeSpec>, edges: Vec<Edge>) -> GraphSpec {
    GraphSpec {
        name: "t".into(),
        caps: 0,
        nodes,
        edges,
        entry: NodeId::from("c"),
        limits: GraphLimits::default(),
    }
}

fn lint(spec: &GraphSpec) -> Vec<String> {
    validate(spec, &registry())
        .unwrap()
        .into_iter()
        .filter(|w| w.contains("finishes the run as succeeded"))
        .collect()
}

#[test]
fn a_coder_err_edge_to_a_terminal_transform_warns() {
    let spec = graph(
        vec![node("c", "coder"), node("t", "transform")],
        vec![edge("c", "err", "t")],
    );
    let w = lint(&spec);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].starts_with("V11"), "{w:?}");
    assert!(w[0].contains("'c'") && w[0].contains("'t'"), "{w:?}");
}

#[test]
fn an_err_edge_to_a_node_that_continues_does_not_warn() {
    let spec = graph(
        vec![
            node("c", "coder"),
            node("t", "transform"),
            node("u", "transform"),
        ],
        vec![edge("c", "err", "t"), edge("t", "ok", "u")],
    );
    assert!(lint(&spec).is_empty());
}

#[test]
fn an_err_edge_to_another_kind_does_not_warn() {
    let spec = graph(
        vec![node("c", "coder"), node("o", "other")],
        vec![edge("c", "err", "o")],
    );
    assert!(lint(&spec).is_empty());
}

#[test]
fn a_coder_with_no_err_edge_does_not_warn() {
    let spec = graph(
        vec![node("c", "coder"), node("t", "transform")],
        vec![edge("c", "ok", "t")],
    );
    assert!(lint(&spec).is_empty());
}

#[test]
fn a_non_coder_node_does_not_warn() {
    let spec = graph(
        vec![node("c", "other"), node("t", "transform")],
        vec![edge("c", "err", "t")],
    );
    assert!(lint(&spec).is_empty());
}

/// Spec 240: the shipped probe now ends `err` in a `refuse` node, so this reads the old shape inline instead.
#[test]
fn a_probe_shaped_graph_ending_err_in_a_transform_warns() {
    let src = r#"
name = "claude-probe-old-shape"
entry = "code"

[[node]]
id = "code"
kind = "coder"

[[node]]
id = "done"
kind = "transform"
value = "done"

[[node]]
id = "refused"
kind = "transform"
value = "refused"

[[edge]]
from = "code"
port = "ok"
to = "done"
kind = "data"

[[edge]]
from = "code"
port = "err"
to = "refused"
kind = "data"
"#;
    let spec = load_toml(src).unwrap();
    let w = lint(&spec);
    assert_eq!(w.len(), 1, "{w:?}");
}

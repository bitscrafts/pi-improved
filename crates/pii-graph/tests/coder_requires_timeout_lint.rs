//! Spec 247: V10 also covers a node whose KIND is `coder`, because it spawns an agent that can run
//! for as long as it likes although its factory declares no `SPAWNS_PROC`. The graphs are read from
//! `graphs/` and loaded with a stand-in `coder` kind (the real one lives in `pii-server`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::Arc;

use pii_abi::{Effects, NodeId, PiiError, PortName};
use pii_core::CapKind;
use pii_graph::fragment;
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::{GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy, validate};

/// Stand-in for the coder kind: `ok`/`err` ports and `Effects::PURE`, exactly like the real coder's
/// lack of `SPAWNS_PROC`. V10 must not depend on the effects to see it.
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

fn node(id: &str, kind: &str, cfg: serde_json::Value) -> NodeSpec {
    NodeSpec {
        id: NodeId::from(id),
        kind: kind.into(),
        join: JoinPolicy::All,
        loop_budget: None,
        retry: RetryPolicy::default(),
        caps: 0,
        cfg,
    }
}

fn graph(nodes: Vec<NodeSpec>) -> GraphSpec {
    GraphSpec {
        name: "t".into(),
        caps: 0,
        nodes,
        edges: Vec::new(),
        entry: NodeId::from("c"),
        limits: GraphLimits::default(),
    }
}

/// Every message `validate` produced, whether it returned `Ok(warnings)` or `Err(errors)`.
fn messages(spec: &GraphSpec) -> Vec<String> {
    match validate(spec, &registry()) {
        Ok(warnings) => warnings,
        Err(errors) => errors,
    }
}

fn v10(spec: &GraphSpec) -> Vec<String> {
    messages(spec)
        .into_iter()
        .filter(|m| m.contains("V10"))
        .collect()
}

/// Loads a shipped graph as the daemon does: `[[attach]]` tables (the podman guard) are woven in from `graphs/fragments`.
fn shipped(name: &str) -> GraphSpec {
    let graphs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs");
    let src = std::fs::read_to_string(graphs.join(format!("{name}.toml"))).unwrap();
    fragment::load_host(&src, &graphs.join("fragments"), &registry())
        .unwrap()
        .spec
}

const SHIPPED: [&str; 4] = [
    "claude-probe",
    "coder-selffix",
    "coder-demo",
    "coder-podman",
];

#[test]
fn a_coder_node_without_timeout_is_a_v10_error() {
    let spec = graph(vec![node("c", "coder", serde_json::json!({}))]);
    let e = v10(&spec);
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(e[0].contains("'c'"), "the message names the node: {e:?}");
}

#[test]
fn a_coder_node_with_zero_timeout_is_a_v10_error() {
    let spec = graph(vec![node("c", "coder", serde_json::json!({"timeout_ms": 0}))]);
    let e = v10(&spec);
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(e[0].contains("'c'"), "the message names the node: {e:?}");
}

#[test]
fn a_coder_node_with_a_timeout_passes_v10() {
    let spec = graph(vec![node(
        "c",
        "coder",
        serde_json::json!({"timeout_ms": 1_800_000}),
    )]);
    assert_eq!(v10(&spec), [] as [String; 0]);
}

#[test]
fn a_non_coder_node_without_timeout_is_not_a_v10_error() {
    let spec = graph(vec![node("c", "other", serde_json::json!({}))]);
    assert_eq!(v10(&spec), [] as [String; 0]);
}

#[test]
fn the_shipped_coder_graphs_pass_v10() {
    for name in SHIPPED {
        let e = v10(&shipped(name));
        assert!(e.is_empty(), "{name}: {e:?}");
    }
}

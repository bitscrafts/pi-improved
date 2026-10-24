//! Spec 259 R8: `graphs/spec.toml` judges the coder's work before it reaches `done`. A `verify` judge sits between `code`
//! and `done`; its failure routes to `unverified`, a refusal with no way out. The stand-in kinds never run: the graph is
//! only loaded and checked here, the real `coder` and `judge` live in `pii-server`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::{GraphSpec, load_toml_with_registry, validate};

/// Stand-in for a kind the graph names: the given ports, no effects, and it never runs.
struct Stub {
    kind: &'static str,
    ports: &'static [&'static str],
}

impl NodeFactory for Stub {
    fn kind(&self) -> &'static str {
        self.kind
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(self.ports.iter().map(|p| PortName::from(*p)).collect())
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(Never))
    }
}

struct Never;

impl GraphNode for Never {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        panic!("the graph is only loaded here, never run");
    }
}

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(Stub {
        kind: "coder",
        ports: &["ok", "err"],
    }));
    reg.register(Arc::new(Stub {
        kind: "judge",
        ports: &["ok", "fail"],
    }));
    reg
}

fn tracked_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs/spec.toml");
    std::fs::read_to_string(path).unwrap()
}

fn loaded() -> GraphSpec {
    load_toml_with_registry(&tracked_source(), &registry()).unwrap()
}

/// Whether the graph has an edge `from.port -> to`.
fn has_edge(spec: &GraphSpec, from: &str, port: &str, to: &str) -> bool {
    spec.edges
        .iter()
        .any(|e| e.from.to_string() == from && e.port.to_string() == port && e.to.to_string() == to)
}

/// Whether any string value in `doc` starts with `/`: an absolute path a graph must not carry.
fn has_absolute_path(doc: &toml::Value) -> bool {
    match doc {
        toml::Value::String(s) => s.starts_with('/'),
        toml::Value::Array(items) => items.iter().any(has_absolute_path),
        toml::Value::Table(map) => map.values().any(has_absolute_path),
        _ => false,
    }
}

#[test]
fn the_graph_routes_ok_through_verify_to_done() {
    let spec = loaded();
    assert!(has_edge(&spec, "code", "ok", "verify"), "code.ok -> verify");
    assert!(has_edge(&spec, "verify", "ok", "done"), "verify.ok -> done");
    assert!(
        has_edge(&spec, "verify", "fail", "unverified"),
        "verify.fail -> unverified"
    );
    assert!(
        has_edge(&spec, "code", "err", "refused"),
        "code.err -> refused"
    );
    assert!(!has_edge(&spec, "code", "ok", "done"), "no code.ok -> done");
}

#[test]
fn a_failed_verdict_cannot_reach_done() {
    let spec = loaded();
    let node = spec
        .nodes
        .iter()
        .find(|n| n.id.to_string() == "unverified")
        .expect("an unverified node");
    assert_eq!(node.kind, "refuse");
    assert_eq!(node.cfg["hook"], "judge");
    assert_eq!(node.cfg["reason"], "the spec's criteria failed");
    assert!(
        spec.edges
            .iter()
            .all(|e| e.from.to_string() != "unverified"),
        "unverified has no outgoing edge"
    );
}

#[test]
fn the_graph_still_validates_with_no_absolute_path() {
    let src = tracked_source();
    let spec = load_toml_with_registry(&src, &registry()).unwrap();
    assert!(validate(&spec, &registry()).is_ok(), "no validation errors");
    let doc: toml::Value = toml::from_str(&src).unwrap();
    assert!(
        !has_absolute_path(&doc),
        "no absolute path in graphs/spec.toml"
    );
}

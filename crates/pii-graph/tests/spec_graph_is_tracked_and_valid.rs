//! Spec 257 R6: `graphs/spec.toml` is the one tracked graph that reads the run input. It loads and validates, it names no
//! absolute path or credential, and its time bound is a literal integer (V10 needs one). The stand-in coder kind is the
//! same as the shipped-graph test's: the real one lives in `pii-server`.

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
use pii_graph::{load_toml_with_registry, validate};

/// Stand-in for the coder kind: ports `ok` and `err`, and it never runs.
struct StubCoder;

impl NodeFactory for StubCoder {
    fn kind(&self) -> &'static str {
        "coder"
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
        Ok(Box::new(Never))
    }
}

struct Never;

impl GraphNode for Never {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        panic!("the graph is only loaded here, never run");
    }
}

/// Stand-in for the judge kind (spec 259 put a `verify` judge in the graph): ports `ok` and `fail`, and it never runs.
struct StubJudge;

impl NodeFactory for StubJudge {
    fn kind(&self) -> &'static str {
        "judge"
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("fail")])
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

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(StubCoder));
    reg.register(Arc::new(StubJudge));
    reg
}

fn tracked_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs/spec.toml");
    std::fs::read_to_string(path).unwrap()
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

fn parsed(src: &str) -> toml::Value {
    toml::from_str(src).unwrap()
}

#[test]
fn spec_graph_loads_validates_and_has_no_absolute_path() {
    let src = tracked_source();
    let spec = load_toml_with_registry(&src, &registry()).unwrap();
    // Warnings are allowed (the `done` and `refused` transforms declare the default ports); errors are not.
    validate(&spec, &registry()).expect("no validation errors");
    assert!(
        !has_absolute_path(&parsed(&src)),
        "an absolute path in graphs/spec.toml"
    );
}

#[test]
fn a_graph_with_an_absolute_path_is_caught_by_the_same_check() {
    let src = tracked_source().replace("workdir = \"$.input.workdir\"", "workdir = \"/x\"");
    assert!(src.contains("workdir = \"/x\""), "the substitution applied");
    assert!(has_absolute_path(&parsed(&src)), "the check can fail");
}

#[test]
fn spec_graph_timeout_is_a_literal() {
    let doc = parsed(&tracked_source());
    let code = doc["node"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"].as_str() == Some("code"))
        .expect("a code node");
    let timeout = code["timeout_ms"]
        .as_integer()
        .expect("timeout_ms is an integer");
    assert!(timeout > 0, "timeout_ms: {timeout}");
}

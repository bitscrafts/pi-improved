//! Spec 240: the shipped coder graphs end a failed agent in a `refuse` node, so the run fails. The `ok` path still ends
//! in a transform. The graphs are read from `graphs/` and loaded with a stand-in `coder` kind (the real one lives in
//! `pii-server`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use common::{ctx, scratch};
use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName, Terminal};
use pii_core::{CapKind, Ctx};
use pii_graph::fragment;
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::spec::GraphSpec;
use pii_graph::{load_toml_with_registry, run, validate};
use pii_store::Store;

const SHIPPED: [&str; 4] = [
    "claude-probe",
    "coder-selffix",
    "coder-demo",
    "coder-podman",
];

/// Stand-in for the coder kind: ports `ok` and `err`, and it always takes `err`, as a non-zero agent exit does.
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
        Ok(Box::new(FailedAgent))
    }
}

struct FailedAgent;

impl GraphNode for FailedAgent {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        Ok(Outcome::Next {
            port: PortName::from("err"),
            value: serde_json::json!({"error": "agent exited 1"}),
        })
    }
}

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(StubCoder));
    reg
}

/// Loads a shipped graph as the daemon does: `[[attach]]` tables (the podman guard) are woven in from `graphs/fragments`.
fn shipped(name: &str) -> GraphSpec {
    let graphs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs");
    let src = std::fs::read_to_string(graphs.join(format!("{name}.toml"))).unwrap();
    fragment::weave(&src, &graphs.join("fragments"), &registry())
        .unwrap()
        .spec
}

/// The kind of the node that the coder's `port` edge reaches.
fn coder_target_kind(spec: &GraphSpec, port: &str) -> String {
    let coder = spec.nodes.iter().find(|n| n.kind == "coder").unwrap();
    let edge = spec
        .edges
        .iter()
        .find(|e| e.from == coder.id && e.port == PortName::from(port))
        .unwrap();
    spec.nodes
        .iter()
        .find(|n| n.id == edge.to)
        .unwrap()
        .kind
        .clone()
}

fn v11(spec: &GraphSpec) -> Vec<String> {
    validate(spec, &registry())
        .unwrap()
        .into_iter()
        .filter(|w| w.contains("V11"))
        .collect()
}

#[test]
fn each_shipped_coder_graph_ends_err_in_a_refuse_node() {
    for name in SHIPPED {
        assert_eq!(coder_target_kind(&shipped(name), "err"), "refuse", "{name}");
    }
}

#[test]
fn no_shipped_coder_graph_carries_the_v11_warning() {
    for name in SHIPPED {
        let w = v11(&shipped(name));
        assert!(w.is_empty(), "{name}: {w:?}");
    }
}

#[test]
fn the_ok_path_still_ends_in_a_transform() {
    for name in SHIPPED {
        assert_eq!(
            coder_target_kind(&shipped(name), "ok"),
            "transform",
            "{name}"
        );
    }
}

#[test]
fn a_graph_that_still_ends_err_in_a_transform_still_warns() {
    let src = r#"
name = "old-shape"
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
    let spec = load_toml_with_registry(src, &registry()).unwrap();
    let w = v11(&spec);
    assert_eq!(w.len(), 1, "{w:?}");
}

#[test]
fn a_failed_agent_fails_the_run() {
    let spec = shipped("claude-probe");
    let dir = scratch("shipped-coder-fails-");
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let id = store.next_run_id().unwrap();
    let terminal = run::run(
        &store,
        &spec,
        &registry(),
        &ctx(id.clone()),
        &id,
        serde_json::Value::Null,
    )
    .unwrap();
    match terminal {
        Terminal::Failure { error } => assert!(error.contains("run.refused"), "{error}"),
        other => panic!("a failed agent must fail the run, got {other:?}"),
    }
}

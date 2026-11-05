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
use pii_graph::fragment::load_host_with_graphs;
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::{GraphSpec, validate};

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
        // Must mirror the REAL judge (crates/pii-server/src/node_kinds/nodes/judge_spec.rs): a spec-mode
        // judge reports a criterion it cannot run as `unrunnable`, and sdd routes that verdict to
        // `refused`. A stub that lists fewer ports makes a legal graph look broken — the same drift that
        // 381 was about, one level down.
        ports: &["ok", "fail", "unrunnable"],
    }));
    reg
}

/// The repo's `graphs/` directory: where the `docs` graph that `spec.toml` calls lives (spec 272).
fn graphs_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

fn tracked_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs/spec.toml");
    std::fs::read_to_string(path).unwrap()
}

fn loaded() -> GraphSpec {
    load_host_with_graphs(
        &tracked_source(),
        &graphs_dir(),
        &[graphs_dir()],
        &registry(),
    )
    .unwrap()
    .spec
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
    // Spec 385: the implementer is a METHODOLOGY the driver picks, so `code` and `verify` now live one
    // level down, namespaced by the call that inlined them. The properties are unchanged; only the ids
    // moved. Asserting the old bare ids asserted a shape the shipped graph no longer has.
    // `pick` routes on $.input.method, so its ports are the METHOD NAMES, not ok/err: choosing a
    // methodology is a route, and the chosen graph is inlined behind the port carrying its name.
    assert!(
        has_edge(&spec, "pick", "method-judge", "method-judge__in"),
        "pick --method-judge--> the method-judge graph"
    );
    assert!(
        has_edge(&spec, "pick", "sdd", "sdd__in"),
        "pick --sdd--> the sdd graph"
    );
    assert!(
        has_edge(&spec, "method-judge__code", "ok", "method-judge__verify"),
        "code.ok -> verify"
    );
    // Spec 272 put the documentation step between the verdict and `done`, and judges again after it:
    // a verified methodology still has to reach the host's `passed`/`reverify`, not jump to `done`.
    assert!(
        !has_edge(&spec, "method-judge__verify", "ok", "done"),
        "verify.ok must not skip the docs step"
    );
    assert!(
        has_edge(&spec, "reverify", "ok", "done"),
        "reverify.ok -> done"
    );
    assert!(
        has_edge(&spec, "reverify", "fail", "unverified"),
        "reverify.fail -> unverified"
    );
    assert!(
        has_edge(&spec, "method-judge__code", "err", "method-judge__refused"),
        "a coder that fails reaches a refusal"
    );
    assert!(
        !has_edge(&spec, "method-judge__code", "ok", "done"),
        "no code.ok -> done"
    );
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
    let spec = load_host_with_graphs(&src, &graphs_dir(), &[graphs_dir()], &registry())
        .unwrap()
        .spec;
    let vres = validate(&spec, &registry());
    assert!(
        vres.is_ok(),
        "the shipped spec graph must validate: {vres:?}"
    );
    let doc: toml::Value = toml::from_str(&src).unwrap();
    assert!(
        !has_absolute_path(&doc),
        "no absolute path in graphs/spec.toml"
    );
}

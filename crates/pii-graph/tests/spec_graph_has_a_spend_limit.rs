//! Spec 282: `graphs/spec.toml` carries a `[limits] max_micro_usd`, the run's spend bound (spec 276 makes it real for the
//! coder). The called graph `docs.toml` carries none: a `call` is inlined, so it shares the host's budget. The stand-in
//! kinds never run here; the run-level proof that the limit bites lives in `pii-server`.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::fragment::{load_host_with_graphs, sha256_hex};
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::{GraphSpec, load_toml_with_registry};

/// The run budget of a graph without `[limits]`: `GraphLimits::default()`.
const UNLIMITED: u64 = u64::MAX;

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

/// Stand-in for the judge kind: ports `ok` and `fail`, and it never runs.
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

struct Never;

impl GraphNode for Never {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        panic!("the graph is only loaded here, never run");
    }
}

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(StubCoder));
    reg.register(Arc::new(StubJudge));
    reg
}

/// The repo's `graphs/` directory: where `spec.toml` and the `docs` graph it calls live.
fn graphs_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

fn read_graph(name: &str) -> String {
    std::fs::read_to_string(graphs_dir().join(name)).unwrap()
}

/// The tracked `spec.toml`, loaded the way the daemon loads a host: `call` nodes inlined from `graphs/`.
fn tracked_spec() -> GraphSpec {
    load_host_with_graphs(
        &read_graph("spec.toml"),
        &graphs_dir(),
        &[graphs_dir()],
        &registry(),
    )
    .unwrap()
    .spec
}

#[test]
fn the_tracked_spec_graph_declares_the_spend_limit() {
    // (a): the shipped graph carries the policy number (4.00 USD since 2026-11-06, spec 282 R1: a reviewer-model run spent the old 1.00 reading the code).
    assert_eq!(tracked_spec().limits.max_micro_usd, 4_000_000);
}

#[test]
fn the_spend_limit_is_not_the_unlimited_default() {
    // (b): a graph without `[limits]` gets `u64::MAX`; the tracked graph must not.
    assert_ne!(tracked_spec().limits.max_micro_usd, UNLIMITED);
}

#[test]
fn CONTROL_the_called_docs_graph_has_no_budget_of_its_own() {
    // (c): `docs` is expanded INLINE by `call`, so it shares the host's `Ctx` and budget. A `[limits]` written in
    // `docs.toml` would be ignored, so the called graph stays unlimited on its own and the host carries the bound.
    let docs = load_toml_with_registry(&read_graph("docs.toml"), &registry()).unwrap();
    assert_eq!(docs.limits.max_micro_usd, UNLIMITED);
}

#[test]
fn CONTROL_docs_toml_is_the_digest_spec_toml_pins() {
    // (d): this change did not edit the called graph. The pin is read from `spec.toml`, not hard-coded here.
    //
    // The pin must be the one belonging to the `docs` CALL. Reading the first `sha256` in the file was
    // true only while `spec.toml` made a single call: spec 385 added a methodology route, so two pins now
    // precede the docs one and the control compared the docs file against the sdd digest.
    let spec_src = read_graph("spec.toml");
    let pin = spec_src
        .lines()
        .scan(false, |after_docs, line| {
            if line.trim() == "graph = \"docs\"" {
                *after_docs = true;
            }
            Some((*after_docs, line))
        })
        .filter(|(after_docs, _)| *after_docs)
        .find_map(|(_, l)| l.trim().strip_prefix("sha256 = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("spec.toml pins the docs graph by sha256");
    assert_eq!(sha256_hex(read_graph("docs.toml").as_bytes()), pin);
}

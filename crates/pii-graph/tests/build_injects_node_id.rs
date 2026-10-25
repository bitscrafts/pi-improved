//! Spec 271 R1, R4: a node's factory is built with the node's id in its config as `node-id`. A cfg that already carries
//! `node-id` keeps its own value, and the spec the run journals (its cfg and its hash) is not changed by the build.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName, RunId};
use pii_core::{Budget, CapKind, CapSet, Ctx, NullObserver, TraceCtx};
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::{GraphSpec, load_toml_with_registry, run};
use pii_store::Store;

/// Records the config each of its nodes is built with.
#[derive(Default)]
struct Probe(Arc<Mutex<Vec<serde_json::Value>>>);

/// A node that finishes on its `ok` port at once.
struct Quiet;

impl GraphNode for Quiet {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({}),
        })
    }
}

impl NodeFactory for Probe {
    fn kind(&self) -> &'static str {
        "probe"
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn effects(&self) -> Effects {
        Effects::empty()
    }
    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        self.0.lock().push(cfg.clone());
        Ok(Box::new(Quiet))
    }
}

/// Loads and runs a one-node graph whose node `a` is `node_cfg`, and returns the configs it was built with, the
/// spec as loaded, and the spec's JSON before the run.
fn build_one(node_cfg: &str) -> (Vec<serde_json::Value>, GraphSpec, Vec<u8>) {
    let probe = Arc::new(Probe::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(probe.clone());
    let src = format!(
        "name = \"ids\"\nentry = \"a\"\n\n[[node]]\nid = \"a\"\nkind = \"probe\"\n{node_cfg}\n"
    );
    let spec = load_toml_with_registry(&src, &reg).unwrap();
    let before = spec.to_json().unwrap();
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "build-id-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    let run_id: RunId = store.next_run_id().unwrap();
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::new(NullObserver),
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    run::run_with_validate(
        &store,
        &spec,
        &reg,
        &ctx,
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let built = probe.0.lock().clone();
    assert_eq!(
        spec.to_json().unwrap(),
        before,
        "the spec is unchanged by the run"
    );
    (built, spec, before)
}

#[test]
fn a_factory_sees_its_node_id() {
    let (built, _, _) = build_one("");
    assert_eq!(built.len(), 1);
    assert_eq!(
        built[0]["node-id"], "a",
        "the built cfg names its node: {}",
        built[0]
    );
}

#[test]
fn CONTROL_an_existing_node_id_is_kept() {
    let (built, _, _) = build_one("node-id = \"mine\"");
    assert_eq!(built[0]["node-id"], "mine", "{}", built[0]);
}

#[test]
fn the_spec_and_its_hash_are_unchanged() {
    let (_, spec, before) = build_one("");
    assert_eq!(spec.to_json().unwrap(), before);
    assert!(
        spec.nodes[0].cfg.get("node-id").is_none(),
        "the spec's own cfg carries no node-id: {}",
        spec.nodes[0].cfg
    );
}

//! Spec 100 R3: the graph engine's metrics reach a `pii-observe` observer by
//! their registered names (via the `ContractObserver` bridge on the `Ctx`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_core::{Budget, CapSet, ContractObserver, Ctx, Observer, TraceCtx};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::toml_load::load_toml;
use pii_observe::{Recorder, names};
use pii_store::Store;

fn run_example(name: &str) -> Arc<Recorder> {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("crate-local _tmp");
    let ws = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let src = std::fs::read_to_string(ws.join("examples").join(name)).expect("read example");
    let spec = load_toml(&src).expect("load example");
    let dir = tempfile::Builder::new()
        .prefix("observe-contract-")
        .tempdir_in(root)
        .expect("tempdir under _tmp");
    let store = Store::open(&dir.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let rec = Arc::new(Recorder::new());
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::new(ContractObserver::new(rec.clone())) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    run::run(
        &store,
        &spec,
        &NodeRegistry::builtin(),
        &ctx,
        &run_id,
        serde_json::json!({}),
    )
    .expect("run");
    rec
}

#[test]
fn batch_width_reaches_the_contract_observer_by_name() {
    let rec = run_example("diamond.toml");
    let widths = rec.events_named(names::GRAPH_BATCH_WIDTH);
    assert!(
        widths
            .iter()
            .any(|e| e.fields.get("value").and_then(serde_json::Value::as_u64) >= Some(2)),
        "want a graph.batch.width event with value >= 2, got {widths:?}"
    );
}

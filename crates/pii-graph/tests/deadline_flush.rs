//! Spec 168/179: the in-flight barrier is a deadline on the oldest unflushed event. A node that
//! outlives the window has its `NodeStarted` made durable while it runs; a fast node costs no
//! extra transaction.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::RunEvent;
use pii_core::{Budget, CapSet, Ctx, NullObserver, Observer, TraceCtx};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::toml_load::load_toml;
use pii_store::Store;

const SLOW: &str = "name = \"slow\"\nentry = \"nap\"\ncaps = []\n[limits]\nmax_steps = 10\nmax_wall_ms = 30000\n[[node]]\nid = \"nap\"\nkind = \"wait\"\ntimer_ms = 3500\n";
const FAST: &str = "name = \"fast\"\nentry = \"step\"\ncaps = []\n[limits]\nmax_steps = 10\nmax_wall_ms = 30000\n[[node]]\nid = \"step\"\nkind = \"transform\"\nvalue = \"ok\"\n";

fn ctx(run_id: &pii_abi::RunId) -> Ctx {
    Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::new(NullObserver) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    )
}

fn started(store: &Store, run_id: &pii_abi::RunId) -> bool {
    store
        .read_journal(run_id)
        .unwrap()
        .iter()
        .any(|(_, e)| matches!(e, RunEvent::NodeStarted { .. }))
}

fn scratch() -> tempfile::TempDir {
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("deadline-flush-")
        .tempdir_in(root)
        .unwrap()
}

#[test]
fn a_node_that_outlives_the_window_has_its_start_durable_while_it_runs() {
    let dir = scratch();
    let store = Arc::new(Store::open(&dir.path().join("db.redb")).unwrap());
    let spec = load_toml(SLOW).unwrap();
    let run_id = store.next_run_id().unwrap();
    let (s2, r2) = (Arc::clone(&store), run_id.clone());
    let worker = std::thread::spawn(move || {
        let reg = NodeRegistry::builtin();
        run::run(&s2, &spec, &reg, &ctx(&r2), &r2, serde_json::json!({})).unwrap();
    });
    // The node sleeps 3.5 s; the window is 2 s. Between them, the start must reach the store.
    let end = Instant::now() + Duration::from_secs(3);
    let mut durable = false;
    while Instant::now() < end && !worker.is_finished() {
        if started(&store, &run_id) {
            durable = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    worker.join().unwrap();
    assert!(
        durable,
        "NodeStarted was not durable while the node still ran"
    );
}

#[test]
fn a_fast_node_costs_no_barrier_transaction() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let spec = load_toml(FAST).unwrap();
    let reg = NodeRegistry::builtin();
    // Warm the id block and the first-sight bodies, then measure a run.
    let warm = store.next_run_id().unwrap();
    run::run(
        &store,
        &spec,
        &reg,
        &ctx(&warm),
        &warm,
        serde_json::json!({}),
    )
    .unwrap();
    let id = store.next_run_id().unwrap();
    let before = store.write_txn_count();
    run::run(&store, &spec, &reg, &ctx(&id), &id, serde_json::json!({})).unwrap();
    let txns = store.write_txn_count() - before;
    assert!(
        txns <= 2,
        "a fast run cost {txns} transactions; expected at most 2"
    );
}

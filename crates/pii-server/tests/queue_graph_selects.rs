//! Spec 208: the queue's `select` node is a gate. An item whose `needs` are unmet is never dispatched, the run
//! output names what it waits for, and two ticks racing for the queue dispatch one item, not two.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_abi::Terminal;
use pii_graph::NodeRegistry;
use pii_server::node_kinds::nodes::queue::SelectFactory;
use pii_store::queue::{ItemStatus, QueueStore, WorkItem};

/// A queue store over a scratch file, seeded with `items` in the order given.
fn seeded(items: &[WorkItem]) -> (tempfile::TempDir, Arc<QueueStore>) {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-store-").unwrap();
    let store = Arc::new(QueueStore::open(&dir.path().join("queue.redb")).unwrap());
    for item in items {
        store.put_item(item).unwrap();
    }
    (dir, store)
}

/// Runs the one-node `select` graph over `queue` and returns its terminal. `ok` continues to a `dispatched`
/// transform, so a claim is visible in the terminal; every other port ends the run with the select's own output.
fn run_select(queue: Arc<QueueStore>) -> Terminal {
    let run_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-run-").unwrap();
    let run_store = pii_store::Store::open(&run_dir.path().join("store.redb")).unwrap();
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(SelectFactory { store: queue }));
    let src = "name = \"select-test\"\nentry = \"select\"\n\n\
        [[node]]\nid = \"select\"\nkind = \"select\"\n\n\
        [[node]]\nid = \"dispatched\"\nkind = \"transform\"\nvalue = \"dispatched\"\n\n\
        [[edge]]\nfrom = \"select\"\nport = \"ok\"\nto = \"dispatched\"\nkind = \"data\"\n";
    let spec = pii_graph::load_toml_with_registry(src, &reg).unwrap();
    let run_id = run_store.next_run_id().unwrap();
    let mut caps = pii_core::CapSet::empty();
    caps.kinds = pii_core::CapKind::empty();
    let ctx = pii_core::Ctx::root(caps, pii_core::Budget::unlimited());
    pii_graph::run::run_with_validate(
        &run_store,
        &spec,
        &reg,
        &ctx,
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap()
}

/// `A1` needs `A2`, and `A2` is parked: `A1` is not dispatched, and the run output names `A2` as the thing it
/// waits for. Nothing is claimed, so `A1` stays pending.
#[test]
fn select_skips_unmet_needs() {
    let mut a2 = WorkItem::new("A2", &[]);
    a2.status = ItemStatus::Parked;
    let (_dir, store) = seeded(&[WorkItem::new("A1", &["A2"]), a2]);

    let terminal = run_select(Arc::clone(&store));

    let Terminal::Success { output } = terminal else {
        panic!("select should end the run on its blocked port, got {terminal:?}");
    };
    let summary = output["summary"].as_str().unwrap_or_default();
    assert!(summary.contains("A1 waits on A2"), "{summary}");
    assert_eq!(
        store.get_item("A1").unwrap().unwrap().status,
        ItemStatus::Pending
    );
    assert!(
        store.events("A1").unwrap().is_empty(),
        "A1 was never claimed"
    );
}

/// Two ticks race for the same queue. Exactly one claims the item and dispatches it; the other reports
/// `already-running` and journals nothing new for it.
#[test]
fn select_is_idempotent() {
    let (_dir, store) = seeded(&[WorkItem::new("A1", &[])]);

    let ticks: Vec<Terminal> = std::thread::scope(|s| {
        let first = s.spawn(|| run_select(Arc::clone(&store)));
        let second = s.spawn(|| run_select(Arc::clone(&store)));
        vec![first.join().unwrap(), second.join().unwrap()]
    });

    let dispatched = ticks
        .iter()
        .filter(|t| matches!(t, Terminal::Success { output } if output == "dispatched"))
        .count();
    let already = ticks
        .iter()
        .filter(
            |t| matches!(t, Terminal::Success { output } if output["outcome"] == "already-running"),
        )
        .count();
    assert_eq!(dispatched, 1, "exactly one tick dispatches: {ticks:?}");
    assert_eq!(
        already, 1,
        "the other tick reports already-running: {ticks:?}"
    );
    let claims = store
        .events("A1")
        .unwrap()
        .iter()
        .filter(|e| e.kind == "item.claimed")
        .count();
    assert_eq!(claims, 1, "one claim journalled");
}

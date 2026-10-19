//! Spec 208: blocked and empty print differently. When every pending row waits on something, the run names the
//! blocking edge; `queue-empty` is reserved for a queue with nothing pending at all.

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

/// Runs the one-node `select` graph over `queue`; every port ends the run with the select's own output.
fn run_select(queue: Arc<QueueStore>) -> Terminal {
    let run_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-run-").unwrap();
    let run_store = pii_store::Store::open(&run_dir.path().join("store.redb")).unwrap();
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(SelectFactory { store: queue }));
    let src = "name = \"select-blocked\"\nentry = \"select\"\n\n\
        [[node]]\nid = \"select\"\nkind = \"select\"\n";
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

/// `A1` waits on `A2` (pending), and `A2` waits on `A3` (parked). Every pending row is blocked, so the run
/// names the blocking edge and never says `queue-empty`.
#[test]
fn all_pending_blocked_names_the_edge() {
    let mut a3 = WorkItem::new("A3", &[]);
    a3.status = ItemStatus::Parked;
    let (_dir, store) = seeded(&[
        WorkItem::new("A1", &["A2"]),
        WorkItem::new("A2", &["A3"]),
        a3,
    ]);

    let Terminal::Success { output } = run_select(store) else {
        panic!("blocked is a result, not a failure");
    };

    assert_eq!(output["outcome"], "blocked", "{output}");
    let summary = output["summary"].as_str().unwrap_or_default();
    assert!(summary.contains("A2 waits on A3 (parked)"), "{summary}");
    assert!(!output.to_string().contains("queue-empty"), "{output}");
}

/// A queue with nothing pending at all is the one case that says `queue-empty`.
#[test]
fn nothing_pending_says_queue_empty() {
    let mut done = WorkItem::new("A1", &[]);
    done.status = ItemStatus::Done;
    let (_dir, store) = seeded(&[done]);

    let Terminal::Success { output } = run_select(store) else {
        panic!("queue-empty is a result, not a failure");
    };

    assert_eq!(output["outcome"], "queue-empty", "{output}");
}

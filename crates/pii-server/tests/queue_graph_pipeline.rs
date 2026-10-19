//! Spec 208: one tick of the queue graph runs select → dispatch → judge → record. A fake `pi` does the work; the
//! item's criteria decide, and the verdict lands in the queue store's status and journal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::Arc;

use pii_abi::Terminal;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::queue::{JudgeFactory, RecordFactory, SelectFactory};
use pii_store::MemStore;
use pii_store::queue::{ItemStatus, QueueStore, WorkItem};

#[derive(Default)]
struct Silent;

impl Observer for Silent {
    fn event(&self, _e: &Event) {}
}

/// A scratch bin directory holding a fake `pi` that writes `ran.txt` into its working directory.
fn fake_pi() -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-bin-").unwrap();
    let pi = dir.path().join("pi");
    std::fs::write(&pi, "#!/bin/sh\necho ran > \"$PWD/ran.txt\"\n").unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    dir
}

fn root_ctx(bin: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// Runs the queue graph over `queue` with `workdir` as the coder's and the judge's working directory.
fn tick(queue: Arc<QueueStore>, memory: Arc<MemStore>, bin: &Path, workdir: &Path) -> Terminal {
    let run_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-run-").unwrap();
    let run_store = pii_store::Store::open(&run_dir.path().join("store.redb")).unwrap();
    let log: Arc<dyn Observer> = Arc::new(Silent);
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(SelectFactory {
        store: Arc::clone(&queue),
    }));
    reg.register(Arc::new(CoderFactory::with_env(
        bin.to_path_buf(),
        std::collections::BTreeMap::new(),
        log,
    )));
    reg.register(Arc::new(JudgeFactory));
    reg.register(Arc::new(RecordFactory {
        store: queue,
        memory: Some(memory),
    }));
    let work = workdir.display();
    let src = format!(
        "name = \"queue\"\nentry = \"select\"\ncaps = [\"spawn\", \"proc\"]\n\n\
        [[node]]\nid = \"select\"\nkind = \"select\"\n\n\
        [[node]]\nid = \"dispatch\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m\"\n\
        workdir = \"{work}\"\nbrief = \"do it\"\n\n\
        [[node]]\nid = \"judge\"\nkind = \"judge\"\ncaps = [\"proc\"]\nworkdir = \"{work}\"\n\n\
        [[node]]\nid = \"record\"\nkind = \"record\"\njoin = \"any\"\n\n\
        [[edge]]\nfrom = \"select\"\nport = \"ok\"\nto = \"dispatch\"\nkind = \"data\"\n\n\
        [[edge]]\nfrom = \"dispatch\"\nport = \"ok\"\nto = \"judge\"\nkind = \"data\"\n\n\
        [[edge]]\nfrom = \"dispatch\"\nport = \"err\"\nto = \"record\"\nkind = \"data\"\n\n\
        [[edge]]\nfrom = \"judge\"\nport = \"ok\"\nto = \"record\"\nkind = \"data\"\n\n\
        [[edge]]\nfrom = \"judge\"\nport = \"fail\"\nto = \"record\"\nkind = \"data\"\n"
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let run_id = run_store.next_run_id().unwrap();
    pii_graph::run::run_with_validate(
        &run_store,
        &spec,
        &reg,
        &root_ctx(bin),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap()
}

/// `A1`'s criteria pass after the fake agent has run: the item is `Done`, its journal shows the claim and the
/// verdict, and the run's terminal is the record.
#[test]
fn green_criteria_mark_the_item_done() {
    let bin = fake_pi();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-work-").unwrap();
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-store-").unwrap();
    let queue = Arc::new(QueueStore::open(&dir.path().join("queue.redb")).unwrap());
    let mut item = WorkItem::new("A1", &[]);
    item.spec = "specs/208-the-queue-is-a-graph.md".to_owned();
    item.criteria = vec!["test -f ran.txt".to_owned()];
    queue.put_item(&item).unwrap();

    let memory = Arc::new(open_memory());
    let terminal = tick(
        Arc::clone(&queue),
        Arc::clone(&memory),
        bin.path(),
        work.path(),
    );

    let Terminal::Success { output } = terminal else {
        panic!("a green tick succeeds: {terminal:?}");
    };
    assert_eq!(output["status"], "done", "{output}");
    assert_eq!(
        queue.get_item("A1").unwrap().unwrap().status,
        ItemStatus::Done
    );
    let kinds: Vec<String> = queue
        .events("A1")
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(kinds, ["item.claimed", "item.done"], "{kinds:?}");
}

/// A failing criterion parks the item, and the parked reason names that criterion.
#[test]
fn red_criterion_parks_the_item_and_names_it() {
    let bin = fake_pi();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-work-").unwrap();
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-store-").unwrap();
    let queue = Arc::new(QueueStore::open(&dir.path().join("queue.redb")).unwrap());
    let mut item = WorkItem::new("A1", &[]);
    item.spec = "specs/208-the-queue-is-a-graph.md".to_owned();
    item.criteria = vec!["false-criterion-xyz".to_owned()];
    queue.put_item(&item).unwrap();

    let memory = Arc::new(open_memory());
    let terminal = tick(
        Arc::clone(&queue),
        Arc::clone(&memory),
        bin.path(),
        work.path(),
    );

    let Terminal::Success { output } = terminal else {
        panic!("the verdict is recorded, the run completes: {terminal:?}");
    };
    assert_eq!(output["status"], "parked", "{output}");
    let stored = queue.get_item("A1").unwrap().unwrap();
    assert_eq!(stored.status, ItemStatus::Parked);
    assert!(
        stored
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("false-criterion-xyz"),
        "{stored:?}"
    );
}

fn open_memory() -> MemStore {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-mem-").unwrap();
    MemStore::open(&dir.path().join("memory.redb")).unwrap()
}

/// Closing a failed item learns exactly one outcome document, and the document names the item and the criterion
/// that failed.
#[test]
fn learn_on_close_writes_one_outcome_document() {
    let bin = fake_pi();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-work-").unwrap();
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "queue-store-").unwrap();
    let queue = Arc::new(QueueStore::open(&dir.path().join("queue.redb")).unwrap());
    let mut item = WorkItem::new("A1", &[]);
    item.spec = "specs/208-the-queue-is-a-graph.md".to_owned();
    item.criteria = vec!["false-criterion-xyz".to_owned()];
    queue.put_item(&item).unwrap();
    let memory = Arc::new(open_memory());

    let _ = tick(
        Arc::clone(&queue),
        Arc::clone(&memory),
        bin.path(),
        work.path(),
    );

    assert_eq!(memory.n_docs().unwrap(), 1, "one outcome document");
    let id = memory.doc_ids().unwrap().remove(0);
    let text = String::from_utf8_lossy(&memory.doc(&id).unwrap().unwrap()).into_owned();
    assert!(text.contains("A1"), "{text}");
    assert!(text.contains("false-criterion-xyz"), "{text}");
}

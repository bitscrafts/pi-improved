//! Spec 391: the ledger is one kind, and it holds the capability.
//!
//! One `ledger` kind carries the four queue and heal writes behind an `op` field. It requires the `LEDGER` capability
//! bit, so a graph must be granted it; the four old kinds refuse at load and name their replacement; an unknown `op`
//! is refused at load, naming the node and the op.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_abi::{NodeInput, Outcome};
use pii_core::{Budget, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_graph::node::NodeFactory;
use pii_server::node_kinds::nodes::ledger::LedgerFactory;
use pii_store::queue::{Claim, QueueStore, WorkItem};

fn graphs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

/// The lint registry, which carries every kind the daemon does.
fn registry() -> (NodeRegistry, tempfile::TempDir, tempfile::TempDir) {
    let state = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    let reg = pii_server::lint_registry(state.path(), runtime.path());
    (reg, state, runtime)
}

/// The validator's errors for a one-node graph whose body is `node` and whose graph-level caps are `caps`.
fn refusals(caps: &str, node: &str) -> String {
    let src = format!(
        "name = \"ledger-fixture\"\nentry = \"select\"\nexit = \"select\"\ncaps = [{caps}]\n\n[[node]]\nid = \"select\"\n{node}\n"
    );
    let (reg, _s, _r) = registry();
    let spec = pii_graph::load_toml_with_registry(&src, &reg).expect("the fixture parses");
    match pii_graph::validate::validate(&spec, &reg) {
        Err(errors) => errors.join("\n"),
        Ok(_) => String::new(),
    }
}

#[test]
fn a_graph_without_the_bit_cannot_claim() {
    let msg = refusals("", "kind = \"ledger\"\nop = \"queue.claim\"");
    assert!(
        msg.contains("requires capability 'ledger'"),
        "the refusal names the capability: {msg}"
    );
}

#[test]
fn the_four_ops_are_the_whole_surface() {
    let msg = refusals("\"ledger\"", "kind = \"ledger\"\nop = \"queue.steal\"");
    assert!(msg.contains("'select'"), "names the node: {msg}");
    assert!(msg.contains("queue.steal"), "names the op: {msg}");
}

#[test]
fn an_old_kind_names_its_replacement() {
    let msg = refusals("\"ledger\"", "kind = \"select\"");
    assert!(msg.contains("ledger"), "names the new kind: {msg}");
    assert!(
        msg.contains("op = \"queue.claim\""),
        "names the new op: {msg}"
    );
}

/// The real `queue.toml` loads with the bit, and a claim through its `select` node takes the row once.
#[test]
fn CONTROL_a_graph_with_the_bit_claims_once() {
    let src = std::fs::read_to_string(graphs_dir().join("queue.toml")).unwrap();
    let (reg, _s, _r) = registry();
    let frags = tempfile::tempdir().unwrap();
    pii_graph::fragment::load_host_with_graphs(&src, frags.path(), &[graphs_dir()], &reg)
        .expect("queue.toml loads with the LEDGER bit granted");

    let dir = tempfile::tempdir().unwrap();
    let queue = Arc::new(QueueStore::open(&dir.path().join("queue.redb")).unwrap());
    queue.put_item(&WorkItem::new("A1", &[])).unwrap();
    let select = LedgerFactory {
        queue: Arc::clone(&queue),
        memory: None,
        heals: None,
    }
    .build(&serde_json::json!({ "op": "queue.claim" }))
    .unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    let first = select.run(&ctx, &input).unwrap();
    let Outcome::Next { port, value } = first else {
        panic!("the first claim routes on a port");
    };
    assert_eq!(port.as_str(), "ok", "the first claim takes the row");
    assert_eq!(value["id"], "A1", "got {value}");

    let second = select.run(&ctx, &input).unwrap();
    let Outcome::Next { port, .. } = second else {
        panic!("the second claim routes on a port");
    };
    assert_ne!(
        port.as_str(),
        "ok",
        "a second claim of the same row must not take it again"
    );
    assert!(matches!(
        queue.claim_next("another-run").unwrap(),
        Claim::Empty | Claim::AlreadyRunning(_) | Claim::Blocked(_)
    ));
}

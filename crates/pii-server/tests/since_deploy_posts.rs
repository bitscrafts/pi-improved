//! Spec 190: `since_deploy` counts the posts a deploy made from the store
//! (graph body + run journal), never from an in-memory counter or the log.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_abi::{NodeId, PortName, RunEvent, RunHeader, RunId, RunStatus};
use pii_server::daemon::since_deploy::since_deploy;
use pii_store::Store;

const POST_TOML: &str = "name = \"g\"\nentry = \"p\"\n[[node]]\nid = \"p\"\nkind = \"post\"\n";

/// The graph body as the engine stores it (`GraphSpec::to_json`).
fn post_graph() -> String {
    let spec = pii_graph::toml_load::load_toml(POST_TOML).unwrap();
    String::from_utf8(spec.to_json().unwrap()).unwrap()
}

#[derive(Clone, Copy)]
enum Outcome {
    Posted,
    Duplicate,
    Refused,
}

fn scratch() -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "posts-").unwrap()
}

/// Seeds a run at `revision` with the post node's outcome in its journal.
/// `graph` is the header's graph key; `None` stores no body at all.
fn seed(store: &Store, id: &str, revision: &str, graph: Option<&String>, out: Outcome) {
    let mut h = RunHeader {
        run: RunId::from(id),
        graph: graph.map_or_else(
            || "absent-hash".into(),
            |b| store.put_graph(b.as_bytes()).unwrap(),
        ),
        status: RunStatus::Succeeded,
        seq: 0,
        dur_ms: 0,
        revision: revision.into(),
        process_start_ms: 0,
        created_ms: 0,
    };
    let node = NodeId::from("p");
    let finish = |duplicate: bool| RunEvent::NodeFinished {
        node: NodeId::from("p"),
        port: PortName::from("ok"),
        value: serde_json::json!({"duplicate": duplicate}),
        value_hash: "v".into(),
        at_ms: 0,
        dur_ms: 0,
    };
    let last = match out {
        Outcome::Posted => finish(false),
        Outcome::Duplicate => finish(true),
        Outcome::Refused => RunEvent::NodeFailed {
            node: node.clone(),
            error: "refused".into(),
            retryable: false,
            attempt: 1,
        },
    };
    for ev in [
        RunEvent::Started {
            graph: "g".into(),
            input_hash: "h".into(),
        },
        RunEvent::NodeStarted { node, at_ms: 0 },
        last,
    ] {
        store.append_event(&mut h, &ev).unwrap();
    }
}

fn counts(store: &Store, rev: &str) -> (u64, u64, u64) {
    let s = since_deploy(store, rev).unwrap();
    (s.posted, s.post_refused, s.unreadable)
}

#[test]
fn posted_counts_a_successful_post_of_this_revision() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("s.redb")).unwrap();
    seed(&store, "r1", "abc123", Some(&post_graph()), Outcome::Posted);
    assert_eq!(counts(&store, "abc123"), (1, 0, 0));
}

#[test]
fn a_refused_post_is_counted_as_refused() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("s.redb")).unwrap();
    seed(
        &store,
        "r1",
        "abc123",
        Some(&post_graph()),
        Outcome::Refused,
    );
    assert_eq!(counts(&store, "abc123"), (0, 1, 0));
}

#[test]
fn a_duplicate_post_is_not_counted_twice() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("s.redb")).unwrap();
    seed(
        &store,
        "r1",
        "abc123",
        Some(&post_graph()),
        Outcome::Duplicate,
    );
    assert_eq!(counts(&store, "abc123"), (0, 0, 0));
}

#[test]
fn posts_of_another_revision_are_not_this_deploy() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("s.redb")).unwrap();
    seed(&store, "r1", "abc123", Some(&post_graph()), Outcome::Posted);
    seed(&store, "r2", "def456", Some(&post_graph()), Outcome::Posted);
    seed(&store, "r3", "", Some(&post_graph()), Outcome::Posted);
    assert_eq!(counts(&store, "abc123").0, 1);
    assert_eq!(counts(&store, "def456").0, 1);
}

#[test]
fn posts_survive_the_process() {
    let dir = scratch();
    let path = dir.path().join("s.redb");
    {
        let store = Store::open(&path).unwrap();
        seed(&store, "r1", "abc123", Some(&post_graph()), Outcome::Posted);
    }
    assert_eq!(counts(&Store::open(&path).unwrap(), "abc123"), (1, 0, 0));
}

#[test]
fn an_unreadable_run_is_counted_not_silently_dropped() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("s.redb")).unwrap();
    seed(&store, "r1", "abc123", None, Outcome::Posted);
    assert_eq!(counts(&store, "abc123"), (0, 0, 1));
}

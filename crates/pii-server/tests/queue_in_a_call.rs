//! Spec 343 R5/R6: the queue graph can be CALLED, and inside a call it reads its own nodes.
//!
//! `graphs/queue.toml` is written once and used twice: alone (a queue run) and inlined by a host (`night`, 342).
//! Inlining namespaces every id — `record` becomes `queue__record` — so a node that looks up its peer by the
//! literal `select` finds nothing and reports "no item was selected" even when an item was claimed. `peer_of`
//! resolves the peer in the node's own namespace, which is what this file proves: the same file loads both ways,
//! and the namespaced record is given the namespaced select.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_graph::fragment;
use pii_graph::node::NodeRegistry;

/// A host that calls the real `graphs/queue.toml`, the way the night will.
const HOST: &str = r#"
name = "night"
entry = "sdd"
exit = "sdd"
# a call may not grant more than the graph it sits in holds (graph.call.widen). The called graph's coder
# node declares fs_read and fs_write (spec 362: a coder without write cannot implement), so the host that
# holds the call must grant them too — this is what graphs/night.toml does.
caps = ["spawn", "proc", "fs_read", "fs_write"]

[[node]]
id = "sdd"
kind = "call"
graph = "queue"
sha256 = "PIN"
caps = ["spawn", "proc", "fs_read", "fs_write"]
in = { workdir = "$.input.workdir", base = "$.input.base" }
"#;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn queue_src() -> String {
    std::fs::read_to_string(repo().join("graphs/queue.toml")).expect("graphs/queue.toml")
}

#[test]
fn the_queue_graph_loads_as_a_called_graph() {
    // the premise of 342: `queue` is callable at all — it declares an exit, and its paths come from `input`
    let src = queue_src();
    let pin = fragment::sha256_hex(src.as_bytes());
    let host = HOST.replace("PIN", &pin);
    let graphs = repo().join("graphs");
    let woven = fragment::load_host_with_graphs(
        &host,
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&graphs),
        &NodeRegistry::builtin(),
    )
    .expect("a host may call graphs/queue.toml");
    let ids: Vec<&str> = woven.spec.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(
        ids.contains(&"sdd__record"),
        "the called graph's nodes are namespaced, got {ids:?}"
    );
    assert!(
        ids.contains(&"sdd__select"),
        "the peer this test is about must be there too: {ids:?}"
    );
}

#[test]
fn the_queue_graph_carries_no_absolute_project_path() {
    let src = queue_src();
    for path in ["/projects/", "/root/"] {
        assert!(
            !src.contains(path),
            "graphs/queue.toml may not hard-code {path}: its paths arrive through `input`"
        );
    }
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for
fn CONTROL_the_queue_graph_still_loads_alone() {
    // the same file, not called: names unnamespaced, which is what makes the peer lookup worth deriving
    let src = queue_src();
    let graphs = repo().join("graphs");
    let woven = fragment::load_host_with_graphs(
        &src,
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&graphs),
        &NodeRegistry::builtin(),
    )
    .expect("graphs/queue.toml loads standalone");
    let ids: Vec<&str> = woven.spec.nodes.iter().map(|n| n.id.as_str()).collect();
    // Spec 385 put the implementer behind a METHODOLOGY call, so the coder is namespaced one level
    // deeper. The property (the queue holds a claim, an implementer and a verdict writer) is unchanged.
    for id in ["select", "implement__method-judge__code", "record"] {
        assert!(ids.contains(&id), "{id} is missing: {ids:?}");
    }
}

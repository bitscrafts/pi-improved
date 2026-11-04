//! Spec 379: the books graph is a composition of `docs` with a stale inventory and a truth check.
//!
//!   * the one writing step is a `call` of `docs`, pinned to the digest of `graphs/docs.toml` on disk (R2);
//!   * it declares `exit = "done"` and has the `nothing-to-do` and `unverified` ports (R2);
//!   * it loads, so the call resolves (R2);
//!   * the call cannot widen the host's capabilities (R5).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use pii_core::CapKind;
use pii_graph::fragment;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn graph(name: &str) -> String {
    std::fs::read_to_string(repo().join(format!("graphs/{name}.toml")))
        .unwrap_or_else(|e| panic!("graphs/{name}.toml: {e}"))
}

fn table(src: &str) -> toml::Table {
    toml::from_str(src).expect("valid toml")
}

fn nodes(doc: &toml::Table) -> Vec<&toml::Table> {
    doc.get("node")
        .and_then(|n| n.as_array())
        .map(|nodes| nodes.iter().filter_map(toml::Value::as_table).collect())
        .unwrap_or_default()
}

fn node_ids(doc: &toml::Table) -> BTreeSet<String> {
    nodes(doc)
        .iter()
        .filter_map(|n| n.get("id").and_then(toml::Value::as_str).map(str::to_owned))
        .collect()
}

#[test]
fn books_graph_declares_its_exit_and_the_docs_call() {
    let src = graph("books");
    let doc = table(&src);
    assert_eq!(doc.get("exit").and_then(toml::Value::as_str), Some("done"));
    assert_eq!(
        src.matches("graph = \"docs\"").count(),
        1,
        "one `graph = \"docs\"` line"
    );
}

#[test]
fn books_graph_has_the_inventory_and_the_honest_middle() {
    let ids = node_ids(&table(&graph("books")));
    for want in [
        "inventory",
        "select",
        "nothing-to-do",
        "call-docs",
        "verify",
        "done",
        "unverified",
    ] {
        assert!(ids.contains(want), "books has no `{want}` node: {ids:?}");
    }
}

#[test]
fn books_graph_writes_through_one_pinned_call_of_docs() {
    let src = graph("books");
    let doc = table(&src);
    let calls: Vec<&toml::Table> = nodes(&doc)
        .into_iter()
        .filter(|n| n.get("kind").and_then(toml::Value::as_str) == Some("call"))
        .collect();
    assert_eq!(calls.len(), 1, "exactly one writing step: {calls:?}");
    let pin = calls[0]
        .get("sha256")
        .and_then(toml::Value::as_str)
        .expect("the call is pinned");
    assert_eq!(
        pin,
        fragment::sha256_hex(graph("docs").as_bytes()),
        "the pin must be the sha256 of graphs/docs.toml as it is on disk"
    );
}

#[test]
fn books_graph_loads_and_its_call_resolves() {
    let woven = fragment::load_host_with_graphs(
        &graph("books"),
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&repo().join("graphs")),
        &pii_graph::node::NodeRegistry::builtin(),
    )
    .expect("books loads");
    assert!(
        woven.calls.iter().any(|c| c.graph == "docs"),
        "the write step must inline graphs/docs.toml: {:?}",
        woven.calls
    );
}

/// R5: the host grants `fs_write`; a ceiling without it refuses the graph, naming the node.
#[test]
fn the_call_cannot_widen() {
    let reg = pii_graph::node::NodeRegistry::builtin()
        .with_grant_ceiling(CapKind::FS_READ | CapKind::PROC | CapKind::SPAWN);
    let err = fragment::load_host_with_graphs(
        &graph("books"),
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&repo().join("graphs")),
        &reg,
    )
    .expect_err("a node beyond the ceiling must not load");
    let msg = err.to_string();
    assert!(msg.contains("fs_write"), "names the capability: {msg}");
}

#[test]
fn books_graph_carries_no_absolute_project_paths() {
    let src = graph("books");
    for path in ["/projects/", "/root/"] {
        assert!(!src.contains(path), "books may not hard-code {path}");
    }
}

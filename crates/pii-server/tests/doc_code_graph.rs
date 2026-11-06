//! Spec 374: the documentation-of-the-code graph is a composition of `docs` with a tree inventory and a truth check.
//!
//! The checks here are the ones a reader of `graphs/doc-code.toml` cannot make by eye:
//!   * the one writing step is a `call` of `docs`, pinned to the digest of `graphs/docs.toml` on disk (R2) — a second
//!     coder node would re-implement the writer;
//!   * the graph loads, so the call resolves and every node kind it names exists (R1);
//!   * it declares `exit = "done"` so a night can call it (R5);
//!   * it carries no absolute project path, because paths arrive through `input`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::PathBuf;

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
fn doc_code_declares_its_exit_so_a_night_can_call_it() {
    let doc = table(&graph("doc-code"));
    assert_eq!(doc.get("exit").and_then(toml::Value::as_str), Some("done"));
}

#[test]
fn doc_code_has_the_four_steps_and_the_honest_middle() {
    let ids = node_ids(&table(&graph("doc-code")));
    for want in ["write", "done", "unverified"] {
        assert!(ids.contains(want), "doc-code has no `{want}` node: {ids:?}");
    }
}

/// Spec 393: the four steps live in `books`, and a run over a documented tree still writes nothing (spec 374 R4), because
/// `doc-drift-inventory` prints nothing for one and `books` routes an empty inventory to `nothing-to-do`.
#[test]
fn an_empty_inventory_ends_on_nothing_to_do() {
    assert!(
        graph("books").contains("{ eq = \"\", port = \"nothing-to-do\" }"),
        "books must route an empty inventory to nothing-to-do"
    );
}

#[test]
fn doc_code_writes_through_one_pinned_call_of_books() {
    let src = graph("doc-code");
    let doc = table(&src);
    let calls: Vec<&toml::Table> = nodes(&doc)
        .into_iter()
        .filter(|n| n.get("kind").and_then(toml::Value::as_str) == Some("call"))
        .collect();
    assert_eq!(calls.len(), 1, "exactly one writing step: {calls:?}");
    assert_eq!(
        src.matches("graph = \"books\"").count(),
        1,
        "one `graph = \"books\"` line"
    );
    let pin = calls[0]
        .get("sha256")
        .and_then(toml::Value::as_str)
        .expect("the call is pinned");
    assert_eq!(
        pin,
        fragment::sha256_hex(graph("books").as_bytes()),
        "the pin must be the sha256 of graphs/books.toml as it is on disk"
    );
}

#[test]
fn doc_code_loads_and_its_call_resolves() {
    let woven = fragment::load_host_with_graphs(
        &graph("doc-code"),
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&repo().join("graphs")),
        &pii_graph::node::NodeRegistry::builtin(),
    )
    .expect("doc-code loads");
    assert!(
        woven.calls.iter().any(|c| c.graph == "books"),
        "the write step must inline graphs/books.toml: {:?}",
        woven.calls
    );
}

#[test]
fn doc_code_verifies_with_the_doc_drift_tree_walk() {
    let src = graph("doc-code");
    assert!(
        src.contains("doc-drift"),
        "verify must run the doc-drift walk"
    );
    assert!(
        src.contains("nothing-to-do"),
        "the file must name the no-op exit it relies on"
    );
}

#[test]
fn doc_code_carries_no_absolute_project_paths() {
    let src = graph("doc-code");
    for path in ["/projects/", "/root/"] {
        assert!(
            !src.contains(path),
            "doc-code may not hard-code {path}: its paths arrive through `input`"
        );
    }
}

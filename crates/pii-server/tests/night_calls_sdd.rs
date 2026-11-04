//! Spec 373: the night's `sdd` node calls the graph named `sdd`, and `graphs/sdd.toml` is the SDD graph: it composes the
//! TDD graph (RED before the code) and the docs graph, and exits on `done` so a host can call it.
//!
//! The checks here are the ones a reader of the TOML cannot make by eye:
//!   * the night's `sdd` node calls a graph named `sdd`, pinned to the digest of `graphs/sdd.toml` on disk;
//!   * `sdd` exits on `done`, which is the only port a host can route on;
//!   * `sdd` calls `tdd` and `docs`, each at its current pin, and the loader's capability ladder accepts it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_graph::fragment;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn graph(name: &str) -> String {
    std::fs::read_to_string(repo().join(format!("graphs/{name}.toml")))
        .unwrap_or_else(|e| panic!("graphs/{name}.toml: {e}"))
}

/// The call records of a host once its called graphs are woven in (the loader refuses a call that widens its caps).
fn calls(src: &str) -> Vec<fragment::CallRecord> {
    fragment::load_host_with_graphs(
        src,
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&repo().join("graphs")),
        &pii_graph::node::NodeRegistry::builtin(),
    )
    .expect("the graph loads")
    .calls
}

#[test]
fn sdd_graph_file_exists() {
    assert!(
        repo().join("graphs/sdd.toml").is_file(),
        "graphs/sdd.toml must exist"
    );
}

#[test]
fn night_sdd_node_calls_the_sdd_graph_at_its_current_pin() {
    let night = graph("night");
    let sdd = graph("sdd");
    let pin = fragment::sha256_hex(sdd.as_bytes());
    let recorded = calls(&night);
    let call = recorded
        .iter()
        .find(|c| c.call == "sdd")
        .unwrap_or_else(|| panic!("no call record for `sdd`: {recorded:?}"));
    assert_eq!(
        call.graph, "sdd",
        "the night's `sdd` node must call the `sdd` graph"
    );
    assert_eq!(
        call.sha256, pin,
        "the pin must be the sha256 of graphs/sdd.toml as it is on disk"
    );
}

#[test]
fn sdd_exits_on_done() {
    let doc: toml::Table = toml::from_str(&graph("sdd")).expect("valid toml");
    assert_eq!(
        doc.get("exit").and_then(toml::Value::as_str),
        Some("done"),
        "sdd must declare exit = \"done\" so the night can call it"
    );
}

#[test]
fn sdd_calls_tdd_and_docs_at_their_pins() {
    let recorded = calls(&graph("sdd"));
    for graph_name in ["tdd", "docs"] {
        let call = recorded
            .iter()
            .find(|c| c.call == graph_name)
            .unwrap_or_else(|| panic!("sdd has no call `{graph_name}`: {recorded:?}"));
        assert_eq!(call.graph, graph_name);
        assert_eq!(
            call.sha256,
            fragment::sha256_hex(graph(graph_name).as_bytes()),
            "the `{graph_name}` pin must be the sha256 of graphs/{graph_name}.toml as it is on disk"
        );
    }
}

#[test]
fn both_refusals_of_sdd_reach_the_report() {
    // Spec 381 R3: a night that refused says so. Each refusal port of the `sdd` call leads into `review`, which passes
    // the verdict through to `report`, and the report names what `review` carried.
    let doc: toml::Table = toml::from_str(&graph("night")).expect("valid toml");
    let edges = doc["edge"].as_array().expect("night has edges");
    let has = |from: &str, port: &str, to: &str| {
        edges.iter().any(|e| {
            e.get("from").and_then(toml::Value::as_str) == Some(from)
                && e.get("port").and_then(toml::Value::as_str) == Some(port)
                && e.get("to").and_then(toml::Value::as_str) == Some(to)
        })
    };
    for port in ["ok", "unverified", "refused"] {
        assert!(has("sdd", port, "review"), "sdd.{port} must reach review");
    }
    assert!(has("review", "ok", "report"), "review must reach report");
    let review = doc["node"]
        .as_array()
        .and_then(|ns| {
            ns.iter()
                .find(|n| n.get("id").and_then(toml::Value::as_str) == Some("review"))
        })
        .expect("night has a review node");
    assert_eq!(
        review.get("join").and_then(toml::Value::as_str),
        Some("any"),
        "review takes one of three exclusive arms, so its join is any"
    );
    let report = doc["node"]
        .as_array()
        .and_then(|ns| {
            ns.iter()
                .find(|n| n.get("id").and_then(toml::Value::as_str) == Some("report"))
        })
        .expect("night has a report node");
    let template = report
        .get("template")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    assert!(
        template.contains("nodes.review"),
        "the report names what review carried, refusal or done: {template}"
    );
}

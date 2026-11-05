//! Spec 385 (R4, Q2): the night fills the queue when it is empty and drains it, and a fill failure never stops a drain.
//!
//! The checks here are the ones a reader of the TOML cannot make by eye, and they run through the loader, so a call that
//! widens its caps or names a stale pin is refused here as it would be at run time:
//!   * the night calls `queue` (drain) and `self-improve` (fill), each at its current pin;
//!   * the fill is gated on an empty queue: a drain that finds nothing routes to the fill, and only that route fills;
//!   * the fill's ok and refused ports both reach the second drain, so a fill that cannot plan still lets the drain finish;
//!   * the review takes one of its arms, and the report names what review carried.
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
    .expect("the night loads with its called graphs")
    .calls
}

fn table_of(name: &str) -> toml::Table {
    toml::from_str(&graph(name)).expect("valid toml")
}

fn nodes(doc: &toml::Table) -> &Vec<toml::Value> {
    doc["node"].as_array().expect("the graph has nodes")
}

fn node<'a>(doc: &'a toml::Table, id: &str) -> &'a toml::Table {
    nodes(doc)
        .iter()
        .filter_map(toml::Value::as_table)
        .find(|n| n.get("id").and_then(toml::Value::as_str) == Some(id))
        .unwrap_or_else(|| panic!("no node `{id}`"))
}

fn has_edge(doc: &toml::Table, from: &str, port: &str, to: &str) -> bool {
    doc.get("edge")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .any(|e| {
            e.get("from").and_then(toml::Value::as_str) == Some(from)
                && e.get("port").and_then(toml::Value::as_str) == Some(port)
                && e.get("to").and_then(toml::Value::as_str) == Some(to)
        })
}

#[test]
fn night_calls_queue_and_self_improve_at_their_pins() {
    let night = graph("night");
    let recorded = calls(&night);
    for graph_name in ["self-improve", "queue"] {
        let call = recorded
            .iter()
            .find(|c| c.graph == graph_name)
            .unwrap_or_else(|| panic!("the night calls no `{graph_name}`: {recorded:?}"));
        assert_eq!(
            call.sha256,
            fragment::sha256_hex(graph(graph_name).as_bytes()),
            "the `{graph_name}` pin must be the sha256 of graphs/{graph_name}.toml as it is on disk"
        );
    }
}

#[test]
fn the_fill_is_gated_on_an_empty_queue() {
    // Q2: fill runs when the queue is empty, and only then. The drain's record carries the status; only `empty` fills.
    let doc = table_of("night");
    let gate = node(&doc, "empty-gate");
    assert_eq!(
        gate.get("kind").and_then(toml::Value::as_str),
        Some("route")
    );
    assert_eq!(
        gate.get("on").and_then(toml::Value::as_str),
        Some("$.payload.status"),
        "the gate routes on the drain's status"
    );
    let rules = gate["rules"].as_array().expect("the gate has rules");
    assert!(
        rules.iter().any(|r| {
            r.get("eq").and_then(toml::Value::as_str) == Some("empty")
                && r.get("port").and_then(toml::Value::as_str) == Some("fill")
        }),
        "an empty queue routes to the fill: {rules:?}"
    );
    assert!(
        has_edge(&doc, "drain", "ok", "empty-gate"),
        "the drain's ok port reaches the gate"
    );
    assert!(
        has_edge(&doc, "empty-gate", "fill", "fill"),
        "the gate's fill port reaches the fill call"
    );
    assert!(
        has_edge(&doc, "empty-gate", "other", "review"),
        "a queue with work skips the fill and goes to review"
    );
}

#[test]
fn a_fill_failure_still_reaches_the_second_drain() {
    // Guardrail: a fill that cannot plan must not stop a drain. Both the fill's ok and its refusal reach the drain.
    let doc = table_of("night");
    // V6: a join over two ports of the SAME node can never be satisfied, so the fill's ok and err
    // merge first and the second drain has one arm. Asserting a direct edge here is asserting a
    // graph the engine refuses.
    assert!(has_edge(&doc, "fill", "ok", "merge-fill"), "fill.ok merges");
    assert!(
        has_edge(&doc, "merge-fill", "ok", "drain-again"),
        "the merge feeds the second drain"
    );
    assert!(
        has_edge(&doc, "fill", "err", "merge-fill"),
        "a refused fill must still drain"
    );
    assert!(has_edge(&doc, "drain-again", "ok", "review"));
}

#[test]
fn the_review_joins_its_arms_and_the_report_names_it() {
    let doc = table_of("night");
    assert_eq!(
        node(&doc, "review")
            .get("join")
            .and_then(toml::Value::as_str),
        Some("any"),
        "review takes one of its exclusive arms, so its join is any"
    );
    assert!(has_edge(&doc, "review", "ok", "report"));
    let template = node(&doc, "report")
        .get("template")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    assert!(
        template.contains("nodes.review"),
        "the report names what review carried: {template}"
    );
}

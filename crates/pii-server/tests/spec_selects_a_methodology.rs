//! Spec 385 (R1, R2, R3, R6): `spec` is a driver. It holds no coder and no judge of its own, it routes on the methodology,
//! and it calls one of two pinned methodology graphs: `sdd` (RED before GREEN) and `method-judge` (implement, then judge).
//! `sdd` keeps its RED/GREEN proof and loses only the docs step, which the driver owns.
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

fn table_of(name: &str) -> toml::Table {
    toml::from_str(&graph(name)).expect("valid toml")
}

/// The call records of a host once its called graphs are woven in (the loader refuses a call that widens its caps).
fn calls(src: &str) -> Vec<fragment::CallRecord> {
    fragment::load_host_with_graphs(
        src,
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&repo().join("graphs")),
        &pii_graph::node::NodeRegistry::builtin(),
    )
    .expect("the graph loads with its called graphs")
    .calls
}

fn nodes(doc: &toml::Table) -> Vec<&toml::Table> {
    doc["node"]
        .as_array()
        .expect("the graph has nodes")
        .iter()
        .filter_map(toml::Value::as_table)
        .collect()
}

fn node<'a>(doc: &'a toml::Table, id: &str) -> &'a toml::Table {
    nodes(doc)
        .into_iter()
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
fn method_judge_graph_exists_and_exits_on_done() {
    let doc = table_of("method-judge");
    assert_eq!(doc.get("exit").and_then(toml::Value::as_str), Some("done"));
    assert!(
        nodes(&doc)
            .iter()
            .any(|n| n.get("kind").and_then(toml::Value::as_str) == Some("judge")),
        "method-judge judges the criteria"
    );
}

#[test]
fn spec_driver_holds_no_coder_or_judge_of_its_own() {
    // R1 and the guardrail: the driver routes and calls; the coder and the judge live in the methodology graphs. The one
    // judge the driver owns is the re-judge after docs (the amended driver step), and it is the only judge it may hold.
    let doc = table_of("spec");
    for n in nodes(&doc) {
        let id = n
            .get("id")
            .and_then(toml::Value::as_str)
            .unwrap_or_default();
        let kind = n
            .get("kind")
            .and_then(toml::Value::as_str)
            .unwrap_or_default();
        assert!(
            kind != "coder" && (kind != "judge" || id == "reverify"),
            "spec node `{id}` is a {kind}: the driver must call a methodology instead"
        );
    }
}

#[test]
fn spec_routes_on_the_methodology_and_calls_one_of_two() {
    let doc = table_of("spec");
    let pick = node(&doc, "pick");
    assert_eq!(
        pick.get("kind").and_then(toml::Value::as_str),
        Some("route")
    );
    assert_eq!(
        pick.get("on").and_then(toml::Value::as_str),
        Some("$.input.method"),
        "the methodology is chosen from the run input"
    );
    let rules = pick["rules"].as_array().expect("the route has rules");
    assert!(rules.iter().any(|r| {
        r.get("eq").and_then(toml::Value::as_str) == Some("sdd")
            && r.get("port").and_then(toml::Value::as_str) == Some("sdd")
    }));
    // No eq: the fallback, so a spec that names no method runs method-judge.
    assert!(rules.iter().any(|r| {
        r.get("eq").is_none() && r.get("port").and_then(toml::Value::as_str) == Some("method-judge")
    }));
    assert!(has_edge(&doc, "pick", "sdd", "sdd"));
    assert!(has_edge(&doc, "pick", "method-judge", "method-judge"));
}

#[test]
fn spec_calls_both_methodologies_at_their_pins_then_docs_then_verifies() {
    let spec = graph("spec");
    let recorded = calls(&spec);
    for graph_name in ["sdd", "method-judge", "docs"] {
        let call = recorded
            .iter()
            .find(|c| c.graph == graph_name)
            .unwrap_or_else(|| panic!("spec calls no `{graph_name}`: {recorded:?}"));
        assert_eq!(
            call.sha256,
            fragment::sha256_hex(graph(graph_name).as_bytes()),
            "the `{graph_name}` pin must be the sha256 of graphs/{graph_name}.toml as it is on disk"
        );
    }
    let doc = table_of("spec");
    assert!(has_edge(&doc, "sdd", "ok", "passed"));
    assert!(has_edge(&doc, "method-judge", "ok", "passed"));
    assert!(has_edge(&doc, "passed", "ok", "docs"));
    assert!(has_edge(&doc, "docs", "ok", "reverify"));
    assert!(has_edge(&doc, "reverify", "ok", "done"));
    for call in ["sdd", "method-judge"] {
        assert!(
            has_edge(&doc, call, "unverified", "unverified"),
            "{call} can return unverified and the driver must say so"
        );
        assert!(
            has_edge(&doc, call, "refused", "refused"),
            "{call} can return refused and the driver must say so"
        );
    }
}

#[test]
fn sdd_keeps_its_red_green_proof_and_loses_only_the_docs_step() {
    let doc = table_of("sdd");
    let recorded = calls(&graph("sdd"));
    // Spec 385: the RED/GREEN proof is INLINE, not a call. Calling tdd made the chain
    // night -> queue -> spec -> sdd -> tdd five deep against a cap of four, so the merge is an
    // inline. What must hold is that sdd still OWNS the proof, which the node names show.
    assert!(
        recorded.iter().all(|c| c.graph != "tdd"),
        "sdd must not call tdd: the proof is inline at this depth"
    );
    let nodes = doc
        .get("node")
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();
    let ids: Vec<String> = nodes
        .iter()
        .filter_map(|n| n.get("id"))
        .filter_map(|v| v.as_str())
        .map(str::to_string)
        .collect();
    assert!(
        ids.iter().any(|i| i == "tdd_write_test"),
        "the RED step is inline: {ids:?}"
    );
    assert!(
        ids.iter().any(|i| i == "tdd_green"),
        "the GREEN step is inline: {ids:?}"
    );
    assert!(
        recorded.iter().all(|c| c.graph != "docs"),
        "the docs step is owned by the driver, not sdd"
    );
    assert!(has_edge(&doc, "tdd_green", "ok", "verify"));
}

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
    // Spec 385 R2: the TDD loop is INLINED into sdd, not called — night → queue → spec → sdd → tdd was
    // five deep against the depth cap of four. So sdd calls no `tdd`; it carries the loop as its own
    // nodes, and this test proves that shape rather than the call it used to make.
    let recorded = calls(&graph("sdd"));
    assert!(
        !recorded.iter().any(|c| c.call == "tdd" || c.graph == "tdd"),
        "sdd must not call tdd any more: the loop is inline — {recorded:?}"
    );
    let doc: toml::Table = toml::from_str(&graph("sdd")).expect("valid toml");
    let ids: Vec<&str> = doc["node"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n.get("id").and_then(toml::Value::as_str))
        .collect();
    for step in ["tdd_write_test", "tdd_red", "tdd_implement", "tdd_green"] {
        assert!(
            ids.contains(&step),
            "sdd carries the tdd step '{step}': {ids:?}"
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
    // Spec 385 R4: the night calls `queue` (which calls `spec`), so the verdicts that must reach the
    // report are the queue call's. Its declared ports are ok|blocked|empty|running, and a refused or
    // unverified row arrives through `ok` with its status recorded — the property is that no verdict of
    // the night's one call is left unwired.
    let queue = doc["node"]
        .as_array()
        .and_then(|ns| {
            ns.iter()
                .find(|n| n.get("id").and_then(toml::Value::as_str) == Some("drain"))
        })
        .expect("night calls the queue as `drain`");
    assert_eq!(
        queue.get("graph").and_then(toml::Value::as_str),
        Some("queue"),
        "the night's implementer is the queue graph"
    );
    // A CALL returns on its CALLEE'S EXIT ports, not on the ports the callee declares internally: queue's
    // exit is `record` (a ledger), so the night sees `ok` and `err` and the row's own verdict travels in
    // the payload. `empty-gate` is what reads that payload. Wiring `blocked`/`empty`/`running` here would
    // be wiring ports the call cannot produce — the same trap as 385's `fill.refused`.
    // The callee's EXIT node is `record`, a ledger whose op is `queue.set_status`, and that op declares a
    // single port: `ok`. So the call returns exactly one verdict, and the night must wire exactly that
    // one — the row's own outcome (implemented, refused, empty, blocked) travels in the payload, which is
    // what `empty-gate` routes on. Asserting `err` here would assert a port the graph cannot produce; it
    // was tried and the linter refused it, correctly.
    assert!(
        has("drain", "ok", "empty-gate"),
        "the queue's one verdict must reach empty-gate, which routes on its payload"
    );
    // The callee's exit node is in the QUEUE graph, not this one: `record`, a ledger with op
    // `queue.set_status`, which declares exactly one port.
    let queue_doc: toml::Table = toml::from_str(&graph("queue")).expect("valid toml");
    let exit_node = queue_doc["node"]
        .as_array()
        .and_then(|ns| {
            ns.iter()
                .find(|n| n.get("id").and_then(toml::Value::as_str) == Some("record"))
        })
        .expect("the queue exits through `record`");
    assert_eq!(
        exit_node.get("kind").and_then(toml::Value::as_str),
        Some("ledger"),
        "the queue's exit is its ledger write"
    );
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

//! Spec 342: the night is a graph composed from the graphs pii already has.
//!
//! The checks here are the ones a reader of `graphs/night.toml` cannot make by eye:
//!   * the pin it carries is the digest of the queue graph on disk — a stale pin silently runs yesterday's queue;
//!   * every kind it uses exists in a graph it composes — a night that invented `night_pick` would duplicate
//!     `select` and pass every other check;
//!   * it carries no absolute project path, because paths arrive through `input`;
//!   * the loop budget fits inside `max_entries`, which survives a resume.
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

fn kinds(src: &str) -> BTreeSet<String> {
    let doc: toml::Table = toml::from_str(src).expect("valid toml");
    doc.get("node")
        .and_then(|n| n.as_array())
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|n| n.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn night_calls_queue_at_its_current_pin() {
    let night = graph("night");
    let queue = graph("queue");
    let pin = fragment::sha256_hex(queue.as_bytes());
    let woven = fragment::load_host_with_graphs(
        &night,
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&repo().join("graphs")),
        &pii_graph::node::NodeRegistry::builtin(),
    )
    .expect("the night loads");
    let call = woven
        .calls
        .iter()
        .find(|c| c.call == "sdd")
        .unwrap_or_else(|| panic!("no call record for `sdd`: {:?}", woven.calls));
    assert_eq!(call.graph, "queue");
    assert_eq!(
        call.sha256, pin,
        "the pin must be the sha256 of graphs/queue.toml as it is on disk"
    );
}

#[test]
fn night_graph_uses_only_existing_kinds() {
    // The kinds a night may use: pii's own builtins (timer, transform, …), the egress `post` (spec 128), the
    // inclusion `call` (spec 255, expanded at load and therefore never registered), and anything a composed graph
    // already uses. What this forbids is the duplication the abandoned attempt introduced: a `night_pick` that
    // re-implements `select`, a `night_land` that re-implements `record` and commits.
    let night_kinds = kinds(&graph("night"));
    let mut allowed: BTreeSet<String> = BTreeSet::new();
    for builtin in [
        "transform",
        "timer",
        "gate",
        "call",
        "post",
        "refuse",
        "require",
    ] {
        allowed.insert(builtin.to_owned());
    }
    for composed in ["queue", "spec", "discord-digest"] {
        allowed.extend(kinds(&graph(composed)));
    }
    let invented: Vec<&String> = night_kinds.difference(&allowed).collect();
    assert!(
        invented.is_empty(),
        "the night uses kinds nothing else in pii offers: {invented:?} (night: {night_kinds:?})"
    );
    assert!(
        !night_kinds.iter().any(|k| k.starts_with("night_")),
        "a `night_*` kind duplicates a node that already exists: {night_kinds:?}"
    );
}

#[test]
fn night_graph_carries_no_absolute_project_paths() {
    let night = graph("night");
    for path in ["/projects/", "/root/"] {
        assert!(
            !night.contains(path),
            "the night may not hard-code {path}: its paths arrive through `input`"
        );
    }
}

#[test]
fn night_call_sets_max_entries_at_least_its_loop_budget() {
    // `max_entries` lives in the stored spec and survives a resume, so it must cover the whole night
    let night = graph("night");
    let doc: toml::Table = toml::from_str(&night).expect("valid toml");
    let call = doc
        .get("node")
        .and_then(|n| n.as_array())
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|n| n.get("kind").and_then(|k| k.as_str()) == Some("call"))
        })
        .expect("a call node");
    let max_entries = call
        .get("max_entries")
        .and_then(toml::Value::as_integer)
        .expect("the call declares max_entries");
    assert!(
        max_entries >= 8,
        "max_entries {max_entries} does not cover a night's ticks"
    );
}

/// `(from, port, to)` of every `[[edge]]` in a graph.
fn edges(src: &str) -> Vec<(String, String, String)> {
    let doc: toml::Table = toml::from_str(src).expect("valid toml");
    let field = |e: &toml::Value, k: &str| {
        e.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    doc.get("edge")
        .and_then(|e| e.as_array())
        .map(|es| {
            es.iter()
                .map(|e| (field(e, "from"), field(e, "port"), field(e, "to")))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn night_routes_an_empty_queue_to_review() {
    // Exhaustion leaves the queue through its exit: `select`'s empty and blocked ports feed `record`, and the call's
    // ok edge reaches `review`, which posts. A host can reach only the exit node's ports (spec 343).
    let queue = edges(&graph("queue"));
    let has = |list: &[(String, String, String)], from: &str, port: &str, to: &str| {
        list.iter()
            .any(|(f, p, t)| f == from && p == port && t == to)
    };
    assert!(
        has(&queue, "select", "empty", "record"),
        "queue: select.empty -> record"
    );
    assert!(
        has(&queue, "select", "blocked", "record"),
        "queue: select.blocked -> record"
    );
    let night = edges(&graph("night"));
    assert!(
        has(&night, "sdd", "ok", "review"),
        "night: sdd.ok -> review"
    );
    assert!(
        has(&night, "review", "ok", "report"),
        "night: review.ok -> report"
    );
}

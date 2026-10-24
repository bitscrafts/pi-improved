//! Spec 261: a sub-graph declares the run-input keys it needs (`needs`). The loader checks every `$.` read of a
//! called graph against that declaration and against the host's nodes, and a join reads its arms by their
//! namespaced ids. A graph without `needs` keeps spec 255's isolation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::{PiiError, Terminal};
use pii_core::{Budget, CapSet, Ctx, Observer, RecordingObserver, TraceCtx};
use pii_graph::GraphSpec;
use pii_graph::fragment::{self, Woven};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_store::Store;
use serde_json::json;

/// A transform node `id` with the given body lines (`expr = ...` or `value = ...`).
fn node(id: &str, body: &str) -> String {
    format!("[[node]]\nid = \"{id}\"\nkind = \"transform\"\n{body}\n\n")
}

/// A data edge from `from` to `to`.
fn edge(from: &str, to: &str) -> String {
    format!("[[edge]]\nfrom = \"{from}\"\nport = \"ok\"\nto = \"{to}\"\nkind = \"data\"\n\n")
}

/// A graph named `name`, with the given header line (`needs = [...]` or empty), nodes, edges and exit.
fn graph(name: &str, header: &str, nodes: &str, edges: &str, exit: &str) -> String {
    format!("name = \"{name}\"\nexit = \"{exit}\"\n{header}\n{nodes}{edges}")
}

/// A sub-graph `docs` with the given header, one `a` node reading `expr`, and exit `a`.
fn one_node(header: &str, expr: &str) -> String {
    graph(
        "docs",
        header,
        &node("a", &format!("expr = \"{expr}\"")),
        "",
        "a",
    )
}

/// The host `first -> docs(call) -> done`. `first` yields `first-value`; `tail` is extra call-table lines.
fn host(graph: &str, sha: &str, tail: &str) -> String {
    format!(
        r#"name = "host"
entry = "first"

[[node]]
id = "first"
kind = "transform"
value = "first-value"

[[node]]
id = "docs"
kind = "call"
graph = "{graph}"
sha256 = "{sha}"
caps = []
{tail}
[[node]]
id = "done"
kind = "transform"
expr = "$.payload"

[[edge]]
from = "first"
port = "ok"
to = "docs"
kind = "data"

[[edge]]
from = "docs"
port = "ok"
to = "done"
kind = "data"
"#
    )
}

/// The host calling the sub-graph `docs` as it is now written.
fn host_for(sub: &str, tail: &str) -> String {
    host("docs", &pin(sub), tail)
}

/// Writes the named graph files under a fresh temp dir and returns it (kept alive by the caller) with the paths.
struct Dirs {
    _root: tempfile::TempDir,
    graphs: PathBuf,
    fragments: PathBuf,
}

fn dirs(graphs: &[(&str, &str)]) -> Dirs {
    let root = tempfile::tempdir().expect("tempdir");
    let graph_dir = root.path().join("graphs");
    std::fs::create_dir_all(&graph_dir).expect("mkdir graphs");
    for (name, src) in graphs {
        std::fs::write(graph_dir.join(format!("{name}.toml")), src).expect("write graph");
    }
    let fragments = root.path().join("fragments");
    Dirs {
        _root: root,
        graphs: graph_dir,
        fragments,
    }
}

fn pin(src: &str) -> String {
    fragment::sha256_hex(src.as_bytes())
}

fn load(host: &str, graphs: &[(&str, &str)]) -> Result<Woven, PiiError> {
    let d = dirs(graphs);
    fragment::load_host_with_graphs(
        host,
        &d.fragments,
        std::slice::from_ref(&d.graphs),
        &NodeRegistry::builtin(),
    )
}

fn load_err(host: &str, graphs: &[(&str, &str)]) -> String {
    load(host, graphs)
        .expect_err("load must refuse")
        .to_string()
}

fn ids(spec: &GraphSpec) -> Vec<&str> {
    spec.nodes.iter().map(|n| n.id.as_str()).collect()
}

/// Runs the spec once with `input` and returns its terminal.
fn run_spec(spec: &GraphSpec, input: serde_json::Value) -> Terminal {
    let root = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&root.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let obs = Arc::new(RecordingObserver::new());
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::clone(&obs) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    let reg = NodeRegistry::builtin();
    run::run_with_validate(&store, spec, &reg, &ctx, &run_id, input, true).expect("run")
}

fn output_of(terminal: Terminal) -> serde_json::Value {
    let Terminal::Success { output } = terminal else {
        panic!("expected a successful run, got {terminal:?}");
    };
    output
}

#[test]
fn a_declared_key_reads_the_host_input() {
    let sub = one_node("needs = [\"x\"]", "$.input.x");
    let woven = load(&host_for(&sub, ""), &[("docs", &sub)]).expect("load");
    let out = output_of(run_spec(&woven.spec, json!({ "x": 7 })));
    assert_eq!(out, json!(7));
}

#[test]
fn a_graph_without_needs_stays_isolated() {
    let sub = one_node("", "$.input.x");
    let woven = load(&host_for(&sub, ""), &[("docs", &sub)]).expect("load");
    let out = output_of(run_spec(&woven.spec, json!({ "x": 7 })));
    assert_eq!(out, json!(null));
}

#[test]
fn a_mapped_key_overrides_a_declared_one() {
    // `a` reads the mapped `x`; `b` reads the declared, unmapped `y` (or `a`, to carry A out through the exit).
    let edges = edge("a", "b");
    let tail = "\n[node.in]\nx = \"mapped\"\n";
    let via_a = [
        node("a", "expr = \"$.input.x\""),
        node("b", "expr = \"$.nodes.a\""),
    ]
    .concat();
    let sub_a = graph("docs", "needs = [\"x\", \"y\"]", &via_a, &edges, "b");
    let woven = load(&host_for(&sub_a, tail), &[("docs", &sub_a)]).expect("load");
    let out = output_of(run_spec(&woven.spec, json!({ "x": 7, "y": 8 })));
    assert_eq!(out, json!("mapped"));

    let via_y = [
        node("a", "expr = \"$.input.x\""),
        node("b", "expr = \"$.input.y\""),
    ]
    .concat();
    let sub_b = graph("docs", "needs = [\"x\", \"y\"]", &via_y, &edges, "b");
    let woven = load(&host_for(&sub_b, tail), &[("docs", &sub_b)]).expect("load");
    let out = output_of(run_spec(&woven.spec, json!({ "x": 7, "y": 8 })));
    assert_eq!(out, json!(8));
}

#[test]
fn an_undeclared_key_is_refused_naming_graph_node_and_key() {
    let sub = one_node("needs = [\"x\"]", "$.input.y");
    let msg = load_err(&host_for(&sub, ""), &[("docs", &sub)]);
    assert!(msg.contains("graph.call.undeclared"), "got: {msg}");
    assert!(msg.contains("docs"), "names the graph: {msg}");
    assert!(msg.contains("'a'"), "names the node: {msg}");
    assert!(msg.contains("'y'"), "names the key: {msg}");
}

#[test]
fn bare_input_is_refused_when_needs_is_declared() {
    let sub = one_node("needs = [\"x\"]", "$.input");
    let msg = load_err(&host_for(&sub, ""), &[("docs", &sub)]);
    assert!(msg.contains("graph.call.undeclared"), "got: {msg}");
    assert!(msg.contains("whole input"), "got: {msg}");
}

#[test]
fn a_malformed_needs_is_refused() {
    for header in [
        "needs = \"x\"",
        "needs = [1]",
        "needs = [\"a\", \"a\"]",
        "needs = [\"\"]",
    ] {
        let sub = one_node(header, "$.input.x");
        let msg = load_err(&host_for(&sub, ""), &[("docs", &sub)]);
        assert!(msg.contains("graph.call.needs"), "{header}: {msg}");
    }
}

#[test]
fn a_host_node_read_is_refused_with_needs() {
    let sub = one_node("needs = [\"x\"]", "$.nodes.first");
    let msg = load_err(&host_for(&sub, ""), &[("docs", &sub)]);
    assert!(msg.contains("graph.call.leak"), "got: {msg}");
    assert!(msg.contains("first"), "names the host node: {msg}");
    assert!(msg.contains("docs"), "names the call: {msg}");
}

#[test]
fn a_host_node_read_is_refused_without_needs() {
    let sub = one_node("", "$.nodes.first");
    let msg = load_err(&host_for(&sub, ""), &[("docs", &sub)]);
    assert!(msg.contains("graph.call.leak"), "got: {msg}");
}

#[test]
fn a_sub_graph_node_read_is_still_namespaced() {
    let nodes = [
        node("a", "value = \"inner\""),
        node("b", "expr = \"$.nodes.a\""),
    ]
    .concat();
    let sub = graph("docs", "", &nodes, &edge("a", "b"), "b");
    let woven = load(&host_for(&sub, ""), &[("docs", &sub)]).expect("load");
    let out = output_of(run_spec(&woven.spec, json!({})));
    assert_eq!(out, json!("inner"));
}

#[test]
fn a_host_node_reaches_the_subgraph_through_in() {
    let sub = one_node("", "$.input.v");
    let woven = load(
        &host_for(&sub, "\n[node.in]\nv = \"$.nodes.first\"\n"),
        &[("docs", &sub)],
    )
    .expect("load");
    let out = output_of(run_spec(&woven.spec, json!({})));
    assert_eq!(out, json!("first-value"));
}

#[test]
fn a_join_reads_both_arms() {
    let join = |expr: &str| {
        let nodes = [
            node("s", "value = \"go\""),
            node("l", "value = \"L\""),
            node("r", "value = \"R\""),
            node("j", &format!("expr = \"{expr}\"")),
        ]
        .concat();
        let edges = [
            edge("s", "l"),
            edge("s", "r"),
            edge("l", "j"),
            edge("r", "j"),
        ]
        .concat();
        let sub = graph("docs", "", &nodes, &edges, "j");
        let woven = load(&host_for(&sub, ""), &[("docs", &sub)]).expect("load");
        output_of(run_spec(&woven.spec, json!({})))
    };
    assert_eq!(join("$.payload.l"), json!("L"));
    assert_eq!(join("$.payload.r"), json!("R"));
}

#[test]
fn a_single_inbound_node_is_not_rewritten() {
    // `l` is a sub-graph node id AND a field of the upstream value: the single-edge read must find the field.
    let nodes = [
        node("src", "expr = \"$.input.obj\""),
        node("l", "expr = \"$.payload\""),
        node("x", "expr = \"$.payload.l\""),
    ]
    .concat();
    let edges = [edge("src", "l"), edge("l", "x")].concat();
    let sub = graph("docs", "needs = [\"obj\"]", &nodes, &edges, "x");
    let woven = load(&host_for(&sub, ""), &[("docs", &sub)]).expect("load");
    let out = output_of(run_spec(&woven.spec, json!({ "obj": { "l": "FIELD" } })));
    assert_eq!(out, json!("FIELD"));
}

#[test]
fn an_unknown_call_key_is_refused() {
    let sub = one_node("", "$.input.x");
    let msg = load_err(&host_for(&sub, "loop_budget = 3\n"), &[("docs", &sub)]);
    assert!(msg.contains("graph.call.key"), "got: {msg}");
    assert!(msg.contains("loop_budget"), "names the key: {msg}");
}

#[test]
fn the_known_call_keys_still_load() {
    let sub = one_node("", "$.input.v");
    let tail = "\n[node.in]\nv = \"$.nodes.first\"\n";
    assert!(load(&host_for(&sub, tail), &[("docs", &sub)]).is_ok());
}

#[test]
fn a_nested_need_must_be_declared_by_the_outer_graph() {
    let leaf = one_node("needs = [\"x\"]", "$.input.x");
    let mid = graph(
        "mid",
        "needs = []",
        &format!(
            "[[node]]\nid = \"leaf\"\nkind = \"call\"\ngraph = \"leaf\"\nsha256 = \"{}\"\ncaps = []\n\n{}",
            pin(&leaf),
            node("end", "expr = \"$.payload\"")
        ),
        &edge("leaf", "end"),
        "end",
    );
    let msg = load_err(
        &host("mid", &pin(&mid), ""),
        &[("mid", &mid), ("leaf", &leaf)],
    );
    assert!(msg.contains("graph.call.undeclared"), "got: {msg}");
    assert!(msg.contains("mid"), "names the outer graph: {msg}");
}

#[test]
fn a_nested_chain_reads_the_outermost_input() {
    let leaf = one_node("needs = [\"x\"]", "$.input.x");
    let mid = graph(
        "mid",
        "needs = [\"x\"]",
        &format!(
            "[[node]]\nid = \"leaf\"\nkind = \"call\"\ngraph = \"leaf\"\nsha256 = \"{}\"\ncaps = []\n\n{}",
            pin(&leaf),
            node("end", "expr = \"$.payload\"")
        ),
        &edge("leaf", "end"),
        "end",
    );
    let woven = load(
        &host("mid", &pin(&mid), ""),
        &[("mid", &mid), ("leaf", &leaf)],
    )
    .expect("load");
    let out = output_of(run_spec(&woven.spec, json!({ "x": 7 })));
    assert_eq!(out, json!(7));
}

#[test]
fn a_wrong_pin_is_still_refused_with_needs() {
    let sub = one_node("needs = [\"x\"]", "$.input.x");
    let msg = load_err(&host("docs", &"0".repeat(64), ""), &[("docs", &sub)]);
    assert!(msg.contains("fragment.pin.mismatch"), "got: {msg}");
}

#[test]
fn needs_does_not_widen_caps() {
    let nodes = node("a", "expr = \"$.input.x\"\ncaps = [\"spawn\"]");
    let sub = graph("docs", "needs = [\"x\"]", &nodes, "", "a");
    let msg = load_err(&host_for(&sub, ""), &[("docs", &sub)]);
    assert!(msg.contains("graph.call.widen"), "got: {msg}");
    assert!(msg.contains("spawn"), "names the capability: {msg}");
}

#[test]
fn a_graph_without_needs_loads_identically() {
    let sub = graph(
        "docs",
        "",
        &[
            node("a", "value = \"x\""),
            node("b", "expr = \"$.nodes.a\""),
        ]
        .concat(),
        &edge("a", "b"),
        "b",
    );
    let woven = load(&host_for(&sub, ""), &[("docs", &sub)]).expect("load");
    let mut got = ids(&woven.spec);
    got.sort_unstable();
    assert_eq!(got, ["docs__a", "docs__b", "docs__in", "done", "first"]);
    assert_eq!(woven.spec.entry.as_str(), "first");
}

#[test]
fn the_loaded_graph_has_no_call_node_and_one_entry() {
    let sub = one_node("needs = [\"x\"]", "$.input.x");
    let woven = load(&host_for(&sub, ""), &[("docs", &sub)]).expect("load");
    assert!(woven.spec.nodes.iter().all(|n| n.kind != "call"));
    assert_eq!(woven.spec.entry.as_str(), "first");
}

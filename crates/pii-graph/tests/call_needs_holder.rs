//! Spec 264: the holder check of `needs`. A sub-graph `leaf` that declares `needs` and is called with no mapping for
//! a key reads its HOLDER's input, so the holder must declare that key or map it. The outer call's own check does not
//! cover this; these tests pin the holder check on its own.

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

/// A graph `name` with the given header that calls `callee` (pinned to `callee_src`) with the `in` table lines
/// `in_table` (empty for no mapping), then passes the call's output to `end`, which is the exit.
fn caller(name: &str, header: &str, callee: &str, callee_src: &str, in_table: &str) -> String {
    let call = format!(
        "[[node]]\nid = \"{callee}\"\nkind = \"call\"\ngraph = \"{callee}\"\nsha256 = \"{}\"\ncaps = []\n\n{in_table}",
        pin(callee_src)
    );
    graph(
        name,
        header,
        &format!("{call}{}", node("end", "expr = \"$.payload\"")),
        &edge(callee, "end"),
        "end",
    )
}

/// The host `first -> docs(call) -> done`, calling the graph `graph` pinned to `sha`.
fn host(graph: &str, sha: &str) -> String {
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

/// Runs the spec once with `input` and returns its output; the run must succeed.
fn run_output(spec: &pii_graph::GraphSpec, input: serde_json::Value) -> serde_json::Value {
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
    let terminal =
        run::run_with_validate(&store, spec, &reg, &ctx, &run_id, input, true).expect("run");
    let Terminal::Success { output } = terminal else {
        panic!("expected a successful run, got {terminal:?}");
    };
    output
}

#[test]
fn a_holder_without_needs_refuses_a_nested_declared_key() {
    // `mid` has no `needs` at all, and its call to `leaf` maps nothing: `leaf` would read `mid`'s input.
    let leaf = one_node("needs = [\"x\"]", "$.input.x");
    let mid = caller("mid", "", "leaf", &leaf, "");
    let msg = load_err(&host("mid", &pin(&mid)), &[("mid", &mid), ("leaf", &leaf)]);
    assert!(msg.contains("graph.call.undeclared"), "got: {msg}");
    assert!(msg.contains("mid"), "names the holder: {msg}");
    assert!(msg.contains("leaf"), "names the called graph: {msg}");
    assert!(msg.contains('x'), "names the key: {msg}");
}

#[test]
fn a_holder_that_maps_the_key_loads_and_reads_it() {
    // CONTROL: the same shape with the key mapped, so the refusal above is about the missing declaration.
    let leaf = one_node("needs = [\"x\"]", "$.input.x");
    let mid = caller("mid", "", "leaf", &leaf, "[node.in]\nx = \"mapped\"\n\n");
    let woven = load(&host("mid", &pin(&mid)), &[("mid", &mid), ("leaf", &leaf)]).expect("load");
    assert_eq!(run_output(&woven.spec, json!({})), json!("mapped"));
}

#[test]
fn a_holder_that_declares_the_key_loads_and_reads_the_host_input() {
    // CONTROL: the holder declares `x`, so `leaf` reads the host's `x`.
    let leaf = one_node("needs = [\"x\"]", "$.input.x");
    let mid = caller("mid", "needs = [\"x\"]", "leaf", &leaf, "");
    let woven = load(&host("mid", &pin(&mid)), &[("mid", &mid), ("leaf", &leaf)]).expect("load");
    assert_eq!(run_output(&woven.spec, json!({ "x": 7 })), json!(7));
}

#[test]
fn the_error_is_not_the_outer_checks() {
    // The outer call's own check words its refusal differently; only the holder check says "does not declare".
    let leaf = one_node("needs = [\"x\"]", "$.input.x");
    let mid = caller("mid", "", "leaf", &leaf, "");
    let msg = load_err(&host("mid", &pin(&mid)), &[("mid", &mid), ("leaf", &leaf)]);
    assert!(
        msg.contains("which the holder does not declare"),
        "got: {msg}"
    );
}

#[test]
fn three_levels_need_every_holder_to_declare() {
    // host -> a (declares x) -> b (no needs) -> leaf (needs x): the middle holder `b` is the one that refuses.
    let leaf = one_node("needs = [\"x\"]", "$.input.x");
    let b = caller("b", "", "leaf", &leaf, "");
    let a = caller("a", "needs = [\"x\"]", "b", &b, "");
    let msg = load_err(
        &host("a", &pin(&a)),
        &[("a", &a), ("b", &b), ("leaf", &leaf)],
    );
    assert!(msg.contains("graph.call.undeclared"), "got: {msg}");
    assert!(msg.contains("'b'"), "names the middle holder: {msg}");
}

//! Spec 255: a `call` node inlines another graph at load time. The sub-graph's ids are namespaced, its input is
//! mapped at the call site, its exit routes back to the host, its pin and caps and recursion are checked, and a
//! graph that uses no call loads exactly as before.

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

/// A sub-graph whose first node reads the mapped `title`, and whose exit `b` reads `a`.
const SUB: &str = r#"
name = "docs"
exit = "b"

[[node]]
id = "a"
kind = "transform"
expr = "$.input.title"

[[node]]
id = "b"
kind = "transform"
expr = "$.nodes.a"

[[edge]]
from = "a"
port = "ok"
to = "b"
kind = "data"
"#;

/// The host: `start -> docs(call, maps title from input x) -> done`. `PIN` is the sub-graph's real digest.
const HOST: &str = r#"
name = "host"
entry = "start"

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "docs"
kind = "call"
graph = "docs"
sha256 = "PIN"
caps = []

[node.in]
title = "$.input.x"

[[node]]
id = "done"
kind = "transform"
expr = "$.payload"

[[edge]]
from = "start"
port = "ok"
to = "docs"
kind = "data"

[[edge]]
from = "docs"
port = "ok"
to = "done"
kind = "data"
"#;

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

/// The host with its `PIN` placeholder replaced by the real digest of `sub`.
fn host_for(sub: &str) -> String {
    HOST.replace("PIN", &pin(sub))
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
fn a_call_node_inlines_the_subgraph() {
    let woven = load(&host_for(SUB), &[("docs", SUB)]).expect("load");
    let ids = ids(&woven.spec);
    for want in ["docs__in", "docs__a", "docs__b"] {
        assert!(ids.contains(&want), "missing {want}: {ids:?}");
    }
    assert!(!ids.contains(&"docs"), "the call node is gone: {ids:?}");
    assert_eq!(woven.spec.entry.as_str(), "start");
}

#[test]
fn the_subgraph_reads_the_mapped_input_not_the_hosts() {
    let woven = load(&host_for(SUB), &[("docs", SUB)]).expect("load");
    let out = output_of(run_spec(&woven.spec, serde_json::json!({ "x": 1 })));
    assert_eq!(out, serde_json::json!(1));
}

#[test]
fn a_host_run_input_does_not_leak_into_the_subgraph() {
    // The sub reads `$.input.x`, but the call maps no `x`: the sub must see null, never the host's x.
    let sub = SUB.replace("$.input.title", "$.input.x");
    let host = host_for(&sub).replace("[node.in]\ntitle = \"$.input.x\"\n", "");
    let woven = load(&host, &[("docs", &sub)]).expect("load");
    let out = output_of(run_spec(&woven.spec, serde_json::json!({ "x": 1 })));
    assert_eq!(out, serde_json::Value::Null);
}

#[test]
fn the_exit_port_routes_back_to_the_host() {
    let sub = SUB.replace("expr = \"$.nodes.a\"", "value = \"from-sub\"");
    let woven = load(&host_for(&sub), &[("docs", &sub)]).expect("load");
    let out = output_of(run_spec(&woven.spec, serde_json::json!({ "x": 1 })));
    assert_eq!(out, serde_json::json!("from-sub"));
}

#[test]
fn a_wrong_pin_is_refused_with_both_digests() {
    let expected = pin(SUB);
    let mut wrong = expected.clone();
    let flipped = if wrong.starts_with('0') { '1' } else { '0' };
    wrong.replace_range(0..1, &flipped.to_string());
    let host = HOST.replace("PIN", &wrong);
    let msg = load_err(&host, &[("docs", SUB)]);
    assert!(msg.contains(&wrong), "expected digest missing: {msg}");
    assert!(msg.contains(&expected), "actual digest missing: {msg}");
}

#[test]
fn a_correct_pin_loads() {
    assert!(load(&host_for(SUB), &[("docs", SUB)]).is_ok());
}

#[test]
fn a_subgraph_cannot_widen_caps() {
    let sub = SUB.replace(
        "expr = \"$.input.title\"",
        "expr = \"$.input.title\"\ncaps = [\"spawn\"]",
    );
    let host = host_for(&sub).replace("caps = []", "caps = [\"proc\"]");
    let host = host.replace("name = \"host\"", "name = \"host\"\ncaps = [\"proc\"]");
    let msg = load_err(&host, &[("docs", &sub)]);
    assert!(msg.contains("spawn"), "names the capability: {msg}");
    assert!(msg.contains("'a'"), "names the node: {msg}");
}

#[test]
fn a_subgraph_within_the_grant_loads() {
    let sub = SUB.replace(
        "expr = \"$.input.title\"",
        "expr = \"$.input.title\"\ncaps = [\"spawn\"]",
    );
    let host = host_for(&sub).replace("caps = []", "caps = [\"spawn\"]");
    let host = host.replace("name = \"host\"", "name = \"host\"\ncaps = [\"spawn\"]");
    assert!(load(&host, &[("docs", &sub)]).is_ok());
}

#[test]
fn a_graph_that_calls_itself_is_refused() {
    let host = HOST
        .replace("name = \"host\"", "name = \"a\"")
        .replace("graph = \"docs\"", "graph = \"a\"")
        .replace("PIN", "0");
    let msg = load_err(&host, &[]);
    assert!(msg.contains("a -> a"), "chain missing: {msg}");
}

#[test]
fn an_indirect_cycle_is_refused_with_the_chain() {
    // host `a` calls `b`; `b` calls `a`.
    let b = "name = \"b\"\nexit = \"d\"\n\n[[node]]\nid = \"c\"\nkind = \"call\"\ngraph = \"a\"\nsha256 = \"0\"\ncaps = []\n\n[[node]]\nid = \"d\"\nkind = \"transform\"\nvalue = \"x\"\n\n[[edge]]\nfrom = \"c\"\nport = \"ok\"\nto = \"d\"\nkind = \"data\"\n";
    let host = HOST
        .replace("name = \"host\"", "name = \"a\"")
        .replace("graph = \"docs\"", "graph = \"b\"")
        .replace("PIN", &pin(b));
    let msg = load_err(&host, &[("b", b)]);
    assert!(msg.contains("a -> b -> a"), "chain missing: {msg}");
}

/// A chain of `n` graphs, `g1 -> g2 -> ... -> gn`, each a call to the next and the last a leaf. Returns the host
/// source (calling `g1`) and the graph files.
fn chain(n: usize) -> (String, Vec<(String, String)>) {
    let mut files: Vec<(String, String)> = Vec::new();
    let mut next: Option<(String, String)> = None;
    for i in (1..=n).rev() {
        let name = format!("g{i}");
        let src = match &next {
            None => format!(
                "name = \"{name}\"\nexit = \"leaf\"\n\n[[node]]\nid = \"leaf\"\nkind = \"transform\"\nvalue = \"deep\"\n"
            ),
            Some((child, child_src)) => format!(
                "name = \"{name}\"\nexit = \"tail\"\n\n[[node]]\nid = \"n\"\nkind = \"call\"\ngraph = \"{child}\"\nsha256 = \"{}\"\ncaps = []\n\n[[node]]\nid = \"tail\"\nkind = \"transform\"\nvalue = \"t\"\n\n[[edge]]\nfrom = \"n\"\nport = \"ok\"\nto = \"tail\"\nkind = \"data\"\n",
                pin(child_src)
            ),
        };
        files.push((name.clone(), src.clone()));
        next = Some((name, src));
    }
    let (first, first_src) = next.expect("chain has a graph");
    let host = HOST
        .replace("graph = \"docs\"", &format!("graph = \"{first}\""))
        .replace("PIN", &pin(&first_src));
    (host, files)
}

#[test]
fn depth_five_is_refused_and_depth_four_loads() {
    let (host4, files4) = chain(4);
    let refs4: Vec<(&str, &str)> = files4
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    assert!(load(&host4, &refs4).is_ok(), "a chain of four loads");

    let (host5, files5) = chain(5);
    let refs5: Vec<(&str, &str)> = files5
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let msg = load_err(&host5, &refs5);
    assert!(msg.contains("depth"), "names the depth cap: {msg}");
    assert!(msg.contains('4'), "names the cap value: {msg}");
}

#[test]
fn a_subgraph_without_an_exit_is_refused() {
    let sub = SUB.replace("exit = \"b\"\n", "");
    let msg = load_err(&host_for(&sub), &[("docs", &sub)]);
    assert!(msg.contains("exit"), "names exit: {msg}");
}

#[test]
fn a_call_without_graph_is_refused() {
    let host = HOST.replace("graph = \"docs\"\n", "");
    let msg = load_err(&host, &[("docs", SUB)]);
    assert!(msg.contains("'graph'"), "names the missing key: {msg}");
}

#[test]
fn a_call_without_sha256_is_refused() {
    let host = HOST.replace("sha256 = \"PIN\"\n", "");
    let msg = load_err(&host, &[("docs", SUB)]);
    assert!(msg.contains("'sha256'"), "names the missing key: {msg}");
}

#[test]
fn a_call_without_caps_is_refused() {
    let host = HOST.replace("caps = []\n", "");
    let msg = load_err(&host, &[("docs", SUB)]);
    assert!(msg.contains("'caps'"), "names the missing key: {msg}");
}

#[test]
fn a_call_in_a_fragment_is_refused() {
    let fragment_src = "name = \"f\"\n\n[fragment]\nname = \"f\"\ninputs = [\"$.input.x\"]\noutputs = [\"ok\"]\nmay_block = false\n\n[[node]]\nid = \"d\"\nkind = \"call\"\ngraph = \"docs\"\nsha256 = \"0\"\ncaps = []\n";
    let msg = fragment::check_fragment(fragment_src, &NodeRegistry::builtin())
        .expect_err("a fragment may not call")
        .to_string();
    assert!(msg.contains("fragment.nesting"), "got: {msg}");
}

#[test]
fn the_old_load_host_refuses_a_call_with_no_directory() {
    let d = dirs(&[]);
    let msg = fragment::load_host(&host_for(SUB), &d.fragments, &NodeRegistry::builtin())
        .expect_err("no dirs, so no call")
        .to_string();
    assert!(msg.contains("no graph directory"), "got: {msg}");
}

#[test]
fn an_id_collision_is_refused() {
    let host = HOST.replace(
        "[[edge]]\nfrom = \"start\"",
        "[[node]]\nid = \"docs__a\"\nkind = \"transform\"\nvalue = \"taken\"\n\n[[edge]]\nfrom = \"start\"",
    );
    assert!(
        host.contains("id = \"docs__a\""),
        "fixture inserts the node"
    );
    let msg = load_err(&host.replace("PIN", &pin(SUB)), &[("docs", SUB)]);
    assert!(msg.contains("docs__a"), "names the colliding id: {msg}");
    assert!(msg.contains("docs"), "names the call: {msg}");
}

#[test]
fn a_graph_with_no_call_loads_byte_identically() {
    let plain = "name = \"plain\"\n\n[[node]]\nid = \"start\"\nkind = \"transform\"\nvalue = \"go\"\n\n[[node]]\nid = \"end\"\nkind = \"transform\"\nexpr = \"$.payload\"\n\n[[edge]]\nfrom = \"start\"\nport = \"ok\"\nto = \"end\"\nkind = \"data\"\n";
    let d = dirs(&[]);
    let reg = NodeRegistry::builtin();
    let with_dirs =
        fragment::load_host_with_graphs(plain, &d.fragments, std::slice::from_ref(&d.graphs), &reg)
            .expect("with dirs");
    let without = fragment::load_host(plain, &d.fragments, &reg).expect("without dirs");
    assert_eq!(with_dirs.spec, without.spec);
}

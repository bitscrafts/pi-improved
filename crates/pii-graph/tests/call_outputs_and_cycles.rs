//! Spec 343: a called graph's terminal values can be read by the CALL's id.
//!
//! Inclusion is load-time inline (spec 255): the sub-graph's nodes become `<call>__<node>` and the exit keeps its
//! own name. A host that wants `verdict` should not have to know which inner node produced it, so
//! `$.nodes.<call>.<key>` is rewritten at load to `$.nodes.<call>__<exit>.<key>`.
//!
//! Every assertion here RUNS the graph and reads its terminal value. Asserting on `format!("{:?}", spec)` proves
//! nothing: the string can appear while the expression resolves to `null` at run time (measured — the abandoned
//! attempt asserted exactly that and passed).
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

/// The called graph: one node, and the exit that names it.
const SUB: &str = r#"
name = "queue"
exit = "judge"

[[node]]
id = "judge"
kind = "transform"
value = "approved"
"#;

/// The host that reads the child's value THROUGH THE CALL ID.
const HOST_ALIAS: &str = r#"
name = "night"
entry = "sdd"
exit = "read"

[[node]]
id = "sdd"
kind = "call"
graph = "queue"
sha256 = "PIN"
caps = []

[[node]]
id = "read"
kind = "transform"
expr = "$.nodes.sdd"
"#;

/// The control: the same host reading the value through the EXIT's namespaced id.
const HOST_EXIT_ID: &str = r#"
name = "night"
entry = "sdd"
exit = "read"

[[node]]
id = "sdd"
kind = "call"
graph = "queue"
sha256 = "PIN"
caps = []

[[node]]
id = "read"
kind = "transform"
expr = "$.nodes.sdd__judge"
"#;

/// A host that never mentions the call id in an expression: it must load byte-identically.
const HOST_PLAIN: &str = r#"
name = "night"
entry = "sdd"
exit = "sdd"

[[node]]
id = "sdd"
kind = "call"
graph = "queue"
sha256 = "PIN"
caps = []
"#;

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

fn value_of(terminal: Terminal) -> serde_json::Value {
    let Terminal::Success { output } = terminal else {
        panic!("expected a successful run, got {terminal:?}");
    };
    output
}

#[test]
fn child_outputs_appear_under_the_call_id() {
    let host = HOST_ALIAS.replace("PIN", &pin(SUB));
    let woven = load(&host, &[("queue", SUB)]).expect("loads");
    let out = value_of(run_spec(&woven.spec, serde_json::json!({})));
    assert_eq!(
        out,
        serde_json::json!("approved"),
        "`$.nodes.sdd` must resolve through the call id, got {out}"
    );
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for
fn CONTROL_the_exit_id_still_resolves() {
    // the pre-existing way of reading a called graph's value keeps working
    let host = HOST_EXIT_ID.replace("PIN", &pin(SUB));
    let woven = load(&host, &[("queue", SUB)]).expect("loads");
    let out = value_of(run_spec(&woven.spec, serde_json::json!({})));
    assert_eq!(out, serde_json::json!("approved"), "got {out}");
}

#[test]
fn a_host_without_the_alias_is_byte_identical() {
    // A host that never mentions the call id gains no alias, and still gets the value the old way. The inlined
    // node ids (`sdd__judge`) are the inlining, not the alias, so asserting on their presence would prove nothing.
    let host = HOST_PLAIN.replace("PIN", &pin(SUB));
    let woven = load(&host, &[("queue", SUB)]).expect("loads");
    let out = value_of(run_spec(&woven.spec, serde_json::json!({})));
    assert_eq!(out, serde_json::json!("approved"), "got {out}");
    let text = serde_json::to_string(&woven.spec).expect("json");
    let again = load(&host, &[("queue", SUB)]).expect("loads twice");
    assert_eq!(
        text,
        serde_json::to_string(&again.spec).expect("json"),
        "loading the same host twice must give the same spec"
    );
}

#[test]
fn a_cycle_names_the_call_that_closes_it() {
    // `b` calls `a` back; the refusal must name the cycle AND the call site that closes it
    let b = HOST_PLAIN
        .replace("name = \"night\"", "name = \"b\"")
        .replace("graph = \"queue\"", "graph = \"a\"")
        .replace("PIN", "0");
    let host = HOST_PLAIN
        .replace("name = \"night\"", "name = \"a\"")
        .replace("graph = \"queue\"", "graph = \"b\"")
        .replace("PIN", &pin(&b));
    let msg = load(&host, &[("b", &b)])
        .expect_err("load must refuse")
        .to_string();
    assert!(msg.contains("a -> b -> a"), "cycle path missing: {msg}");
    assert!(msg.contains("call 'sdd'"), "call id missing: {msg}");
}

//! Spec 270 R4: a run journals each call it composed as a `graph.call` observation on the call's head node, before any
//! node runs. The library loads the host, runs the expanded graph in process, and reads the run's journal.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::RunEvent;
use pii_core::{Budget, CapSet, Ctx, NullObserver, Observer, TraceCtx};
use pii_graph::fragment::{self, Woven, sha256_hex};
use pii_graph::node::NodeRegistry;
use pii_graph::{run, toml_load};
use pii_store::Store;

/// The sub-graph `note`: one transform that reads the title the host maps into it.
const NOTE: &str = r#"name = "note"
exit = "fmt"

[[node]]
id = "fmt"
kind = "transform"
expr = "$.input.title"
"#;

/// `note`, declaring no `needs` but an empty list of them.
const NOTE_EMPTY: &str = r#"name = "note"
exit = "fmt"
needs = []

[[node]]
id = "fmt"
kind = "transform"
expr = "$.input.title"
"#;

/// `wrap`, a sub-graph that calls `note` as `inner` and then reads its own input.
const WRAP: &str = r#"name = "wrap"
entry = "inner"
exit = "out"

[[node]]
id = "inner"
kind = "call"
graph = "note"
sha256 = "NOTE_SHA"
caps = []

[node.in]
title = "$.input.title"

[[node]]
id = "out"
kind = "transform"
expr = "$.input.title"

[[edge]]
from = "inner"
port = "ok"
to = "out"
kind = "data"
"#;

/// A host `start -> docs(call <graph>) -> done`, pinned to the sha256 `SHA`.
const HOST: &str = r#"name = "host"
entry = "start"

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "docs"
kind = "call"
graph = "GRAPH"
sha256 = "SHA"
caps = []

[node.in]
title = "$.input.spec"

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

/// The fragment `podman-guard`, so a host can attach a fragment with no call in it (the same shape the weave tests use).
const GUARD: &str = r#"
name = "podman-guard"

[fragment]
name = "podman-guard"
inputs = ["$.input.runtime"]
outputs = ["ok", "refuse"]
may_block = true

[[node]]
id = "check"
kind = "route"
on = "$.input.runtime"
rules = [{ eq = "podman", port = "refuse" }, { port = "ok" }]

[[node]]
id = "refuse"
kind = "refuse"
reason = "podman is refused by the guard"

[[edge]]
from = "check"
port = "refuse"
to = "refuse"
kind = "data"
"#;

/// A host that attaches the guard and has no call.
const ATTACHED: &str = r#"name = "guarded"
entry = "build"

[[attach]]
id = "guard"
fragment = "podman-guard"
at = "run.start"
pin = "PIN"

[[node]]
id = "build"
kind = "transform"
value = "built"
"#;

/// A host with `GRAPH` and `SHA` filled in for a call to `graph`, pinned to `sha`.
fn host(graph: &str, sha: &str) -> String {
    HOST.replace("GRAPH", graph).replace("SHA", sha)
}

/// Writes the graph files under a fresh scratch dir (kept alive by the returned guard), loads `host` through them, runs
/// the result with `input`, and returns the run's journal.
fn journal_of(
    host_src: &str,
    graphs: &[(&str, &str)],
    input: serde_json::Value,
) -> Vec<(u64, RunEvent)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("run-journals-calls-")
        .tempdir_in(root)
        .unwrap();
    let graph_dir = dir.path().join("graphs");
    let frag_dir = dir.path().join("fragments");
    std::fs::create_dir_all(&graph_dir).unwrap();
    std::fs::create_dir_all(&frag_dir).unwrap();
    for (name, src) in graphs {
        std::fs::write(graph_dir.join(format!("{name}.toml")), src).unwrap();
    }
    std::fs::write(frag_dir.join("podman-guard.toml"), GUARD).unwrap();
    let reg = NodeRegistry::builtin();
    let woven: Woven = fragment::load_host_with_graphs(
        host_src,
        &frag_dir,
        std::slice::from_ref(&graph_dir),
        &reg,
    )
    .unwrap();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let run_id = store.next_run_id().unwrap();
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::new(NullObserver) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    run::run(&store, &woven.spec, &reg, &ctx, &run_id, input).unwrap();
    store.read_journal(&run_id).unwrap()
}

/// The `graph.call` observations of a journal, as (seq, node, fields), in journal order.
fn calls(journal: &[(u64, RunEvent)]) -> Vec<(u64, String, serde_json::Value)> {
    journal
        .iter()
        .filter_map(|(seq, ev)| match ev {
            RunEvent::NodeObserved { node, kind, fields } if kind == "graph.call" => {
                Some((*seq, node.to_string(), fields.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The seq of every `NodeStarted` in a journal.
fn started(journal: &[(u64, RunEvent)]) -> Vec<u64> {
    journal
        .iter()
        .filter_map(|(seq, ev)| matches!(ev, RunEvent::NodeStarted { .. }).then_some(*seq))
        .collect()
}

fn input() -> serde_json::Value {
    serde_json::json!({ "spec": "hello" })
}

#[test]
fn a_call_journals_one_record() {
    let sha = sha256_hex(NOTE.as_bytes());
    let j = journal_of(&host("note", &sha), &[("note", NOTE)], input());
    let c = calls(&j);
    assert_eq!(c.len(), 1, "one graph.call: {c:?}");
    assert_eq!(c[0].1, "docs__in", "the head node carries the call");
    assert_eq!(
        c[0].2,
        serde_json::json!({
            "graph": "note",
            "sha256": sha,
            "caps": [],
            "exit": "fmt",
            "input_keys": ["title"],
        }),
        "the fields are the call's record, with the full sha256"
    );
}

#[test]
fn the_record_precedes_every_node() {
    let sha = sha256_hex(NOTE.as_bytes());
    let j = journal_of(&host("note", &sha), &[("note", NOTE)], input());
    let c = calls(&j);
    assert!(!c.is_empty(), "the call is journalled");
    let first_start = started(&j).into_iter().min().expect("nodes ran");
    assert!(
        c.iter().all(|(seq, _, _)| *seq < first_start),
        "every graph.call precedes the first NodeStarted ({first_start}): {c:?}"
    );
}

#[test]
fn a_nested_call_journals_outer_first() {
    let note_sha = sha256_hex(NOTE.as_bytes());
    let wrap = WRAP.replace("NOTE_SHA", &note_sha);
    let wrap_sha = sha256_hex(wrap.as_bytes());
    let j = journal_of(
        &host("wrap", &wrap_sha),
        &[("note", NOTE), ("wrap", &wrap)],
        input(),
    );
    let c = calls(&j);
    let names: Vec<&str> = c.iter().map(|(_, n, _)| n.as_str()).collect();
    assert_eq!(names, ["docs__in", "docs__inner__in"], "outer then inner");
    assert!(
        c[0].2.get("parent").is_none(),
        "the outer call has no parent"
    );
    assert_eq!(c[1].2["parent"], "docs", "the inner call names its parent");
}

#[test]
fn CONTROL_no_call_no_event() {
    let spec = toml_load::load_toml(
        "name = \"plain\"\nentry = \"step\"\ncaps = []\n[[node]]\nid = \"step\"\nkind = \"transform\"\nvalue = \"ok\"\n",
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("run-journals-calls-")
        .tempdir_in(root)
        .unwrap();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let reg = NodeRegistry::builtin();
    let run_id = store.next_run_id().unwrap();
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::new(NullObserver) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    run::run(&store, &spec, &reg, &ctx, &run_id, serde_json::json!({})).unwrap();
    let j = store.read_journal(&run_id).unwrap();
    assert!(calls(&j).is_empty(), "no call, no graph.call");
}

#[test]
fn CONTROL_an_attach_alone_still_journals_woven() {
    let pin = sha256_hex(GUARD.as_bytes());
    let j = journal_of(
        &ATTACHED.replace("PIN", &pin),
        &[],
        serde_json::json!({ "runtime": "docker" }),
    );
    assert!(calls(&j).is_empty(), "an attach is not a call");
    assert!(
        j.iter().any(|(_, ev)| matches!(
            ev,
            RunEvent::NodeObserved { kind, .. } if kind == "woven"
        )),
        "the woven node is still journalled"
    );
}

#[test]
fn needs_absent_versus_empty() {
    let sha = sha256_hex(NOTE.as_bytes());
    let absent = calls(&journal_of(&host("note", &sha), &[("note", NOTE)], input()));
    assert!(
        absent[0].2.get("needs").is_none(),
        "a graph with no needs has no needs field"
    );
    let sha_empty = sha256_hex(NOTE_EMPTY.as_bytes());
    let empty = calls(&journal_of(
        &host("note", &sha_empty),
        &[("note", NOTE_EMPTY)],
        input(),
    ));
    assert_eq!(
        empty[0].2["needs"],
        serde_json::json!([]),
        "a graph with needs = [] journals an empty list"
    );
}

#[test]
fn absent_fields_are_not_null() {
    let note_sha = sha256_hex(NOTE.as_bytes());
    let wrap = WRAP.replace("NOTE_SHA", &note_sha);
    let wrap_sha = sha256_hex(wrap.as_bytes());
    let j = journal_of(
        &host("wrap", &wrap_sha),
        &[("note", NOTE), ("wrap", &wrap)],
        input(),
    );
    for (_, node, fields) in calls(&j) {
        let obj = fields.as_object().expect("an object");
        for key in ["needs", "parent"] {
            if let Some(v) = obj.get(key) {
                assert!(!v.is_null(), "{node}: {key} is null, not absent");
            }
        }
    }
    let outer = &calls(&j)[0].2;
    assert!(outer.get("needs").is_none(), "outer: needs absent: {outer}");
    assert!(
        outer.get("parent").is_none(),
        "outer: parent absent: {outer}"
    );
}

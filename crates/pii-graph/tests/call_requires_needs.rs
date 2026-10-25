//! Spec 267: a sub-graph's declared `needs` that resolves to nothing refuses the run before any sub-graph node runs.
//! The call's chain gets one `require` node, `<call>__need`, after the mapped-input transforms and before the sub-graph
//! entry. Each test loads a host with a real temp graph dir and runs the expanded graph in-process.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_abi::{PiiError, RunEvent, Terminal};
use pii_core::{Budget, CapSet, Ctx, ObsEvent, Observer, RecordingObserver, TraceCtx};
use pii_graph::fragment::{self, Woven};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_store::Store;
use serde_json::{Value, json};

/// The sub-graph `note` with the given top-level header (`needs = [...]` or empty), one node `a` that reads `expr`,
/// and exit `a`.
fn note(header: &str, expr: &str) -> String {
    format!(
        "name = \"note\"\nexit = \"a\"\n{header}\n[[node]]\nid = \"a\"\nkind = \"transform\"\nexpr = \"{expr}\"\n\n"
    )
}

/// The host: the call `docs` (entry) to the graph `note` pinned to `sha`, with `in_table` lines after the call, then
/// `done` passing the call's output on.
fn host(sha: &str, in_table: &str) -> String {
    format!(
        r#"name = "host"

[[node]]
id = "docs"
kind = "call"
graph = "note"
sha256 = "{sha}"
caps = []

{in_table}[[node]]
id = "done"
kind = "transform"
expr = "$.payload"

[[edge]]
from = "docs"
port = "ok"
to = "done"
kind = "data"
"#
    )
}

/// Writes the graph `note` under a fresh temp dir and loads the host with it.
fn load_note(header: &str, expr: &str, in_table: &str) -> Result<Woven, PiiError> {
    let sub = note(header, expr);
    let root = tempfile::tempdir().expect("tempdir");
    let graph_dir = root.path().join("graphs");
    std::fs::create_dir_all(&graph_dir).expect("mkdir graphs");
    std::fs::write(graph_dir.join("note.toml"), &sub).expect("write graph");
    let host_src = host(&fragment::sha256_hex(sub.as_bytes()), in_table);
    fragment::load_host_with_graphs(
        &host_src,
        &root.path().join("fragments"),
        std::slice::from_ref(&graph_dir),
        &NodeRegistry::builtin(),
    )
}

/// Runs the woven graph once with `input`; returns the terminal and the observer that saw the run's journal.
fn run_woven(woven: &Woven, input: Value) -> (Terminal, Arc<RecordingObserver>) {
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
    let terminal = run::run_with_validate(
        &store,
        &woven.spec,
        &NodeRegistry::builtin(),
        &ctx,
        &run_id,
        input,
        true,
    )
    .expect("run");
    (terminal, obs)
}

/// The run's failure text; the run must have failed.
fn refusal(terminal: &Terminal) -> String {
    match terminal {
        Terminal::Failure { error } => error.clone(),
        other => panic!("expected a refused run, got {other:?}"),
    }
}

/// True when the observer saw node `node` start.
fn started(obs: &RecordingObserver, node: &str) -> bool {
    obs.events().iter().any(|e| {
        matches!(e, ObsEvent::Journal { event: RunEvent::NodeStarted { node: n, .. }, .. } if n.as_str() == node)
    })
}

/// `needs = [...]` for the given keys, as a header line.
fn needs(keys: &[&str]) -> String {
    let items: Vec<String> = keys.iter().map(|k| format!("\"{k}\"")).collect();
    format!("needs = [{}]", items.join(", "))
}

#[test]
fn a_present_key_runs_the_subgraph() {
    let woven = load_note(&needs(&["spec"]), "$.input.spec", "").expect("load");
    let (terminal, _) = run_woven(&woven, json!({ "spec": "S" }));
    let Terminal::Success { output } = terminal else {
        panic!("expected success, got {terminal:?}");
    };
    assert_eq!(output, json!("S"));
}

#[test]
fn an_absent_key_refuses_before_any_subgraph_node() {
    let woven = load_note(&needs(&["spec"]), "$.input.spec", "").expect("load");
    let (terminal, obs) = run_woven(&woven, json!({}));
    assert_eq!(
        refusal(&terminal),
        "graph.call.need: call 'docs' graph 'note' needs 'spec'"
    );
    assert!(!started(&obs, "docs__a"), "the sub-graph node must not run");
}

#[test]
fn a_mapped_key_that_resolves_runs() {
    let in_table = "[node.in]\nspec = \"$.input.raw\"\n\n";
    let woven = load_note(&needs(&["spec"]), "$.input.spec", in_table).expect("load");
    let (terminal, _) = run_woven(&woven, json!({ "raw": "R" }));
    let Terminal::Success { output } = terminal else {
        panic!("expected success, got {terminal:?}");
    };
    assert_eq!(output, json!("R"));
}

#[test]
fn a_mapped_key_that_is_null_refuses() {
    let in_table = "[node.in]\nspec = \"$.input.missing\"\n\n";
    let woven = load_note(&needs(&["spec"]), "$.input.spec", in_table).expect("load");
    let (terminal, obs) = run_woven(&woven, json!({}));
    assert_eq!(
        refusal(&terminal),
        "graph.call.need: call 'docs' graph 'note' needs 'spec'"
    );
    assert!(!started(&obs, "docs__a"), "the sub-graph node must not run");
}

#[test]
fn the_second_missing_key_is_the_one_named() {
    let woven = load_note(&needs(&["first", "second"]), "$.input.second", "").expect("load");
    let (terminal, _) = run_woven(&woven, json!({ "first": 1 }));
    assert_eq!(
        refusal(&terminal),
        "graph.call.need: call 'docs' graph 'note' needs 'second'"
    );
}

#[test]
fn control_without_needs_nothing_changes() {
    // CONTROL: no `needs`, so an absent `$.input.x` reads Null and the run goes on as before.
    let woven = load_note("", "$.input.x", "").expect("load");
    assert!(
        !woven
            .spec
            .nodes
            .iter()
            .any(|n| n.id.as_str() == "docs__need"),
        "a sub-graph without needs gets no require node"
    );
    let (terminal, obs) = run_woven(&woven, json!({}));
    let Terminal::Success { output } = terminal else {
        panic!("expected success, got {terminal:?}");
    };
    assert_eq!(output, Value::Null);
    assert!(started(&obs, "docs__a"), "the sub-graph node runs");
}

#[test]
fn control_an_empty_string_is_accepted() {
    // CONTROL: only Null refuses; an empty string is a value.
    let woven = load_note(&needs(&["spec"]), "$.input.spec", "").expect("load");
    let (terminal, _) = run_woven(&woven, json!({ "spec": "" }));
    let Terminal::Success { output } = terminal else {
        panic!("expected success, got {terminal:?}");
    };
    assert_eq!(output, json!(""));
}

#[test]
fn a_require_without_keys_is_refused_at_build() {
    let reg = NodeRegistry::builtin();
    let factory = reg.factory("require").expect("require is a built-in kind");
    let no_keys = factory.build(&json!({ "call": "docs", "graph": "note" }));
    assert!(
        matches!(no_keys.err(), Some(PiiError::Invalid(_))),
        "a require without keys is refused with PiiError::Invalid"
    );
    let bad_keys = factory.build(&json!({ "call": "docs", "graph": "note", "keys": "spec" }));
    assert!(
        matches!(bad_keys.err(), Some(PiiError::Invalid(_))),
        "a require whose keys are not a table is refused with PiiError::Invalid"
    );
}

#[test]
fn the_need_node_exists_only_with_needs() {
    let with = load_note(&needs(&["spec"]), "$.input.spec", "").expect("load");
    let ids: Vec<&str> = with.spec.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&"docs__need"), "got: {ids:?}");
    assert!(ids.contains(&"docs__a"), "got: {ids:?}");

    let without = load_note("", "$.input.x", "").expect("load");
    let ids: Vec<&str> = without.spec.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(!ids.contains(&"docs__need"), "got: {ids:?}");
}

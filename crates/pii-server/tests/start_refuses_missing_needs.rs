//! Spec 288: a call's declared `needs` that the run input cannot supply refuses the run at `Engine::start_run`, before any
//! run exists. A key the call's `in` table maps is left to the run-time `require` node (spec 267), as is a key that
//! resolves only at run time. Each test writes its graphs to a temp dir and starts them through the engine.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{RunEvent, RunId, RunStatus};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;
use serde_json::json;

/// The sub-graph `note`: one transform that reads `$.input.title`, declaring `needs` with `needs` (empty for none).
fn note(needs: &str) -> String {
    format!(
        "name = \"note\"\nexit = \"fmt\"\n{needs}\n[[node]]\nid = \"fmt\"\nkind = \"transform\"\nexpr = \"$.input.title\"\n"
    )
}

/// A host: `start`, then the call `docs` (to `note`, pinned to `sha`, with `in_table` after it), then `done`.
fn host(name: &str, sha: &str, in_table: &str) -> String {
    format!(
        r#"name = "{name}"
entry = "start"

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "docs"
kind = "call"
graph = "note"
sha256 = "{sha}"
caps = []

{in_table}
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
"#
    )
}

fn hello() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

/// An engine whose graph dir holds the given `(name, toml)` graphs.
fn engine_with(dir: &Path, graphs: &[(&str, String)]) -> Arc<Engine> {
    let graph_dir = dir.join("graphs");
    std::fs::create_dir_all(&graph_dir).unwrap();
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    for (name, toml) in graphs {
        std::fs::write(graph_dir.join(format!("{name}.toml")), toml).unwrap();
    }
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.join("ws"),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir,
    })
    .expect("engine")
}

/// A host `docs-host` calling `note` (declaring `needs = ["title"]`) with no `in` mapping: `title` comes from the input.
fn needs_title(dir: &Path) -> Arc<Engine> {
    let note = note("needs = [\"title\"]");
    let sha = pii_graph::fragment::sha256_hex(note.as_bytes());
    engine_with(
        dir,
        &[("note", note), ("docs-host", host("docs-host", &sha, ""))],
    )
}

/// Asserts that no run is held by the store in any status.
fn holds_no_run(engine: &Engine) {
    for status in [
        RunStatus::Running,
        RunStatus::Suspended,
        RunStatus::Succeeded,
        RunStatus::Failed,
        RunStatus::Refused,
    ] {
        let runs: Vec<RunId> = engine.store.runs_by_status(status).unwrap();
        assert!(runs.is_empty(), "no run is created: {status:?} {runs:?}");
    }
}

/// Polls the store until the run reaches a terminal status, then returns the status and the journal's failures.
async fn settle(engine: &Engine, run: &RunId) -> (RunStatus, Vec<String>) {
    for _ in 0..500 {
        if let Some(header) = engine.store.get_run(run).unwrap()
            && matches!(header.status, RunStatus::Succeeded | RunStatus::Failed)
        {
            let errors = engine
                .store
                .read_journal(run)
                .unwrap()
                .into_iter()
                .filter_map(|(_, ev)| match ev {
                    RunEvent::NodeFailed { error, .. } => Some(error),
                    _ => None,
                })
                .collect();
            return (header.status, errors);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run {run} did not settle");
}

#[tokio::test]
async fn a_missing_needs_key_refuses_at_start_with_no_run() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = needs_title(tmp.path());

    let Err(err) = engine.start_run("docs-host", &json!({})) else {
        panic!("a run missing a declared key is refused at start");
    };
    assert!(
        err.to_string()
            .contains("graph.call.need: call 'docs' graph 'note' needs 'title'"),
        "the run-time node's text: {err}"
    );
    holds_no_run(&engine);
}

#[tokio::test]
async fn the_first_missing_key_in_needs_order_is_named() {
    let tmp = tempfile::tempdir().unwrap();
    let note = note("needs = [\"zeta\", \"title\"]");
    let sha = pii_graph::fragment::sha256_hex(note.as_bytes());
    let engine = engine_with(
        tmp.path(),
        &[("note", note), ("docs-host", host("docs-host", &sha, ""))],
    );

    let Err(err) = engine.start_run("docs-host", &json!({ "title": "x" })) else {
        panic!("'zeta' is missing, so the run is refused");
    };
    assert!(err.to_string().ends_with("needs 'zeta'"), "{err}");
    holds_no_run(&engine);
}

#[tokio::test]
async fn a_present_needs_key_starts_the_run() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = needs_title(tmp.path());

    engine
        .start_run("docs-host", &json!({ "title": "hello" }))
        .expect("a present key starts the run");
}

#[tokio::test]
async fn an_empty_string_is_a_value_and_starts_the_run() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = needs_title(tmp.path());

    engine
        .start_run("docs-host", &json!({ "title": "" }))
        .expect("an empty string is a value");
}

#[tokio::test]
async fn a_null_needs_key_refuses_at_start() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = needs_title(tmp.path());

    let Err(err) = engine.start_run("docs-host", &json!({ "title": null })) else {
        panic!("a null declared key is refused at start");
    };
    assert!(err.to_string().ends_with("needs 'title'"), "{err}");
    holds_no_run(&engine);
}

#[tokio::test]
async fn a_mapped_key_is_left_to_the_run_time_node() {
    // The host maps `title` from `$.payload.absent`, an expression the preflight cannot decide (it is not a plain
    // `$.input.<name>` read). It resolves to null at run time, so the run starts, and the run-time `require` refuses.
    let tmp = tempfile::tempdir().unwrap();
    let note = note("needs = [\"title\"]");
    let sha = pii_graph::fragment::sha256_hex(note.as_bytes());
    let in_table = "[node.in]\ntitle = \"$.payload.absent\"\n";
    let engine = engine_with(
        tmp.path(),
        &[
            ("note", note),
            ("docs-host", host("docs-host", &sha, in_table)),
        ],
    );

    let run = engine
        .start_run("docs-host", &json!({}))
        .expect("the preflight does not refuse a mapped key");
    let (status, errors) = settle(&engine, &run).await;
    assert!(matches!(status, RunStatus::Failed), "{status:?}");
    assert!(
        errors
            .iter()
            .any(|e| e == "graph.call.need: call 'docs' graph 'note' needs 'title'"),
        "the run-time node refuses: {errors:?}"
    );
}

#[tokio::test]
async fn a_key_mapped_from_a_missing_input_refuses_at_start() {
    // `title = "$.input.spec"` is a plain run-input read, so the preflight decides it: `spec` is absent, and no run exists.
    let tmp = tempfile::tempdir().unwrap();
    let note = note("needs = [\"title\"]");
    let sha = pii_graph::fragment::sha256_hex(note.as_bytes());
    let in_table = "[node.in]\ntitle = \"$.input.spec\"\n";
    let engine = engine_with(
        tmp.path(),
        &[
            ("note", note),
            ("docs-host", host("docs-host", &sha, in_table)),
        ],
    );

    let Err(err) = engine.start_run("docs-host", &json!({})) else {
        panic!("a mapped plain input read that is absent refuses at start");
    };
    assert!(
        err.to_string()
            .contains("graph.call.need: call 'docs' graph 'note' needs 'title'"),
        "{err}"
    );
    holds_no_run(&engine);
}

#[tokio::test]
async fn a_graph_without_calls_starts_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = "name = \"plain\"\nentry = \"start\"\n\n[[node]]\nid = \"start\"\nkind = \"transform\"\nvalue = \"go\"\n";
    let engine = engine_with(tmp.path(), &[("plain", plain.to_owned())]);

    engine
        .start_run("plain", &json!({}))
        .expect("a graph with no call starts");
}

#[tokio::test]
async fn a_call_without_needs_starts_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let note = note("");
    let sha = pii_graph::fragment::sha256_hex(note.as_bytes());
    let engine = engine_with(
        tmp.path(),
        &[("note", note), ("docs-host", host("docs-host", &sha, ""))],
    );

    engine
        .start_run("docs-host", &json!({}))
        .expect("a call whose graph declares no needs starts");
}

#[tokio::test]
async fn a_nested_call_missing_its_needs_key_refuses_at_start() {
    // `docs` maps `title` into `mid` from `$.payload`, which the preflight leaves to the run. `mid` declares `title` and
    // calls `leaf` (needs `title`, mapped by nothing); the host's input has no `title`, so the nested `inner` is refused.
    let tmp = tempfile::tempdir().unwrap();
    let leaf = note("needs = [\"title\"]");
    let leaf_sha = pii_graph::fragment::sha256_hex(leaf.as_bytes());
    let mid = format!(
        r#"name = "mid"
entry = "inner"
exit = "tail"
needs = ["title"]

[[node]]
id = "inner"
kind = "call"
graph = "note"
sha256 = "{leaf_sha}"
caps = []

[[node]]
id = "tail"
kind = "transform"
expr = "$.payload"

[[edge]]
from = "inner"
port = "ok"
to = "tail"
kind = "data"
"#
    );
    let mid_sha = pii_graph::fragment::sha256_hex(mid.as_bytes());
    let in_table = "[node.in]\ntitle = \"$.payload\"\n";
    let outer =
        host("docs-host", &mid_sha, in_table).replace("graph = \"note\"", "graph = \"mid\"");
    let engine = engine_with(
        tmp.path(),
        &[("note", leaf), ("mid", mid), ("docs-host", outer)],
    );

    let Err(err) = engine.start_run("docs-host", &json!({})) else {
        panic!("the nested call's missing key refuses the run at start");
    };
    assert!(
        err.to_string()
            .contains("graph.call.need: call 'inner' graph 'note' needs 'title'"),
        "{err}"
    );
    holds_no_run(&engine);
}

/// With BOTH keys missing the first one in `needs` order is named, in either order. (The first version of this check
/// supplied `title`, so only `zeta` was missing: a scan that ran the keys backwards named the same key and passed.)
#[tokio::test]
async fn both_keys_missing_names_the_first_in_either_order() {
    for (needs, first) in [
        ("needs = [\"zeta\", \"title\"]", "zeta"),
        ("needs = [\"title\", \"zeta\"]", "title"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let note = note(needs);
        let sha = pii_graph::fragment::sha256_hex(note.as_bytes());
        let engine = engine_with(
            tmp.path(),
            &[("note", note), ("docs-host", host("docs-host", &sha, ""))],
        );
        let Err(err) = engine.start_run("docs-host", &json!({})) else {
            panic!("both keys are missing, so the run is refused ({needs})");
        };
        assert!(
            err.to_string().ends_with(&format!("needs '{first}'")),
            "{needs}: the first key in order is named: {err}"
        );
        holds_no_run(&engine);
    }
}

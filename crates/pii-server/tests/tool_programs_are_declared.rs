//! Spec 399 R8/R9: a `tool` node with `tool = "run"` declares its `programs`. It is refused at load without them, and at run
//! time it executes an argv only when the argv's first entry EQUALS a declared program. The run scope's admission does
//! not stand in for the declaration, and a declared `cargo` reaches the tool node while staying claimed by `coder`.
//!
//! The run-time cases go through `Engine::start_run`, so the dispatch-level scope (`node_proc_argv0`) is the one that
//! decides, not a scope the test builds by hand.
#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::Duration;

use pii_abi::{NodeId, PortName, RunEvent, RunId, RunStatus};
use pii_core::ArgvScope;
use pii_graph::fragment;
use pii_graph::sched::dispatch::node_proc_argv0;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// A graph whose one `tool` node runs `argv`, declaring `programs` when given. Both outcomes route to `done`, so a
/// denial is a journalled `err` on `build`, not a failed run.
fn probe(programs: Option<&str>, argv: &str) -> String {
    let programs = programs.map_or_else(String::new, |p| format!("programs = {p}\n"));
    format!(
        r#"name = "probe"
entry = "build"
exit = "done"
caps = ["fs_read", "fs_write", "proc"]

[[node]]
id = "build"
kind = "tool"
tool = "run"
{programs}args = {{ argv = {argv} }}
caps = ["proc"]

[[node]]
id = "done"
kind = "transform"
value = "done"
join = "any"

[[edge]]
from = "build"
port = "ok"
to = "done"
kind = "data"

[[edge]]
from = "build"
port = "err"
to = "done"
kind = "data"
"#
    )
}

/// An engine whose graph dir holds `probe.toml`, with a workspace the tool node's cwd defaults into.
fn engine_with(dir: &std::path::Path, graph: &str) -> Arc<Engine> {
    let graphs = dir.join("graphs");
    std::fs::create_dir_all(&graphs).unwrap();
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::write(graphs.join("probe.toml"), graph).unwrap();
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.join("ws"),
        store_path: dir.join("store.redb"),
        model: format!(
            "scripted:{}",
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/phase2/hello.json")
                .display()
        ),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: graphs,
        mqtt_tool_url: None,
    })
    .expect("engine")
}

/// Polls the store until the run is terminal, then returns its status and journal.
async fn settle(engine: &Engine, run: &RunId) -> (RunStatus, Vec<(u64, RunEvent)>) {
    for _ in 0..500 {
        if let Some(header) = engine.store.get_run(run).unwrap()
            && matches!(header.status, RunStatus::Succeeded | RunStatus::Failed)
        {
            return (header.status, engine.store.read_journal(run).unwrap());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run {run} did not settle");
}

/// The port and value the `build` node finished with.
fn build_finished(journal: &[(u64, RunEvent)]) -> (PortName, serde_json::Value) {
    journal
        .iter()
        .find_map(|(_, ev)| match ev {
            RunEvent::NodeFinished {
                node, port, value, ..
            } if *node == NodeId::from("build") => Some((port.clone(), value.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("build never finished: {journal:?}"))
}

/// Runs the probe with `programs` and `argv`, and returns the port `build` took and its value.
async fn run_probe(programs: &str, argv: &str) -> (PortName, serde_json::Value) {
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_with(tmp.path(), &probe(Some(programs), argv));
    let run = engine
        .start_run("probe", &serde_json::json!({}))
        .expect("the probe graph starts");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );
    build_finished(&journal)
}

#[test]
fn a_run_node_without_programs_is_refused_at_load() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    let runtime = tmp.path().join("runtime");
    std::fs::create_dir_all(tmp.path().join("graphs/fragments")).unwrap();
    std::fs::create_dir_all(&ws).unwrap();
    let graph = probe(None, "[\"true\"]");
    let err = fragment::load_host_with_graphs(
        &graph,
        &tmp.path().join("graphs/fragments"),
        &[tmp.path().join("graphs")],
        &pii_server::lint_registry(&ws, &runtime),
    )
    .expect_err("a `run` tool node without `programs` must be refused at load")
    .to_string();
    assert!(err.contains("build"), "names the node: {err}");
    assert!(err.contains("programs"), "names what is missing: {err}");
}

#[tokio::test]
async fn an_undeclared_program_is_denied_even_if_the_run_scope_has_it() {
    // `cargo` is in the run scope (the coder's criteria table); this node declares only `true`.
    let (port, value) = run_probe("[\"true\"]", "[\"cargo\", \"--version\"]").await;
    assert_eq!(port, PortName::from("err"), "denied, not run: {value}");
    let error = value["error"].as_str().unwrap_or_default();
    assert!(error.contains("cargo"), "names the program: {value}");
    assert!(value.get("stdout").is_none(), "nothing ran: {value}");
}

#[tokio::test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
async fn CONTROL_a_declared_cargo_runs_for_a_tool_node() {
    let (port, value) = run_probe("[\"cargo\"]", "[\"cargo\", \"--version\"]").await;
    assert_eq!(
        port,
        PortName::from("ok"),
        "the declared program runs: {value}"
    );
    // The control proves the program was ADMITTED and SPAWNED, not that cargo can print a version: a tool node's child gets a
    // scrubbed environment (PATH, HOME, LANG, ...) with no RUSTUP_HOME, so a rustup shim cannot choose a toolchain here
    // (measured 2026-11-09). A denial never carries an exit code; a spawned process always does.
    assert!(
        value["exit_code"].is_number(),
        "the declared program was spawned, so the result carries an exit code: {value}"
    );
}

#[test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
fn CONTROL_cargo_stays_claimed_against_a_non_coder_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = pii_server::lint_registry(&tmp.path().join("ws"), &tmp.path().join("runtime"));
    // The run scope admits cargo; the coder kind claims it through the criteria table.
    let root = ArgvScope::new(vec![String::from("cargo")]);
    let scope = node_proc_argv0(&reg, "transform", &root);
    assert!(
        !scope.allowed().iter().any(|p| p == "cargo"),
        "a transform node must not be admitted cargo: {:?}",
        scope.allowed()
    );
}

#[tokio::test]
async fn a_prefix_of_a_declared_program_is_not_the_program() {
    // `car` is declared; the argv's program is `cargo`. The check is exact, so this is a denial.
    let (port, value) = run_probe("[\"car\"]", "[\"cargo\", \"--version\"]").await;
    assert_eq!(port, PortName::from("err"), "denied, not run: {value}");
    assert!(
        value["error"]
            .as_str()
            .unwrap_or_default()
            .contains("cargo"),
        "names the program: {value}"
    );
}

//! Spec 379: the books graph is a composition of `docs` with a stale inventory and a truth check.
//!
//!   * the one writing step is a `call` of `docs`, pinned to the digest of `graphs/docs.toml` on disk (R2);
//!   * it declares `exit = "done"` and has the `nothing-to-do` and `unverified` ports (R2);
//!   * it loads, so the call resolves (R2);
//!   * the call cannot widen the host's capabilities (R5).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{NodeId, PortName, RunEvent, RunStatus};
use pii_core::CapKind;
use pii_graph::fragment;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn graph(name: &str) -> String {
    std::fs::read_to_string(repo().join(format!("graphs/{name}.toml")))
        .unwrap_or_else(|e| panic!("graphs/{name}.toml: {e}"))
}

fn table(src: &str) -> toml::Table {
    toml::from_str(src).expect("valid toml")
}

fn nodes(doc: &toml::Table) -> Vec<&toml::Table> {
    doc.get("node")
        .and_then(|n| n.as_array())
        .map(|nodes| nodes.iter().filter_map(toml::Value::as_table).collect())
        .unwrap_or_default()
}

fn node_ids(doc: &toml::Table) -> BTreeSet<String> {
    nodes(doc)
        .iter()
        .filter_map(|n| n.get("id").and_then(toml::Value::as_str).map(str::to_owned))
        .collect()
}

#[test]
fn books_graph_declares_its_exit_and_the_docs_call() {
    let src = graph("books");
    let doc = table(&src);
    assert_eq!(doc.get("exit").and_then(toml::Value::as_str), Some("done"));
    assert_eq!(
        src.matches("graph = \"docs\"").count(),
        1,
        "one `graph = \"docs\"` line"
    );
}

#[test]
fn books_graph_has_the_inventory_and_the_honest_middle() {
    let ids = node_ids(&table(&graph("books")));
    for want in [
        "inventory",
        "select",
        "nothing-to-do",
        "call-docs",
        "verify",
        "done",
        "unverified",
    ] {
        assert!(ids.contains(want), "books has no `{want}` node: {ids:?}");
    }
}

#[test]
fn books_graph_writes_through_one_pinned_call_of_docs() {
    let src = graph("books");
    let doc = table(&src);
    let calls: Vec<&toml::Table> = nodes(&doc)
        .into_iter()
        .filter(|n| n.get("kind").and_then(toml::Value::as_str) == Some("call"))
        .collect();
    assert_eq!(calls.len(), 1, "exactly one writing step: {calls:?}");
    let pin = calls[0]
        .get("sha256")
        .and_then(toml::Value::as_str)
        .expect("the call is pinned");
    assert_eq!(
        pin,
        fragment::sha256_hex(graph("docs").as_bytes()),
        "the pin must be the sha256 of graphs/docs.toml as it is on disk"
    );
}

#[test]
fn books_graph_loads_and_its_call_resolves() {
    let woven = fragment::load_host_with_graphs(
        &graph("books"),
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&repo().join("graphs")),
        &pii_graph::node::NodeRegistry::builtin(),
    )
    .expect("books loads");
    assert!(
        woven.calls.iter().any(|c| c.graph == "docs"),
        "the write step must inline graphs/docs.toml: {:?}",
        woven.calls
    );
}

/// R5: the host grants `fs_write`; a ceiling without it refuses the graph, naming the node.
#[test]
fn the_call_cannot_widen() {
    let reg = pii_graph::node::NodeRegistry::builtin()
        .with_grant_ceiling(CapKind::FS_READ | CapKind::PROC | CapKind::SPAWN);
    let err = fragment::load_host_with_graphs(
        &graph("books"),
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&repo().join("graphs")),
        &reg,
    )
    .expect_err("a node beyond the ceiling must not load");
    let msg = err.to_string();
    assert!(msg.contains("fs_write"), "names the capability: {msg}");
}

#[test]
fn books_graph_carries_no_absolute_project_paths() {
    let src = graph("books");
    for path in ["/projects/", "/root/"] {
        assert!(!src.contains(path), "books may not hard-code {path}");
    }
}

/// Runs `books` with an inventory that prints nothing and a verify that passes, through the engine, and settles it.
async fn run_books_with_an_empty_inventory() -> (RunStatus, Vec<(u64, RunEvent)>) {
    run_books(&serde_json::json!({
        "agent": "claude",
        "model": "scripted",
        "provider": "scripted",
        "spec": "specs/none.md",
        "inventory": ["true"],
        "verify": ["true"],
        "programs": ["true"],
    }))
    .await
}

/// Runs `books` with `input` (its `workdir` is filled in), through the engine, and settles it.
async fn run_books(input: &serde_json::Value) -> (RunStatus, Vec<(u64, RunEvent)>) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let engine: Arc<Engine> = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: ws.clone(),
        store_path: tmp.path().join("store.redb"),
        model: format!(
            "scripted:{}",
            repo().join("fixtures/phase2/hello.json").display()
        ),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: repo().join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine");
    // Spec 399 R4: the commands are argv arrays; the program comes from the graph file (spec 400 R4).
    let mut input = input.clone();
    input["workdir"] = serde_json::json!(std::fs::canonicalize(&ws).unwrap());
    let run = engine
        .start_run("books", &input)
        .expect("books starts with its inputs");
    for _ in 0..500 {
        if let Some(header) = engine.store.get_run(&run).unwrap()
            && matches!(header.status, RunStatus::Succeeded | RunStatus::Failed)
        {
            let journal = engine.store.read_journal(&run).unwrap();
            return (header.status, journal);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run {run} did not settle");
}

/// The port a node finished on, if it finished.
fn finished_on(journal: &[(u64, RunEvent)], id: &str) -> Option<PortName> {
    journal.iter().find_map(|(_, ev)| match ev {
        RunEvent::NodeFinished { node, port, .. } if *node == NodeId::from(id) => {
            Some(port.clone())
        }
        _ => None,
    })
}

fn started(journal: &[(u64, RunEvent)], id: &str) -> bool {
    journal.iter().any(|(_, ev)| match ev {
        RunEvent::NodeStarted { node, .. } => *node == NodeId::from(id),
        _ => false,
    })
}

/// Spec 399 R4: `inventory` is a `run` tool node whose argv arrives as input, so the graph runs to its exit.
#[tokio::test]
async fn books_runs_its_inventory_as_an_argv() {
    let (status, journal) = run_books_with_an_empty_inventory().await;
    assert_eq!(
        finished_on(&journal, "inventory"),
        Some(PortName::from("ok")),
        "{journal:?}"
    );
    assert_eq!(
        finished_on(&journal, "done"),
        Some(PortName::from("ok")),
        "{journal:?}"
    );
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );
}

/// Spec 399 R4: an inventory that prints nothing still ends on `nothing-to-do`, and the writing step never starts.
#[tokio::test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
async fn CONTROL_books_still_ends_on_nothing_to_do_for_an_empty_inventory() {
    let (status, journal) = run_books_with_an_empty_inventory().await;
    assert!(
        finished_on(&journal, "nothing-to-do").is_some(),
        "{journal:?}"
    );
    assert!(
        !started(&journal, "call-docs"),
        "the writing step must not start: {journal:?}"
    );
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );
}

/// Spec 400 R4: `books` declares its program in the file. Every `run` tool node carries a literal, non-empty `programs`
/// array, and `programs` is no longer a run input the graph needs.
#[test]
fn books_declares_its_program_in_the_file() {
    let src = graph("books");
    let doc = table(&src);
    let runs: Vec<&toml::Table> = nodes(&doc)
        .into_iter()
        .filter(|n| {
            n.get("kind").and_then(toml::Value::as_str) == Some("tool")
                && n.get("tool").and_then(toml::Value::as_str) == Some("run")
        })
        .collect();
    assert!(!runs.is_empty(), "books has `run` tool nodes");
    for node in runs {
        let id = node.get("id").and_then(toml::Value::as_str).unwrap_or("?");
        let programs = node
            .get("programs")
            .and_then(toml::Value::as_array)
            .unwrap_or_else(|| panic!("`{id}` declares `programs` as a literal array: {node:?}"));
        assert!(!programs.is_empty(), "`{id}` declares a program");
        for program in programs {
            let name = program.as_str().unwrap_or_default();
            assert!(
                !name.trim().is_empty() && !name.starts_with("$."),
                "`{id}` declares a literal name, not {program:?}"
            );
        }
    }
    let needs: Vec<&str> = doc
        .get("needs")
        .and_then(toml::Value::as_array)
        .map(|n| n.iter().filter_map(toml::Value::as_str).collect())
        .unwrap_or_default();
    assert!(
        !needs.contains(&"programs"),
        "the run input may not supply `programs`: {needs:?}"
    );
}

/// Spec 400 R4: a run input cannot widen the programs. `books` is run with a hostile `programs = ["python3"]` input and an
/// inventory that is a `python3` argv; the file declares no `python3`, so the node is denied and nothing ran.
#[tokio::test]
async fn run_input_cannot_widen_the_programs() {
    let (_status, journal) = run_books(&serde_json::json!({
        "agent": "claude",
        "model": "scripted",
        "provider": "scripted",
        "spec": "specs/none.md",
        "inventory": ["python3", "-c", "print(1)"],
        "verify": ["true"],
        "programs": ["python3"],
    }))
    .await;
    assert_eq!(
        finished_on(&journal, "inventory"),
        Some(PortName::from("err")),
        "the hostile program is denied, not run: {journal:?}"
    );
    let value = journal
        .iter()
        .find_map(|(_, ev)| match ev {
            RunEvent::NodeFinished { node, value, .. } if *node == NodeId::from("inventory") => {
                Some(value.clone())
            }
            _ => None,
        })
        .unwrap_or_default();
    let error = value["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("is not in this node's `programs`"),
        "denied by the declaration, not by a spawn: {value}"
    );
    assert!(value.get("stdout").is_none(), "nothing ran: {value}");
    assert!(value.get("exit_code").is_none(), "nothing ran: {value}");
}

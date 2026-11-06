//! Spec 270 R4 (h): a host that calls a graph, started through the engine (the daemon's path, which the CLI also reaches),
//! journals the call as a `graph.call` observation on its head node, the same event the library journals.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::Duration;

use pii_abi::{RunEvent, RunId, RunStatus};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// The sub-graph `note`: one transform that reads the title the host maps into it.
const NOTE: &str = r#"name = "note"
exit = "fmt"

[[node]]
id = "fmt"
kind = "transform"
expr = "$.input.title"
"#;

/// A host `start -> docs(call note, maps title from $.input.spec) -> done`, pinned to `NOTE`.
const HOST: &str = r#"name = "docs-host"
entry = "start"

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "docs"
kind = "call"
graph = "note"
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

fn hello() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

/// An engine whose graph dir holds `docs-host.toml` and `note.toml`.
fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
    let graphs = dir.join("graphs");
    std::fs::create_dir_all(&graphs).unwrap();
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    let sha = pii_graph::fragment::sha256_hex(NOTE.as_bytes());
    std::fs::write(graphs.join("docs-host.toml"), HOST.replace("SHA", &sha)).unwrap();
    std::fs::write(graphs.join("note.toml"), NOTE).unwrap();
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
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: graphs,
        mqtt_tool_url: None,
    })
    .expect("engine")
}

/// Polls the store until the run reaches a terminal status, then returns its journal.
async fn settle(engine: &Engine, run: &RunId) -> Vec<(u64, RunEvent)> {
    for _ in 0..500 {
        // The header is written by the spawned run thread, so "not yet" is a normal first poll.
        if let Some(header) = engine.store.get_run(run).unwrap()
            && matches!(header.status, RunStatus::Succeeded | RunStatus::Failed)
        {
            return engine.store.read_journal(run).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run {run} did not settle");
}

#[tokio::test]
async fn the_daemon_journals_the_same_event() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_in(tmp.path());

    let run = engine
        .start_run("docs-host", &serde_json::json!({ "spec": "hello" }))
        .expect("a host with a call starts");
    let journal = settle(&engine, &run).await;

    let call = journal.iter().find_map(|(_, ev)| match ev {
        RunEvent::NodeObserved { node, kind, fields } if kind == "graph.call" => {
            Some((node.to_string(), fields.clone()))
        }
        _ => None,
    });
    let (node, fields) = call.expect("the daemon's journal holds a graph.call");
    assert_eq!(node, "docs__in", "the head node carries the call");
    assert_eq!(fields["graph"], "note");
    assert_eq!(
        fields["sha256"],
        pii_graph::fragment::sha256_hex(NOTE.as_bytes()),
        "the full digest the loader verified"
    );
    assert_eq!(fields["exit"], "fmt");
    assert_eq!(fields["input_keys"], serde_json::json!(["title"]));
    assert!(fields.get("needs").is_none(), "needs absent: {fields}");
}

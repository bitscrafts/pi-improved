//! Spec 219 R1/R5: a host graph that attaches a fragment is woven when it is started, the run executes the woven form,
//! and the journal records each woven node with its origin.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::Duration;

use pii_abi::{NodeId, RunEvent, RunId, RunStatus};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// The shipped guard fragment, byte for byte, so the pin is the real one.
const GUARD: &str = include_str!("../../../graphs/fragments/podman-guard.toml");

const HOST: &str = r#"name = "guarded"
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

/// The host with its pin placeholder replaced by the guard's real hash.
fn host_src() -> String {
    HOST.replace("PIN", &pii_graph::fragment::sha256_hex(GUARD.as_bytes()))
}

fn hello() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

/// An engine whose graph dir holds `guarded.toml` and its `fragments/podman-guard.toml`.
fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
    let graphs = dir.join("graphs");
    std::fs::create_dir_all(graphs.join("fragments")).unwrap();
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::write(graphs.join("guarded.toml"), host_src()).unwrap();
    std::fs::write(graphs.join("fragments/podman-guard.toml"), GUARD).unwrap();
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
async fn a_woven_graph_starts_and_journals_its_origin() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_in(tmp.path());

    let run = engine
        .start_run("guarded", &serde_json::json!({ "runtime": "docker" }))
        .expect("a woven graph starts");
    let journal = settle(&engine, &run).await;

    let woven = journal.iter().find_map(|(_, ev)| match ev {
        RunEvent::NodeObserved { node, kind, fields } if kind == "woven" => Some((
            node.clone(),
            fields["woven-from"].as_str().map(str::to_owned),
        )),
        _ => None,
    });
    assert_eq!(
        woven,
        Some((
            NodeId::from("guard.check"),
            Some("podman-guard#check".to_owned())
        )),
        "the woven guard node is journalled with its origin"
    );
    let header = engine.store.get_run(&run).unwrap().unwrap();
    assert_eq!(
        header.status,
        RunStatus::Succeeded,
        "the woven run completes"
    );
}

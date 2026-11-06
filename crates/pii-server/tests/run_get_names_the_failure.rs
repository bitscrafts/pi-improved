//! Spec 293 R1, through the DAEMON: `run.get` on a run that failed on an engine limit carries the reason the journal recorded, and a run that
//! succeeded carries none. The CLI tests in `pii-cli/tests/run_failure_reason.rs` read the store directly, so a daemon that never added the field
//! passed them all (review mutants d1 and d2, 2026-10-27).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// Two transforms with a step limit of one: the second exceeds it and the run fails with the engine's own words.
const STEPS: &str = r#"name = "steps"
entry = "a"
caps = []

[limits]
max_steps = 1

[[node]]
id = "a"
kind = "transform"
value = "x"

[[node]]
id = "b"
kind = "transform"
value = "y"

[[edge]]
from = "a"
port = "ok"
to = "b"
kind = "data"
"#;

/// The same two nodes with the default limits: it succeeds.
const FINE: &str = r#"name = "fine"
entry = "a"
caps = []

[[node]]
id = "a"
kind = "transform"
value = "x"

[[node]]
id = "b"
kind = "transform"
value = "y"

[[edge]]
from = "a"
port = "ok"
to = "b"
kind = "data"
"#;

fn hello() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn engine_in(dir: &Path) -> Arc<Engine> {
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/steps.toml"), STEPS).unwrap();
    std::fs::write(dir.join("graphs/fine.toml"), FINE).unwrap();
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
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine")
}

async fn get(engine: &Arc<Engine>, run: &str) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(engine));
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-293".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _) = pii_proto::client::Client::connect(r, w, info)
        .await
        .expect("connect");
    let out = client
        .call(Method::RunGet, serde_json::json!({ "run_id": run }))
        .await
        .expect("run.get answers");
    client.shutdown();
    out
}

/// Starts `graph` and polls `run.get` until the run is terminal.
async fn finished(engine: &Arc<Engine>, graph: &str) -> serde_json::Value {
    let run = engine
        .start_run(graph, &serde_json::json!({}))
        .expect("the graph starts");
    for _ in 0..500 {
        let h = get(engine, run.as_str()).await;
        if matches!(h["status"].as_str(), Some("failed" | "succeeded")) {
            return h;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{graph} never finished");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_names_the_limit_a_run_failed_on() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "rg-reason-").unwrap();
    let e = engine_in(d.path());
    let h = finished(&e, "steps").await;
    assert_eq!(h["status"], "failed", "{h}");
    assert_eq!(h["reason"], "max_steps exceeded", "{h}");
}

#[tokio::test(flavor = "multi_thread")]
async fn control_a_succeeded_run_carries_no_reason() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "rg-noreason-").unwrap();
    let e = engine_in(d.path());
    let h = finished(&e, "fine").await;
    assert_eq!(h["status"], "succeeded", "{h}");
    assert!(h.get("reason").is_none(), "no reason on success: {h}");
}

/// A run that failed and was then resumed to success holds two terminals; `run.get` carries the LAST one's story, so no reason.
/// (A reader that took the first terminal reported `max_steps exceeded` for a run that finally succeeded.)
#[tokio::test(flavor = "multi_thread")]
async fn a_later_success_hides_the_older_failure_through_the_daemon() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "rg-resumed-").unwrap();
    let e = engine_in(d.path());
    let h = finished(&e, "steps").await;
    assert_eq!(
        h["reason"], "max_steps exceeded",
        "the failure is there first: {h}"
    );
    let run = pii_abi::RunId::from(h["run_id"].as_str().unwrap());
    let mut header = e.store.get_run(&run).unwrap().unwrap();
    header.status = pii_abi::RunStatus::Succeeded;
    e.store
        .append_event(
            &mut header,
            &pii_abi::RunEvent::Finished {
                terminal: pii_abi::Terminal::Success {
                    output: serde_json::json!({}),
                },
                output_hash: String::new(),
            },
        )
        .unwrap();
    let after = get(&e, run.as_str()).await;
    assert_eq!(after["status"], "succeeded", "{after}");
    assert!(
        after.get("reason").is_none(),
        "the old failure is not the reason: {after}"
    );
}

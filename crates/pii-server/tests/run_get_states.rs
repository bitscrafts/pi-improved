//! Spec 115 R1: `run.get` tells "not yet" from "unknown", and a start that was
//! refused after its id was handed out reaches a terminal state instead of
//! polling `NotFound` forever.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// Loads fine, but the agent node's capabilities are not granted by `caps`,
/// so the run's own validation refuses it at start (V2).
const UNGRANTED: &str = r#"name = "ungranted"
entry = "a"
caps = []

[[node]]
id = "a"
kind = "agent"
system = "s"
"#;

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn engine_in(dir: &Path) -> Arc<Engine> {
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/ungranted.toml"), UNGRANTED).unwrap();
    Engine::new(EngineConfig {
        workspace: dir.join("ws"),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        graph_dir: dir.join("graphs"),
    })
    .expect("engine")
}

async fn call(
    engine: &Arc<Engine>,
    method: Method,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(engine));
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-115".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _) = pii_proto::client::Client::connect(r, w, info)
        .await
        .expect("connect");
    let out = client
        .call(method, params)
        .await
        .map_err(|e| format!("{e:?}"));
    client.shutdown();
    out
}

/// A run the broker holds (registered, cancel token live) but whose journal
/// has not materialised answers `accepted`; an id nobody knows is `NotFound`;
/// when the broker lets go of a run that never materialised it is unknown
/// again (nothing lingers as "accepted").
#[tokio::test(flavor = "multi_thread")]
async fn an_in_flight_run_is_accepted_and_an_unknown_id_is_not_found() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "rg-flight-").unwrap();
    let e = engine_in(d.path());
    let rid = pii_abi::RunId::from("r-in-flight");
    e.register_run(&rid, pii_core::CancelToken::new());

    let known = call(
        &e,
        Method::RunGet,
        serde_json::json!({"run_id": "r-in-flight"}),
    )
    .await
    .expect("an in-flight run must answer, not error");
    assert_eq!(known["status"], "accepted", "{known}");
    assert_eq!(known["run_id"], "r-in-flight");

    let unknown = call(&e, Method::RunGet, serde_json::json!({"run_id": "r-nope"}))
        .await
        .unwrap_err();
    assert!(
        unknown.contains("NotFound") || unknown.contains("not found"),
        "{unknown}"
    );

    e.unregister_run("r-in-flight");
    let gone = call(
        &e,
        Method::RunGet,
        serde_json::json!({"run_id": "r-in-flight"}),
    )
    .await
    .unwrap_err();
    assert!(
        gone.contains("NotFound") || gone.contains("not found"),
        "{gone}"
    );
}

/// A start refused by the run's own validation hands back an id (`run.start`
/// returns before execution); that id must reach a terminal `failed` whose
/// trace names the refusal, never `NotFound` forever.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_start_reaches_a_terminal_failed_state() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "rg-refuse-").unwrap();
    let e = engine_in(d.path());
    let started = call(
        &e,
        Method::RunStart,
        serde_json::json!({"graph": "ungranted", "input": {}}),
    )
    .await
    .unwrap();
    let rid = started["run_id"].as_str().unwrap().to_owned();
    let mut last = String::new();
    for _ in 0..100 {
        match call(&e, Method::RunGet, serde_json::json!({"run_id": rid})).await {
            Ok(h) if h["status"] == "failed" => {
                let trace = call(&e, Method::RunTrace, serde_json::json!({"run_id": rid}))
                    .await
                    .unwrap();
                assert!(
                    trace.to_string().contains("V2"),
                    "the trace must name the refusal: {trace}"
                );
                return;
            }
            Ok(h) => last = h.to_string(),
            Err(err) => last = err,
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("refused run never reached a terminal state; last answer: {last}");
}

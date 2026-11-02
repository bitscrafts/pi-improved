//! Spec 115 R3/R4: `sys.status` carries run-lifecycle latency by outcome, the
//! process RSS, and the one-line statement that the store never shrinks.

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

const UNGRANTED: &str = r#"name = "ungranted"
entry = "a"
caps = []

[[node]]
id = "a"
kind = "agent"
system = "s"
"#;

fn engine_in(dir: &Path) -> Arc<Engine> {
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/ungranted.toml"), UNGRANTED).unwrap();
    // Five turns, one per model-calling run below: the engine holds ONE
    // provider for its whole life, and the scripted provider hands out turn n
    // on call n from one shared counter — a one-turn fixture left runs 2..5
    // failing with "scripted provider exhausted" (measured 2026-10-19; the
    // shared fixtures/phase2/hello.json serves other tests one turn each and
    // must not grow).
    let five_turns: Vec<String> = (0..6)
        .map(|_| {
            serde_json::json!({
                "text": "hello from the scripted fixture",
                "tool_calls": [],
                "tokens_in": 3,
                "tokens_out": 5
            })
            .to_string()
        })
        .collect();
    let fixture = dir.join("five-turns.json");
    std::fs::write(
        &fixture,
        format!("{{\"turns\": [{}]}}", five_turns.join(",")),
    )
    .unwrap();
    let hello = fixture;
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.join("ws"),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello.display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .unwrap()
}

async fn call(
    engine: &Arc<Engine>,
    method: Method,
    params: serde_json::Value,
) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(engine));
    let (rd, wr) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-115".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _) = pii_proto::client::Client::connect(rd, wr, info)
        .await
        .unwrap();
    let out = client.call(method, params).await;
    client.shutdown();
    out.unwrap_or_else(|err| serde_json::json!({"error": format!("{err:?}")}))
}

async fn until_terminal(e: &Arc<Engine>, rid: &str) {
    for _ in 0..200 {
        let h = call(e, Method::RunGet, serde_json::json!({"run_id": rid})).await;
        if matches!(h["status"].as_str(), Some("succeeded" | "failed")) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("run {rid} never terminated");
}

fn entry<'a>(st: &'a serde_json::Value, outcome: &str) -> &'a serde_json::Value {
    st["latency"]
        .as_array()
        .unwrap_or_else(|| panic!("no latency array: {st}"))
        .iter()
        .find(|l| l["outcome"] == outcome)
        .unwrap_or_else(|| panic!("no latency entry for {outcome}: {st}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn status_reports_latency_by_outcome_rss_and_the_store_note() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "lat-").unwrap();
    let e = engine_in(d.path());
    for _ in 0..5 {
        let r = call(
            &e,
            Method::RunStart,
            serde_json::json!({"graph": "default", "input": {"prompt": "hi"}}),
        )
        .await;
        until_terminal(&e, r["run_id"].as_str().unwrap()).await;
    }
    let r = call(
        &e,
        Method::RunStart,
        serde_json::json!({"graph": "ungranted", "input": {}}),
    )
    .await;
    until_terminal(&e, r["run_id"].as_str().unwrap()).await;

    let st = call(&e, Method::SysStatus, serde_json::json!({})).await;
    let ok = entry(&st, "succeeded");
    assert_eq!(ok["count"], 5, "{st}");
    let (p50, p95, p99, max) = (
        ok["p50_ms"].as_u64().unwrap(),
        ok["p95_ms"].as_u64().unwrap(),
        ok["p99_ms"].as_u64().unwrap(),
        ok["max_ms"].as_u64().unwrap(),
    );
    assert!(p50 >= 1 && p50 <= p95 && p95 <= p99 && p99 <= max, "{ok}");
    // A run journals ~8 events (each a fsynced commit): it cannot take ~0.
    // This is what a histogram stuck at "1 ms" would fail.
    assert!(max >= 3, "latency looks unmeasured: {ok}");
    assert_eq!(
        entry(&st, "refused")["count"],
        1,
        "the refused run is its own outcome"
    );
    assert_eq!(entry(&st, "failed")["count"], 0);
    assert!(st["rss_kb"].as_u64().unwrap() > 0);
    let notes = st["notes"].to_string();
    assert!(notes.contains("never shrinks"), "{notes}");
}

//! Latency test (N4, part-4 EC4): `session.prompt` returns a `run_id` in
//! < 50 ms even when the run itself takes 2 s (scripted provider sleeps).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use pii_proto::method::Method;
use pii_proto::session::{ContentBlock, SessionCreateParams, SessionPromptParams};
use pii_proto::sys::PeerInfo;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", slow_fixture().display()),
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

fn slow_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("fixtures/phase2/slow.json")
}

/// `session.prompt` returns a `run_id` in < 50 ms; `run_finished` arrives
/// later (after the 2 s provider sleep).
#[tokio::test]
async fn prompt_returns_before_run_finishes() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "latency-").expect("tempdir");
    let engine = engine_in(dir.path());
    let stream = pii_server::engine::loopback::spawn_loopback(engine);
    let (r, w) = tokio::io::split(stream);
    let client_info = PeerInfo {
        name: "test".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _hello) = pii_proto::client::Client::connect(r, w, client_info)
        .await
        .expect("connect");

    let created = client
        .call(
            Method::SessionCreate,
            serde_json::to_value(SessionCreateParams {
                cwd: "/".into(),
                model: None,
                tools: Vec::new(),
                graph: None,
            })
            .expect("params"),
        )
        .await
        .expect("session.create");
    let created: pii_proto::session::SessionCreateResult =
        serde_json::from_value(created).expect("parse session");

    // N4: the prompt returns before the run finishes. A single wall-clock sample of a 50 ms bound measures the
    // scheduler, not the product -- measured 2026-11-17: 1 failure in 3 runs with no load. Five samples, the median
    // is the claim, and a hard ceiling keeps a genuinely blocking implementation failing.
    let mut samples = Vec::new();
    let mut run = serde_json::Value::Null;
    for _ in 0..5 {
        let started = Instant::now();
        run = client
            .call(
                Method::SessionPrompt,
                serde_json::to_value(SessionPromptParams {
                    session_id: created.session_id.clone(),
                    content: vec![ContentBlock::Text {
                        text: "hello".into(),
                    }],
                    input: None,
                })
                .expect("params"),
            )
            .await
            .expect("session.prompt");
        samples.push(started.elapsed().as_millis());
    }
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    let slowest = *samples.last().expect("samples");
    assert!(
        median < 50,
        "session.prompt median {median} ms (must be < 50); samples {samples:?}"
    );
    assert!(
        slowest < 500,
        "session.prompt slowest {slowest} ms (hard ceiling 500); samples {samples:?}"
    );
    assert!(run.get("run_id").is_some(), "prompt must return a run_id");
}

//! Spec 397: a gate declares how long it may wait, and a run suspended past that deadline fails with
//! `wait_deadline_exceeded`, releasing its queue row with the session and attempts intact.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{RunId, RunStatus};
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_store::queue::{ItemStatus, WorkItem};
use pii_tools::sandbox::SandboxProfileName;

fn graph(deadline: Option<u64>) -> String {
    let deadline = deadline.map_or_else(String::new, |d| format!("deadline_ms = {d}\n"));
    format!(
        r#"name = "gated"
entry = "gate"
caps = []

[[node]]
id = "gate"
kind = "gate"
policy = "p"
prompt = "ok?"
{deadline}
[[node]]
id = "done"
kind = "transform"
value = "done"

[[edge]]
from = "gate"
port = "allow"
to = "done"
kind = "data"
"#
    )
}

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn daemon_in(dir: &Path, deadline: Option<u64>) -> Arc<Daemon> {
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/gated.toml"), graph(deadline)).unwrap();
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace,
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
        models: pii_server::daemon::empty_model_roles(),
    })
    .expect("engine");
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, "# spec 397\n").unwrap();
    let mut cfg = Config::load(&cfg_path).expect("config");
    cfg.core.durable_dir = Some(dir.join("durable").display().to_string());
    let d = Daemon::build(engine, &cfg, dir, cfg_path, Arc::new(pii_core::SystemClock))
        .expect("daemon");
    drop(d.spawn_background());
    d
}

async fn call(
    engine: &Arc<Engine>,
    method: Method,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(engine));
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-397".into(),
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

async fn start(e: &Arc<Engine>) -> Result<String, String> {
    let v = call(
        e,
        Method::RunStart,
        serde_json::json!({"graph": "gated", "input": {}}),
    )
    .await?;
    Ok(v["run_id"].as_str().unwrap().to_owned())
}

async fn wait_status(e: &Arc<Engine>, run: &str, want: RunStatus, ms: u64) -> Option<RunStatus> {
    let mut last = None;
    for _ in 0..(ms / 25) {
        last = e
            .store
            .get_run(&RunId::from(run))
            .unwrap()
            .map(|h| h.status);
        if last == Some(want) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    last
}

fn journal(e: &Arc<Engine>, run: &str) -> String {
    format!("{:?}", e.store.read_journal(&RunId::from(run)).unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gate_without_a_deadline_is_refused() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "gd-none-").unwrap();
    let d = daemon_in(tmp.path(), None);
    let err = start(&d.engine)
        .await
        .expect_err("a gate with no deadline must be refused");
    assert!(
        err.contains("gate"),
        "the refusal must name the node: {err}"
    );
    assert!(
        err.contains("deadline_ms"),
        "the refusal must name the key: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gate_with_a_deadline_loads() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "gd-ok-").unwrap();
    let d = daemon_in(tmp.path(), Some(60_000));
    start(&d.engine)
        .await
        .expect("a gate with a deadline loads");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_suspended_past_its_deadline_fails_and_names_it() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "gd-late-").unwrap();
    let d = daemon_in(tmp.path(), Some(50));
    let run = start(&d.engine).await.expect("start");
    let got = wait_status(&d.engine, &run, RunStatus::Failed, 5_000).await;
    assert_eq!(got, Some(RunStatus::Failed), "must fail past its deadline");
    let j = journal(&d.engine, &run);
    assert!(j.contains("wait_deadline_exceeded"), "reason missing: {j}");
    assert!(j.contains("gate"), "node must be named: {j}");
    assert!(j.contains("50"), "deadline must be named: {j}");
}

#[tokio::test(flavor = "multi_thread")]
async fn control_a_run_approved_inside_the_deadline_resumes() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "gd-ctl-").unwrap();
    let d = daemon_in(tmp.path(), Some(5_000));
    let run = start(&d.engine).await.expect("start");
    let mut pending = Vec::new();
    for _ in 0..100 {
        pending = d.engine.approvals.pending_approvals().unwrap();
        if !pending.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(pending.len(), 1, "the gate must be waiting");
    call(
        &d.engine,
        Method::ApproveResolve,
        serde_json::json!({"id": pending[0].approval_id, "allow": true}),
    )
    .await
    .expect("approve");
    let got = wait_status(&d.engine, &run, RunStatus::Succeeded, 5_000).await;
    assert_eq!(got, Some(RunStatus::Succeeded));
    assert!(!journal(&d.engine, &run).contains("wait_deadline_exceeded"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_row_returns_to_the_queue_with_its_session() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "gd-row-").unwrap();
    let d = daemon_in(tmp.path(), Some(50));
    let run = start(&d.engine).await.expect("start");
    let mut row = WorkItem::new("row-397", &[]);
    row.status = ItemStatus::Running;
    row.attempts = 2;
    row.session = Some(String::from("sess-keep"));
    row.claimed_by = Some(run.clone());
    d.engine.queue.put_item(&row).unwrap();

    let got = wait_status(&d.engine, &run, RunStatus::Failed, 5_000).await;
    assert_eq!(got, Some(RunStatus::Failed));
    let mut after = None;
    for _ in 0..100 {
        after = d.engine.queue.get_item("row-397").unwrap();
        if after
            .as_ref()
            .is_some_and(|i| i.status == ItemStatus::Pending)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let after = after.expect("row");
    assert_eq!(after.status, ItemStatus::Pending);
    assert_eq!(after.session.as_deref(), Some("sess-keep"));
    assert_eq!(after.attempts, 2);
    assert!(
        after
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("wait_deadline_exceeded")),
        "reason recorded: {:?}",
        after.reason
    );
}

//! Spec 106: the operator surfaces must not lie — `run.list` lists every run
//! and refuses an unknown status, and a suspended gate yields a visible,
//! run-scoped pending approval.

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
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const GATED: &str = r#"name = "gated"
entry = "gate"
caps = []

[[node]]
id = "gate"
kind = "gate"
policy = "p"
prompt = "ok?"

[[node]]
id = "done"
kind = "transform"
value = "done"

[[edge]]
from = "gate"
port = "allow"
to = "done"
kind = "data"
"#;

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn scratch(prefix: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).expect("tempdir")
}

fn daemon_in(dir: &Path) -> Arc<Daemon> {
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/gated.toml"), GATED).unwrap();
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
    })
    .expect("engine");
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, "# spec 106\n").unwrap();
    let cfg = Config::load(&cfg_path).expect("config");
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
        name: "spec-106".into(),
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

fn engine_of(d: &Arc<Daemon>) -> Arc<Engine> {
    Arc::clone(&d.engine)
}

async fn wait_pending(engine: &Arc<Engine>, n: usize) -> Vec<pii_store::approval::Approval> {
    for _ in 0..100 {
        let p = engine.approvals.pending_approvals().unwrap();
        if p.len() >= n {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    engine.approvals.pending_approvals().unwrap()
}

/// R1: `run.list` with no status returns every run, whatever its status.
#[tokio::test(flavor = "multi_thread")]
async fn run_list_without_status_returns_every_run() {
    let d = scratch("ops-list-");
    let daemon = daemon_in(d.path());
    let e = engine_of(&daemon);
    for g in ["default", "default", "gated"] {
        call(
            &e,
            Method::RunStart,
            serde_json::json!({"graph": g, "input": {"prompt": "hi"}}),
        )
        .await
        .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let all = call(&e, Method::RunList, serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(all.as_array().unwrap().len(), 3, "all runs listed: {all}");
    let susp = call(
        &e,
        Method::RunList,
        serde_json::json!({"status": "suspended"}),
    )
    .await
    .unwrap();
    assert_eq!(susp.as_array().unwrap().len(), 1, "filter still works");
}

/// R2: a status the wire cannot express is an error naming the accepted values.
#[tokio::test(flavor = "multi_thread")]
async fn run_list_unknown_status_is_an_error() {
    let d = scratch("ops-bogus-");
    let daemon = daemon_in(d.path());
    let e = engine_of(&daemon);
    let err = call(&e, Method::RunList, serde_json::json!({"status": "bogus"}))
        .await
        .unwrap_err();
    for v in ["running", "suspended", "succeeded", "failed"] {
        assert!(err.contains(v), "error must name '{v}': {err}");
    }
}

/// R3: a suspended gate is a visible pending approval, and two runs on the
/// same policy produce two distinct approval rows.
#[tokio::test(flavor = "multi_thread")]
async fn suspended_runs_have_distinct_visible_approvals() {
    let d = scratch("ops-appr-");
    let daemon = daemon_in(d.path());
    let e = engine_of(&daemon);
    let mut runs = Vec::new();
    for _ in 0..2 {
        let v = call(
            &e,
            Method::RunStart,
            serde_json::json!({"graph": "gated", "input": {}}),
        )
        .await
        .unwrap();
        runs.push(v["run_id"].as_str().unwrap().to_owned());
    }
    let pending = wait_pending(&e, 2).await;
    assert_eq!(pending.len(), 2, "two runs, two approvals: {pending:?}");
    assert_ne!(pending[0].approval_id, pending[1].approval_id);
    let mut got: Vec<_> = pending.iter().map(|a| a.run_id.clone()).collect();
    got.sort();
    runs.sort();
    assert_eq!(got, runs, "each approval is scoped to its own run");
    let st = call(&e, Method::SysStatus, serde_json::json!({}))
        .await
        .unwrap();
    assert!(st["counts"]["approvals_pending"].as_u64().unwrap() >= 2);
}

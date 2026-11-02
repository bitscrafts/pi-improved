//! Spec 337: the durable stores (channel cursor, timer wheel) live under the
//! state dir, not the per-boot runtime dir (a tmpfs on the host).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    // reason: the restart test re-runs this test binary as a child process, the only way to
    // release the store locks between two daemon starts.
    clippy::disallowed_methods
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_proto::method::Method;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// A scratch root under the crate's `_tmp`, so nothing lands in the real `$HOME` or `/run`.
fn scratch(prefix: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).unwrap()
}

fn fixtures_hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn engine_in(dir: &Path) -> Arc<Engine> {
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace,
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixtures_hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine")
}

/// Starts a daemon-shaped process: the config at `config_text`, the runtime dir `runtime`, and a
/// `$HOME` of `home` (so the installer's state dir is `home/.local/state/pii`).
fn start(home: &Path, runtime: &Path, config_text: &str) -> (Arc<Engine>, Arc<Daemon>) {
    std::fs::create_dir_all(runtime).unwrap();
    std::fs::create_dir_all(home).unwrap();
    let cfg_path = home.join("pii.toml");
    std::fs::write(&cfg_path, config_text).unwrap();
    let cfg = Config::load(&cfg_path).expect("config");
    let engine = engine_in(home);
    let home_str = home.display().to_string();
    let env = move |k: &str| (k == "HOME").then(|| home_str.clone());
    let daemon = Daemon::build_with_env(
        Arc::clone(&engine),
        &cfg,
        runtime,
        cfg_path,
        Arc::new(pii_core::SystemClock),
        &env,
    )
    .expect("daemon");
    (engine, daemon)
}

async fn stores_status(engine: Arc<Engine>) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(engine);
    let (r, w) = tokio::io::split(stream);
    let info = pii_proto::sys::PeerInfo {
        name: "durable-store-placement-test".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _hello) = pii_proto::client::Client::connect(r, w, info)
        .await
        .expect("connect");
    let v = client
        .call(Method::SysStatus, serde_json::json!({}))
        .await
        .expect("sys.status");
    client.shutdown();
    v
}

/// The resolved path of the named store as `sys.status` reports it.
fn store_path(v: &serde_json::Value, name: &str) -> PathBuf {
    let s = v["stores"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("no store {name} in {}", v["stores"]));
    PathBuf::from(s["path"].as_str().unwrap())
}

fn state_dir(home: &Path) -> PathBuf {
    home.join(".local/state/pii")
}

#[tokio::test]
async fn the_channel_store_is_not_under_the_runtime_dir() {
    let t = scratch("dsp-chan-");
    let home = t.path().join("home");
    let runtime = t.path().join("run/pii");
    let (engine, _d) = start(&home, &runtime, "# default\n");
    let v = stores_status(engine).await;
    let p = store_path(&v, "channel");
    assert!(
        p.starts_with(state_dir(&home)),
        "channel store at {}",
        p.display()
    );
    assert!(
        !p.starts_with(&runtime),
        "channel store under runtime: {}",
        p.display()
    );
}

#[tokio::test]
async fn the_timer_store_is_not_under_the_runtime_dir() {
    let t = scratch("dsp-timer-");
    let home = t.path().join("home");
    let runtime = t.path().join("run/pii");
    let (engine, _d) = start(&home, &runtime, "# default\n");
    let v = stores_status(engine).await;
    let p = store_path(&v, "timer");
    assert!(
        p.starts_with(state_dir(&home)),
        "timer store at {}",
        p.display()
    );
    assert!(
        !p.starts_with(&runtime),
        "timer store under runtime: {}",
        p.display()
    );
}

const CHILD_PHASE: &str = "PII_DSP_CHILD_PHASE";

/// The daemon-shaped process body. Only runs when re-invoked by [`run_child`]: a restart is a new
/// process, because the engine holds itself and so never releases its store locks in-process.
#[tokio::test]
async fn daemon_child() {
    let Ok(phase) = std::env::var(CHILD_PHASE) else {
        return;
    };
    let home = PathBuf::from(std::env::var("PII_DSP_HOME").unwrap());
    let runtime = PathBuf::from(std::env::var("PII_DSP_RUNTIME").unwrap());
    let (_engine, d) = start(&home, &runtime, "# default\n");
    if phase == "write" {
        d.ctx.channel_store.advance("chan", 42, "run-1").unwrap();
    } else {
        let cur = d
            .ctx
            .channel_store
            .get("chan")
            .unwrap()
            .expect("cursor kept");
        assert_eq!(cur.cursor, 42, "no message may be re-processed");
        assert_eq!(cur.run_id, "run-1");
    }
}

fn run_child(home: &Path, runtime: &Path, phase: &str) {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "daemon_child", "--nocapture", "--test-threads=1"])
        .env(CHILD_PHASE, phase)
        .env("PII_DSP_HOME", home)
        .env("PII_DSP_RUNTIME", runtime)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{phase} child failed:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// The property, not the path: a cursor written by one process is the same when a second process
/// starts, even when the runtime dir is gone (a reboot on tmpfs).
#[test]
fn a_restart_keeps_the_cursor() {
    let t = scratch("dsp-restart-");
    let home = t.path().join("home");
    let runtime = t.path().join("run/pii");
    run_child(&home, &runtime, "write");
    std::fs::remove_dir_all(&runtime).unwrap();
    run_child(&home, &runtime, "read");
}

/// Control: the old placement (a store under the runtime dir) loses the cursor across the same
/// removal. This is the defect, measured on the host in spec 337.
#[test]
fn control_the_old_placement_loses_the_cursor() {
    let t = scratch("dsp-control-");
    let runtime = t.path().join("run/pii");
    std::fs::create_dir_all(&runtime).unwrap();
    let store = pii_store::channel::ChannelStore::open(&runtime.join("channel.redb")).unwrap();
    store.advance("chan", 42, "run-1").unwrap();
    drop(store);
    std::fs::remove_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&runtime).unwrap();

    let store = pii_store::channel::ChannelStore::open(&runtime.join("channel.redb")).unwrap();
    assert!(
        store.get("chan").unwrap().is_none(),
        "the old placement must lose the cursor"
    );
}

#[tokio::test]
async fn durable_dir_overrides_both_stores() {
    let t = scratch("dsp-override-");
    let home = t.path().join("home");
    let runtime = t.path().join("run/pii");
    let elsewhere = t.path().join("elsewhere");
    let cfg = format!("[core]\ndurable_dir = \"{}\"\n", elsewhere.display());
    let (engine, _d) = start(&home, &runtime, &cfg);
    let v = stores_status(engine).await;
    assert_eq!(store_path(&v, "channel"), elsewhere.join("channel.redb"));
    assert_eq!(store_path(&v, "timer"), elsewhere.join("timer.redb"));
}

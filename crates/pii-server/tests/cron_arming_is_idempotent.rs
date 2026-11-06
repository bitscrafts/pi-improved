//! Spec 187: a daemon start arms a cron entry once. Every restart armed another copy and all
//! copies fired at the occurrence (1,846 runs from one 09:00). Scratch store, never the live one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_observe::{Recorder, names};
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const DAILY: &str = "name = \"daily\"\nentry = \"tick\"\n\
[[node]]\nid = \"tick\"\nkind = \"timer\"\ncron = \"0 9 * * *\"\n";

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

/// Builds a daemon over `engine` (a "restart" is a fresh daemon context on the same store).
fn boot(engine: &Arc<Engine>, dir: &Path, n: usize) -> Arc<Daemon> {
    let rt = dir.join(format!("rt{n}"));
    std::fs::create_dir_all(&rt).unwrap();
    let cfg_path = dir.join("pii.toml");
    let mut cfg = Config::load(&cfg_path).expect("config");
    // Per boot, as the runtime dir is: an in-process restart cannot reopen a store still held open.
    cfg.core.durable_dir = Some(dir.join(format!("durable{n}")).display().to_string());
    Daemon::build(
        Arc::clone(engine),
        &cfg,
        &rt,
        cfg_path,
        Arc::new(pii_core::SystemClock),
    )
    .expect("daemon")
}

fn suspended(d: &Arc<Daemon>) -> usize {
    d.engine
        .store
        .runs_by_status(pii_abi::RunStatus::Suspended)
        .unwrap()
        .len()
}

#[tokio::test(flavor = "multi_thread")]
async fn n_restarts_leave_one_armed_run_per_cron_entry() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "arming").unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/daily.toml"), DAILY).unwrap();
    std::fs::write(
        dir.join("pii.toml"),
        "[[cron]]\nname = \"d\"\ngraph = \"daily\"\n",
    )
    .unwrap();
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
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
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine");
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    for n in 0..4 {
        let d = boot(&engine, dir, n);

        d.start_cron_entries();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let d = boot(&engine, dir, 4);
    assert_eq!(suspended(&d), 1, "four starts armed more than one run");
    assert_eq!(
        log.events_named(names::CRON_REARMED).len(),
        3,
        "each restart that kept the armed run must say so"
    );
}

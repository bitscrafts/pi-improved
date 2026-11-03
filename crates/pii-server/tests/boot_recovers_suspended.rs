//! Spec 353 R3: a run left `Suspended` with no timer row is recovered at boot by `Daemon::build`, with no bus
//! `Lagged` event: its timer is armed again, and a resume that is refused is journalled.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{NodeId, ResumeKey, RunEvent, RunHeader, RunId, RunStatus, SuspendReason};
use pii_core::SystemClock;
use pii_observe::Recorder;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn engine_in(dir: &Path) -> Arc<Engine> {
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.to_path_buf(),
        mqtt_tool_url: None,
    })
    .expect("engine")
}

#[tokio::test]
async fn a_suspended_run_with_no_timer_is_recovered_at_boot() {
    let tmp =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "boot-suspended").unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, "").unwrap();
    let mut cfg = Config::load(&cfg_path).expect("config");
    cfg.core.durable_dir = Some(dir.join("durable").display().to_string());

    // The crash left the run Suspended on a timer that was due at 0 ms, and no timer row for it.
    let engine = engine_in(dir);
    let mut header = RunHeader {
        run: RunId::from("r-boot"),
        graph: String::from("ghost"),
        status: RunStatus::Suspended,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    };
    engine
        .store
        .append_event(
            &mut header,
            &RunEvent::Suspended {
                node: NodeId::from("tick"),
                reason: SuspendReason::Timer { in_ms: 0 },
                key: ResumeKey(String::from("timer:r-boot:t:0")),
            },
        )
        .unwrap();
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());

    let d = Daemon::build(engine, &cfg, dir, cfg_path, Arc::new(SystemClock)).expect("daemon");
    let handles = d.spawn_background();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let left_suspended = d
        .engine
        .store
        .get_run(&RunId::from("r-boot"))
        .unwrap()
        .is_some_and(|h| h.status == RunStatus::Suspended);
    let armed = d.ctx.timer_store.next_timer().unwrap().is_some();
    let journalled = !log
        .events_named(pii_observe::names::RUN_RESUME_FAILED)
        .is_empty();
    for h in handles {
        h.abort();
    }

    assert!(
        !left_suspended || armed,
        "a suspended run with no timer must be re-armed at boot"
    );
    assert!(
        !left_suspended || journalled,
        "a suspended run recovered at boot must leave an event"
    );
}

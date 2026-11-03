//! Spec 353 R2: a due timer whose resume is refused is not lost. The run is still `Suspended`, so its timer
//! comes back and the refusal is journalled as `run.resume.failed`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{NodeId, ResumeKey, RunEvent, RunHeader, RunId, RunStatus, SuspendReason};
use pii_core::SystemClock;
use pii_observe::Recorder;
use pii_server::daemon::scheduler::Scheduler;
use pii_server::{Engine, EngineConfig};
use pii_store::timer::TimerWheel;
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
async fn a_failed_resume_does_not_lose_the_run() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "keeps-timer").unwrap();
    let dir = tmp.path();
    let engine = engine_in(dir);
    // A suspended run whose graph was never stored: every resume of it is refused.
    let mut header = RunHeader {
        run: RunId::from("r-lost"),
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
                key: ResumeKey(String::from("timer:r-lost:t:0")),
            },
        )
        .unwrap();
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let wheel = Arc::new(TimerWheel::open(&dir.join("timer.redb")).unwrap());
    wheel.put_timer(0, "r-lost", "timer:r-lost:t:0").unwrap();
    let sched = Scheduler::new(engine, wheel.clone(), Arc::new(SystemClock));
    let handle = tokio::spawn(Arc::clone(&sched).run());

    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.abort();

    assert!(
        wheel.next_timer().unwrap().is_some(),
        "a refused resume of a suspended run must put its timer back"
    );
    assert!(
        !log.events_named(pii_observe::names::RUN_RESUME_FAILED)
            .is_empty(),
        "the refused resume must be journalled as run.resume.failed"
    );
}

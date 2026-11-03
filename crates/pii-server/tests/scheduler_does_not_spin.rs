//! Spec 353 R1: a timer-wheel read error is reported once and backed off, so the scheduler never busy-loops.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pii_abi::PiiError;
use pii_core::SystemClock;
use pii_observe::Recorder;
use pii_server::daemon::scheduler::{Scheduler, TimerSource};
use pii_server::{Engine, EngineConfig};
use pii_store::timer::{DueTimer, TimerWheel};
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

/// A wheel whose every read fails, counting the reads it is asked for.
#[derive(Default)]
struct BrokenWheel {
    polls: AtomicU64,
}

impl TimerSource for BrokenWheel {
    fn next_timer(&self) -> Result<Option<u64>, PiiError> {
        self.polls.fetch_add(1, Ordering::Relaxed);
        Err(PiiError::Internal(String::from("wheel unreadable")))
    }

    fn take_due(&self, _now_ms: u64) -> Result<Vec<DueTimer>, PiiError> {
        Ok(Vec::new())
    }

    fn put_timer(&self, _fire_at_ms: u64, _run: &str, _key: &str) -> Result<(), PiiError> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn an_error_does_not_busy_loop() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "spin").unwrap();
    let engine = engine_in(tmp.path());
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let wheel = Arc::new(BrokenWheel::default());
    let sched = Scheduler::new(engine, wheel.clone(), Arc::new(SystemClock));
    let handle = tokio::spawn(Arc::clone(&sched).run());

    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    handle.abort();

    let polls = wheel.polls.load(Ordering::Relaxed);
    assert!(
        polls <= 2,
        "a failing wheel was polled {polls} times in 1 s"
    );
    let errors = log
        .events_named(pii_observe::names::TIMER_WHEEL_ERROR)
        .len();
    assert_eq!(
        errors, 1,
        "timer.wheel.error must appear once, not {errors} times"
    );
}

#[tokio::test]
#[allow(non_snake_case)] // the name states what it controls for
async fn CONTROL_a_normal_timer_still_fires() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "spin-control").unwrap();
    let engine = engine_in(tmp.path());
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let wheel = Arc::new(TimerWheel::open(&tmp.path().join("timer.redb")).unwrap());
    let sched = Scheduler::new(engine, wheel.clone(), Arc::new(SystemClock));
    let fire_at = pii_core::Clock::now_ms(&SystemClock) + 1;
    sched.arm(fire_at, "r-control", "timer:r-control:t:1");
    let handle = tokio::spawn(Arc::clone(&sched).run());

    tokio::time::sleep(Duration::from_millis(200)).await;
    handle.abort();
    // Each firing of the timer resumes its run, which is refused here (no such run): one refusal per firing.
    let fired = log
        .events_named(pii_observe::names::TIMER_RESUME_REFUSED)
        .len();
    assert_eq!(
        fired, 1,
        "a 1 ms timer must fire exactly once, not {fired} times"
    );
    assert_eq!(
        wheel.next_timer().unwrap(),
        None,
        "a fired timer leaves no row"
    );
}

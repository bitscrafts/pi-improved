//! Spec 196: a cancelled run's timer does not fire it. Measured on the deployed daemon: four runs
//! cancelled the day before executed and posted when their wheel entries came due.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::{ResumeKey, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_core::FixedClock;
use pii_observe::Recorder;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::daemon::scheduler::Scheduler;
use pii_server::{Engine, EngineConfig};
use pii_store::timer::TimerWheel;
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

fn engine_in(dir: &Path) -> Arc<Engine> {
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/daily.toml"), DAILY).unwrap();
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    Engine::new(EngineConfig {
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
    })
    .expect("engine")
}

fn daemon_in(dir: &Path, engine: Arc<Engine>) -> Arc<Daemon> {
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, "[[cron]]\nname = \"d\"\ngraph = \"daily\"\n").unwrap();
    let cfg = Config::load(&cfg_path).expect("config");
    Daemon::build(engine, &cfg, dir, cfg_path, Arc::new(pii_core::SystemClock)).expect("daemon")
}

fn header(run: &str, status: RunStatus) -> RunHeader {
    RunHeader {
        run: RunId::from(run),
        graph: String::from("daily"),
        status,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    }
}

/// A run the store shows `Suspended` on a timer key, as the daemon leaves one.
fn suspended_run(engine: &Engine, run: &str, key: &str) {
    let mut h = header(run, RunStatus::Suspended);
    engine
        .store
        .append_event(
            &mut h,
            &RunEvent::Suspended {
                node: pii_abi::NodeId::from("tick"),
                reason: pii_abi::SuspendReason::Timer { in_ms: 100 },
                key: ResumeKey(String::from(key)),
            },
        )
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_run_s_timer_does_not_resurrect_it() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cancel-disarm").unwrap();
    let engine = engine_in(tmp.path());
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let d = daemon_in(tmp.path(), Arc::clone(&engine));
    suspended_run(&engine, "r-cancel", "timer:r-cancel:tick:100");
    d.ctx
        .timer_store
        .put_timer(100, "r-cancel", "timer:r-cancel:tick:100")
        .unwrap();
    d.ctx
        .timer_store
        .put_timer(100, "r-other", "timer:r-other:tick:100")
        .unwrap();

    pii_server::engine::runs::cancel_run(&engine, &RunId::from("r-cancel"), "stale duplicate")
        .unwrap();

    let due = d.ctx.timer_store.take_due(1_000).unwrap();
    assert_eq!(
        due.iter().map(|t| t.run.as_str()).collect::<Vec<_>>(),
        vec!["r-other"],
        "the cancelled run's entry must be gone, another run's intact"
    );
    let ev = log.events_named(pii_observe::names::RUN_CANCELLED);
    assert_eq!(ev.len(), 1);
    assert_eq!(
        ev.first().unwrap().fields.get("disarmed"),
        Some(&serde_json::json!(1))
    );
    let h = engine
        .store
        .get_run(&RunId::from("r-cancel"))
        .unwrap()
        .unwrap();
    assert_eq!(h.status, RunStatus::Failed, "the run stays cancelled");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_suspended_run_still_fires_when_not_cancelled() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cancel-ctl").unwrap();
    let engine = engine_in(tmp.path());
    let d = daemon_in(tmp.path(), Arc::clone(&engine));
    let handles = d.spawn_background();
    d.start_cron_entries();

    // The cron graph parks on its daily timer; the daemon arms the wheel.
    let end = Instant::now() + Duration::from_secs(10);
    let run = loop {
        let suspended = engine.store.runs_by_status(RunStatus::Suspended).unwrap();
        if let (Some(r), Some(_)) = (suspended.first(), d.ctx.timer_store.next_timer().unwrap()) {
            break r.clone();
        }
        assert!(Instant::now() < end, "the run never parked on its timer");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let before = engine.store.get_run(&run).unwrap().unwrap().seq;
    // Bring its instant forward to now: the scheduler must resume it.
    let entry = d.ctx.timer_store.take_due(u64::MAX).unwrap();
    let entry = entry.iter().find(|t| t.run == run.as_str()).unwrap();
    d.scheduler.arm(1, &entry.run, &entry.key);
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if engine.store.get_run(&run).unwrap().unwrap().seq > before {
            break;
        }
        assert!(
            Instant::now() < end,
            "an uncancelled due timer did not resume its run"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for h in handles {
        h.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_due_timer_for_a_finished_run_is_refused_loudly() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cancel-refuse").unwrap();
    let engine = engine_in(tmp.path());
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let mut h = header("r-done", RunStatus::Failed);
    let terminal = Terminal::Failure {
        error: String::from("cancelled: stale duplicate"),
    };
    engine
        .store
        .append_event(
            &mut h,
            &RunEvent::Finished {
                terminal,
                output_hash: String::new(),
            },
        )
        .unwrap();
    let seq_before = engine
        .store
        .get_run(&RunId::from("r-done"))
        .unwrap()
        .unwrap()
        .seq;
    let wheel = Arc::new(TimerWheel::open(&tmp.path().join("timer.redb")).unwrap());
    let sched = Scheduler::new(Arc::clone(&engine), wheel, Arc::new(FixedClock::new()));
    sched.arm(1, "r-done", "timer:r-done:tick:1");
    let handle = tokio::spawn(Arc::clone(&sched).run());

    let end = Instant::now() + Duration::from_secs(10);
    let refused = loop {
        let ev = log.events_named(pii_observe::names::RUN_RESUME_REFUSED);
        if let Some(e) = ev.into_iter().next() {
            break e;
        }
        assert!(
            Instant::now() < end,
            "no run.resume.refused for a finished run"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    handle.abort();
    assert_eq!(
        refused.fields.get("run"),
        Some(&serde_json::json!("r-done"))
    );
    let after = engine
        .store
        .get_run(&RunId::from("r-done"))
        .unwrap()
        .unwrap();
    assert_eq!(after.status, RunStatus::Failed);
    assert_eq!(after.seq, seq_before, "nothing executed");
}

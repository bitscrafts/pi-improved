//! Spec 243: `run.cancel` with `kill` stops a running node's agent and closes the run. Without the flag the refusal
//! stands. The agent is a real `/bin/sh` process group that starts a `sleep` child; a Drop guard kills the group so a
//! failed assertion leaves nothing behind.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pii_abi::{CapKind, NodeId, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_core::{ArgvScope, Budget, CancelToken, CapSet, Ctx, TraceCtx};
use pii_observe::Recorder;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::engine::runs::{cancel_run, cancel_run_with};
use pii_server::node_kinds::nodes::coder_runtime::wait_child;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const WAIT_30S: Duration = Duration::from_secs(30);

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn engine_in(dir: &Path) -> Arc<Engine> {
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
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
    })
    .expect("engine")
}

fn header(run: &str, status: RunStatus) -> RunHeader {
    RunHeader {
        run: RunId::from(run),
        graph: String::from("agent"),
        status,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    }
}

fn suspended_run(engine: &Engine, run: &str) {
    let mut h = header(run, RunStatus::Suspended);
    engine
        .store
        .append_event(
            &mut h,
            &RunEvent::Suspended {
                node: NodeId::from("wait"),
                reason: pii_abi::SuspendReason::Timer { in_ms: 3_600_000 },
                key: pii_abi::ResumeKey(String::from("k")),
            },
        )
        .unwrap();
}

fn finished_run(engine: &Engine, run: &str) {
    let mut h = header(run, RunStatus::Failed);
    engine
        .store
        .append_event(
            &mut h,
            &RunEvent::Finished {
                terminal: Terminal::Failure {
                    error: String::from("done earlier"),
                },
                output_hash: String::new(),
            },
        )
        .unwrap();
}

/// Kills the process groups of the fake agents when the test ends, pass or fail.
struct Guard {
    leaders: Vec<u32>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        for pid in &self.leaders {
            let _ = pii_core::kill_group(*pid);
        }
    }
}

/// True once a pid is gone: no `/proc` entry, or a zombie the init process has not reaped yet.
fn gone(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |s| {
        s.rsplit(')')
            .next()
            .is_some_and(|rest| rest.trim_start().starts_with('Z'))
    })
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// A run in flight on node `agent`. Its agent is a `/bin/sh` group running `sleep`. `honour` makes the agent observe the
/// run's registered cancel token; without it the agent ignores cancellation and runs for `script`'s own duration.
struct InFlight {
    run: RunId,
    leader_file: PathBuf,
    child_file: PathBuf,
    thread: JoinHandle<()>,
}

fn in_flight(engine: &Arc<Engine>, dir: &Path, id: &str, honour: bool) -> InFlight {
    let mut h = header(id, RunStatus::Running);
    engine
        .store
        .append_event(
            &mut h,
            &RunEvent::NodeStarted {
                node: NodeId::from("agent"),
                at_ms: 0,
            },
        )
        .unwrap();
    let registered = CancelToken::new();
    engine.register_run(&RunId::from(id), registered.clone());
    let observed = if honour {
        registered
    } else {
        CancelToken::new()
    };
    let leader_file = dir.join(format!("{id}.leader"));
    let child_file = dir.join(format!("{id}.child"));
    let script = if honour {
        format!(
            "echo $$ > {}; sleep 100000 & echo $! > {}; wait",
            leader_file.display(),
            child_file.display()
        )
    } else {
        String::from("sleep 1.5")
    };
    let engine_for_thread = Arc::clone(engine);
    let run_id = id.to_owned();
    let thread = std::thread::spawn(move || {
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::SPAWN | CapKind::PROC;
        caps.proc_argv0 = ArgvScope::new(vec![String::from("/bin/sh")]);
        let ctx = Ctx::with_observer_and_cancel(
            caps,
            Budget::unlimited(),
            Arc::new(pii_core::NullObserver),
            TraceCtx {
                run: RunId::from(run_id.as_str()),
                span: 0,
            },
            observed,
        );
        let argv = vec![String::from("/bin/sh"), String::from("-c"), script];
        let _ = wait_child(&ctx, &argv, Path::new("."), &[], None, &mut |_| {});
        engine_for_thread.unregister_run(&run_id);
    });
    InFlight {
        run: RunId::from(id),
        leader_file,
        child_file,
        thread,
    }
}

/// Waits until the fake agent has started, and returns its leader and child pids.
fn agent_pids(f: &InFlight) -> (u32, u32) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let (Some(l), Some(c)) = (read_pid(&f.leader_file), read_pid(&f.child_file)) {
            return (l, c);
        }
        assert!(Instant::now() < end, "the fake agent never started");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn status_of(engine: &Engine, run: &RunId) -> RunStatus {
    engine.store.get_run(run).unwrap().unwrap().status
}

fn last_failure(engine: &Engine, run: &RunId) -> Option<String> {
    engine
        .store
        .read_journal(run)
        .unwrap()
        .iter()
        .rev()
        .find_map(|(_, e)| match e {
            RunEvent::Finished {
                terminal: Terminal::Failure { error },
                ..
            } => Some(error.clone()),
            _ => None,
        })
}

#[test]
fn cancel_without_kill_is_still_refused_on_a_running_node() {
    let tmp =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "kill-243-refuse").unwrap();
    let engine = engine_in(tmp.path());
    let f = in_flight(&engine, tmp.path(), "r-refuse", true);
    let (leader, child) = agent_pids(&f);
    let _guard = Guard {
        leaders: vec![leader],
    };

    let err = cancel_run(&engine, &f.run, "no kill").unwrap_err();
    assert!(err.to_string().contains("cancel refused"), "{err}");
    assert!(
        !gone(leader) && !gone(child),
        "a refused cancel stops nothing"
    );
    assert_eq!(status_of(&engine, &f.run), RunStatus::Running);

    engine.cancel(f.run.as_str());
    f.thread.join().unwrap();
}

#[test]
fn cancel_with_kill_stops_a_hung_agent_and_closes_the_run() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "kill-243-stop").unwrap();
    let engine = engine_in(tmp.path());
    let f = in_flight(&engine, tmp.path(), "r-stop", true);
    let (leader, child) = agent_pids(&f);
    let _guard = Guard {
        leaders: vec![leader],
    };

    cancel_run_with(&engine, &f.run, "stuck", true, WAIT_30S).unwrap();
    f.thread.join().unwrap();

    assert_eq!(status_of(&engine, &f.run), RunStatus::Failed);
    let error = last_failure(&engine, &f.run).expect("a failure terminal");
    assert!(error.starts_with("cancelled:"), "{error}");
    assert!(error.contains("abandoned"), "{error}");

    let end = Instant::now() + Duration::from_secs(2);
    while !(gone(leader) && gone(child)) {
        assert!(
            Instant::now() < end,
            "the agent and its child outlived the kill"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_run_that_will_not_stop_is_not_closed() {
    let tmp =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "kill-243-stuck").unwrap();
    let engine = engine_in(tmp.path());
    let f = in_flight(&engine, tmp.path(), "r-stuck", false);

    let err =
        cancel_run_with(&engine, &f.run, "stuck", true, Duration::from_millis(300)).unwrap_err();
    assert!(err.to_string().contains("did not stop"), "{err}");
    assert_eq!(
        status_of(&engine, &f.run),
        RunStatus::Running,
        "a run whose agent may still run is never marked closed"
    );
    f.thread.join().unwrap();
}

#[test]
fn kill_on_a_run_that_is_not_in_flight_is_the_old_path() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "kill-243-old").unwrap();
    let engine = engine_in(tmp.path());
    suspended_run(&engine, "r-parked");

    cancel_run_with(&engine, &RunId::from("r-parked"), "stale", true, WAIT_30S).unwrap();

    assert_eq!(
        status_of(&engine, &RunId::from("r-parked")),
        RunStatus::Failed
    );
    let error = last_failure(&engine, &RunId::from("r-parked")).unwrap();
    assert_eq!(error, "cancelled: stale");
}

#[test]
fn kill_on_a_finished_run_is_refused_as_before() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "kill-243-done").unwrap();
    let engine = engine_in(tmp.path());
    finished_run(&engine, "r-done");

    let err = cancel_run_with(&engine, &RunId::from("r-done"), "late", true, WAIT_30S).unwrap_err();
    assert!(err.to_string().contains("already finished"), "{err}");
}

#[test]
fn the_timer_of_a_killed_run_is_disarmed() {
    let tmp =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "kill-243-timer").unwrap();
    let dir = tmp.path();
    let engine = engine_in(dir);
    std::fs::write(dir.join("pii.toml"), "").unwrap();
    let cfg = Config::load(&dir.join("pii.toml")).expect("config");
    let d = Daemon::build(
        Arc::clone(&engine),
        &cfg,
        dir,
        dir.join("pii.toml"),
        Arc::new(pii_core::SystemClock),
    )
    .expect("daemon");
    let f = in_flight(&engine, dir, "r-timer", true);
    let (leader, _) = agent_pids(&f);
    let _guard = Guard {
        leaders: vec![leader],
    };
    d.ctx
        .timer_store
        .put_timer(u64::MAX / 2, f.run.as_str(), "k")
        .unwrap();
    assert!(d.ctx.timer_store.next_timer().unwrap().is_some());

    cancel_run_with(&engine, &f.run, "stop", true, WAIT_30S).unwrap();
    f.thread.join().unwrap();

    assert!(
        d.ctx.timer_store.next_timer().unwrap().is_none(),
        "a killed run's timer is removed"
    );
}

#[test]
fn the_kill_is_journalled() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "kill-243-log").unwrap();
    let engine = engine_in(tmp.path());
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let f = in_flight(&engine, tmp.path(), "r-log", true);
    let (leader, _) = agent_pids(&f);
    let _guard = Guard {
        leaders: vec![leader],
    };

    cancel_run_with(&engine, &f.run, "logged", true, WAIT_30S).unwrap();
    f.thread.join().unwrap();

    let kinds: Vec<String> = log.events().into_iter().map(|e| e.kind).collect();
    let killing = kinds
        .iter()
        .position(|k| k == "run.cancel_killing")
        .expect("run.cancel_killing journalled");
    let closed = kinds
        .iter()
        .position(|k| k == "run.cancelled")
        .expect("run.cancelled journalled");
    assert!(killing < closed, "{kinds:?}");
}

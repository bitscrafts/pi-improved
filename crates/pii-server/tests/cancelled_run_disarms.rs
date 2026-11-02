//! Spec 198: a cancelled run does not fire, proven on a scratch daemon with a two-minute cron.
//! Spec 196 removed the wheel entry on cancel; this is the same claim measured end to end: the
//! run is parked by the real daemon, cancelled, and then watched past the instant it was due.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::{RunEvent, RunStatus, Terminal};
use pii_core::{Clock, SystemClock};
use pii_observe::Recorder;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const EVERY_TWO_MINUTES: &str = "name = \"digest\"\nentry = \"tick\"\n\
[[node]]\nid = \"tick\"\nkind = \"timer\"\ncron = \"*/2 * * * *\"\n";

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

/// A scratch daemon whose only cron entry runs the two-minute graph.
fn scratch_daemon() -> (tempfile::TempDir, Arc<Daemon>, Arc<Recorder>) {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cancel-198").unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/digest.toml"), EVERY_TWO_MINUTES).unwrap();
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
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine");
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, "[[cron]]\nname = \"d\"\ngraph = \"digest\"\n").unwrap();
    let mut cfg = Config::load(&cfg_path).expect("config");
    cfg.core.durable_dir = Some(dir.join("durable").display().to_string());
    let d = Daemon::build(engine, &cfg, dir, cfg_path, Arc::new(SystemClock)).expect("daemon");
    (tmp, d, log)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_run_stays_cancelled_past_its_due_instant() {
    let (_tmp, d, log) = scratch_daemon();
    let handles = d.spawn_background();
    d.start_cron_entries();

    // The daemon parks the cron run on its timer and arms the wheel.
    let end = Instant::now() + Duration::from_secs(10);
    let (run, due_ms) = loop {
        let suspended = d.engine.store.runs_by_status(RunStatus::Suspended).unwrap();
        if let (Some(r), Some(due)) = (suspended.first(), d.ctx.timer_store.next_timer().unwrap()) {
            break (r.clone(), due);
        }
        assert!(Instant::now() < end, "the run never parked on its timer");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    pii_server::engine::runs::cancel_run(&d.engine, &run, "operator cancelled").unwrap();
    let seq_at_cancel = d.engine.store.get_run(&run).unwrap().unwrap().seq;

    // Watch until the instant the run was due has passed, with margin for the scheduler.
    let watch_until = due_ms + 5_000;
    while SystemClock.now_ms() < watch_until {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    for h in handles {
        h.abort();
    }

    let after = d.engine.store.get_run(&run).unwrap().unwrap();
    assert_eq!(
        after.status,
        RunStatus::Failed,
        "the cancelled run stays cancelled"
    );
    assert_eq!(
        after.seq, seq_at_cancel,
        "nothing was journalled after the cancel"
    );
    let journal = d.engine.store.read_journal(&run).unwrap();
    let reason = journal.iter().rev().find_map(|(_, e)| match e {
        RunEvent::Finished {
            terminal: Terminal::Failure { error },
            ..
        } => Some(error.clone()),
        _ => None,
    });
    assert_eq!(
        reason.as_deref(),
        Some("cancelled: operator cancelled"),
        "the run stays visible with its reason"
    );
    assert!(
        d.engine
            .store
            .runs_by_status(RunStatus::Failed)
            .unwrap()
            .contains(&run),
        "run ls shows the cancelled run"
    );
    assert!(
        log.events_named(pii_observe::names::RUN_RESUME_REFUSED)
            .is_empty()
            && log
                .events_named(pii_observe::names::TIMER_RESUME_REFUSED)
                .is_empty(),
        "no resume was even attempted for the cancelled run"
    );
}

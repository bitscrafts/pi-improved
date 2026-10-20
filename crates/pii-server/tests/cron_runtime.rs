//! Spec 178: a run started by a `[[cron]]` timer executes inside a Tokio runtime.
//! The re-arm after a finished cron run starts on a plain thread; it used to panic
//! with "there is no reactor running" and crash-loop the supervisor.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_observe::Recorder;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// A timer entry whose run ends at once: the step budget is spent by the timer itself.
const TICKER: &str = "name = \"ticker\"\nentry = \"tick\"\n[limits]\nmax_steps = 0\n\
[[node]]\nid = \"tick\"\nkind = \"timer\"\ncron = \"* * * * *\"\n";

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_cron_run_is_started_again_inside_the_runtime() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cron-rt").unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/ticker.toml"), TICKER).unwrap();
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
    })
    .expect("engine");
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, "[[cron]]\nname = \"t\"\ngraph = \"ticker\"\n").unwrap();
    let cfg = Config::load(&cfg_path).expect("config");
    let d = Daemon::build(engine, &cfg, dir, cfg_path, Arc::new(pii_core::SystemClock))
        .expect("daemon");
    let handles = d.spawn_background();
    d.start_cron_entries();

    // The first firing ends at once and parks on its timer (spec 179: it stays tracked).
    // The SECOND run starts when the next occurrence fires: up to 60 s for `* * * * *`.
    let count = || {
        [
            pii_abi::RunStatus::Failed,
            pii_abi::RunStatus::Running,
            pii_abi::RunStatus::Suspended,
            pii_abi::RunStatus::Succeeded,
        ]
        .into_iter()
        .map(|s| d.engine.store.runs_by_status(s).unwrap().len())
        .sum::<usize>()
    };
    let end = Instant::now() + Duration::from_secs(75);
    let mut started = 0;
    while Instant::now() < end {
        started = count();
        if started >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Spec 179 at the runtime level: one occurrence starts one run, not one per re-arm.
    let settle = Instant::now() + Duration::from_secs(6);
    let mut after = started;
    while Instant::now() < settle {
        after = count();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for h in handles {
        h.abort();
    }
    assert!(
        started >= 2,
        "the cron was started {started} time(s); the re-arm never ran a second"
    );
    assert_eq!(
        after,
        started,
        "a further 6 s started {} more run(s); a parked run must not be re-armed",
        after - started
    );
}

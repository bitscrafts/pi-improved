//! Spec 179: a cron graph whose timer fires once a day is one run per occurrence, not one
//! run per re-arm. Measured on the deployed daemon: 91 to 516 runs in four minutes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_observe::Recorder;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// A daily timer feeding nothing: the run must park until the next 09:00.
const DAILY: &str = "name = \"daily\"\nentry = \"tick\"\n\
[[node]]\nid = \"tick\"\nkind = \"timer\"\ncron = \"0 9 * * *\"\n";

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn total_runs(d: &Daemon) -> usize {
    use pii_abi::RunStatus::{Failed, Running, Succeeded, Suspended};
    [Failed, Running, Suspended, Succeeded]
        .into_iter()
        .map(|s| d.engine.store.runs_by_status(s).unwrap().len())
        .sum()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daily_timer_is_one_run_over_a_window_far_longer_than_its_re_arm() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "no-refire").unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/daily.toml"), DAILY).unwrap();
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
    std::fs::write(&cfg_path, "[[cron]]\nname = \"d\"\ngraph = \"daily\"\n").unwrap();
    let cfg = Config::load(&cfg_path).expect("config");
    let d = Daemon::build(engine, &cfg, dir, cfg_path, Arc::new(pii_core::SystemClock))
        .expect("daemon");
    let handles = d.spawn_background();
    d.start_cron_entries();

    // Six seconds is six times the 1 s minimum re-arm.
    tokio::time::sleep(Duration::from_secs(6)).await;
    let runs = total_runs(&d);
    for h in handles {
        h.abort();
    }
    assert_eq!(runs, 1, "a 09:00 daily timer produced {runs} runs in 6 s");
}

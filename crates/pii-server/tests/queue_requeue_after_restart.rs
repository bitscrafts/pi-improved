//! Spec 353 R4/R5: a queue row claimed by a run that has died is offered again after a restart. A row whose run is
//! still alive is not, and it keeps the queue blocked.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_abi::{RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_core::SystemClock;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_store::queue::{Claim, WorkItem};
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

/// Writes the run's header with `status` into `engine`'s store, ending it `Failed` when `status` says so.
fn write_run(engine: &Engine, run: &str, status: RunStatus) {
    let mut header = RunHeader {
        run: RunId::from(run),
        graph: String::from("queue"),
        status,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    };
    let terminal = Terminal::Failure {
        error: String::from("killed"),
    };
    engine
        .store
        .append_event(
            &mut header,
            &RunEvent::Finished {
                terminal,
                output_hash: String::new(),
            },
        )
        .unwrap();
}

/// A daemon built fresh over `dir`, as after a restart.
fn boot(dir: &Path) -> Arc<Daemon> {
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, "").unwrap();
    let mut cfg = Config::load(&cfg_path).expect("config");
    cfg.core.durable_dir = Some(dir.join("durable").display().to_string());
    Daemon::build(engine_in(dir), &cfg, dir, cfg_path, Arc::new(SystemClock)).expect("daemon")
}

#[test]
fn a_claim_whose_run_died_is_offered_again() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "requeue-dead").unwrap();
    let dir = tmp.path();
    let first = engine_in(dir);
    first.queue.put_item(&WorkItem::new("A1", &[])).unwrap();
    assert!(matches!(
        first.queue.claim_next("r-died").unwrap(),
        Claim::Claimed(_)
    ));
    write_run(&first, "r-died", RunStatus::Failed);
    drop(first);

    let d = boot(dir);
    let claim = d.engine.queue.claim_next("r-next").unwrap();
    assert!(
        matches!(&claim, Claim::Claimed(item) if item.id == "A1"),
        "the dead run's row must be offered again: {claim:?}"
    );
    let kinds: Vec<String> = d
        .engine
        .queue
        .events("A1")
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(
        kinds.iter().any(|k| k == "requeued_after_restart"),
        "no requeued_after_restart event: {kinds:?}"
    );
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for
fn CONTROL_a_live_claim_is_not_requeued() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "requeue-live").unwrap();
    let dir = tmp.path();
    let first = engine_in(dir);
    first.queue.put_item(&WorkItem::new("A1", &[])).unwrap();
    assert!(matches!(
        first.queue.claim_next("r-live").unwrap(),
        Claim::Claimed(_)
    ));
    let mut header = RunHeader {
        run: RunId::from("r-live"),
        graph: String::from("queue"),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    };
    first
        .store
        .append_event(
            &mut header,
            &RunEvent::Started {
                graph: String::from("queue"),
                input_hash: String::new(),
            },
        )
        .unwrap();
    drop(first);

    let d = boot(dir);
    let claim = d.engine.queue.claim_next("r-next").unwrap();
    assert!(
        !matches!(claim, Claim::Claimed(_)),
        "a live run's row must not be offered again: {claim:?}"
    );
}

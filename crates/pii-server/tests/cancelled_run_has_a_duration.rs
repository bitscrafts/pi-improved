//! Spec 310 R1: a run closed from outside its process (cancel, abandon, boot recovery) records how long it ran, from its
//! acceptance to the close. A duration the run's own finish recorded is never overwritten by the close path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    non_snake_case
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{NodeId, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_core::Clock as _;
use pii_server::engine::runs::{abandon_run, cancel_run};
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

/// A run accepted now (its `created_ms` stamped as the engine stamps it) and started, still running.
fn accepted_run(engine: &Engine, run: &str) -> RunId {
    let mut header = RunHeader {
        run: RunId::from(run),
        graph: String::from("agent"),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    };
    engine
        .store
        .stamp_origin(&mut header, pii_core::SystemClock.now_ms());
    engine
        .store
        .append_event(
            &mut header,
            &RunEvent::NodeStarted {
                node: NodeId::from("wait"),
                at_ms: 0,
            },
        )
        .unwrap();
    RunId::from(run)
}

fn stored_dur(engine: &Engine, run: &RunId) -> u32 {
    engine.store.get_run(run).unwrap().expect("run").dur_ms
}

#[test]
fn a_cancelled_run_reports_its_duration() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cancel-dur-").unwrap();
    let engine = engine_in(dir.path());
    let run = accepted_run(&engine, "run-cancel-dur");
    std::thread::sleep(Duration::from_millis(300));
    cancel_run(&engine, &run, "test").unwrap();
    let dur = stored_dur(&engine, &run);
    assert!(
        dur >= 250,
        "a run open ~300 ms reports at least 250 ms, got {dur}"
    );
}

#[test]
fn CONTROL_a_finished_run_keeps_its_duration() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "finish-dur-").unwrap();
    let engine = engine_in(dir.path());
    let run = accepted_run(&engine, "run-finish-dur");
    std::thread::sleep(Duration::from_millis(100));
    // What the run's own `finish` records (run/engine.rs: `set_dur_ms` from the root clock) before it journals `Finished`.
    let mut header = engine.store.get_run(&run).unwrap().expect("run");
    header.dur_ms = 100;
    engine
        .store
        .append_event(
            &mut header,
            &RunEvent::Finished {
                terminal: Terminal::Success {
                    output: serde_json::Value::Null,
                },
                output_hash: String::new(),
            },
        )
        .unwrap();
    let finished = stored_dur(&engine, &run);
    abandon_run(&engine, &run, "after the finish").unwrap();
    assert_eq!(stored_dur(&engine, &run), finished);
    assert_eq!(finished, 100);
}

#[test]
fn a_header_that_already_has_a_duration_is_left_alone() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "keep-dur-").unwrap();
    let engine = engine_in(dir.path());
    let run = RunId::from("run-keep-dur");
    let mut header = RunHeader {
        run: run.clone(),
        graph: String::from("agent"),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 7,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: pii_core::SystemClock.now_ms(),
    };
    engine
        .store
        .append_event(
            &mut header,
            &RunEvent::NodeStarted {
                node: NodeId::from("wait"),
                at_ms: 0,
            },
        )
        .unwrap();
    abandon_run(&engine, &run, "test").unwrap();
    assert_eq!(stored_dur(&engine, &run), 7);
}

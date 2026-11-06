//! Approval tests (F5.9/F5.10/F5.11, part-4 TDD table): request creates a
//! pending store-backed record; resolve is idempotent and typed; unknown ids
//! are `NotFound`; the decision survives a store reopen (daemon restart).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_abi::PiiError;
use pii_server::daemon::approvals::request;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn engine_in(dir: &Path, fixture: &Path) -> Arc<Engine> {
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixture.display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .unwrap()
}

fn ws_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/phase3"));
    std::fs::create_dir_all(&root).expect("create _tmp/phase3");
    let dir = root.join("approvals-it").join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[test]
fn resolve_is_idempotent_and_typed() {
    let dir = scratch(&format!("idem-{}", std::process::id()));
    let fixture = ws_root().join("fixtures/phase3/model/assistant.json");
    let engine = engine_in(&dir, &fixture);
    let store = &engine.approvals;

    // Request records the pending approval (store-backed, F5.10).
    request(
        &engine,
        "ap-1",
        "r-does-not-exist",
        "g",
        "fs_read",
        "may I read the workspace?",
    )
    .unwrap();
    let got = store.get_approval("ap-1").unwrap().unwrap();
    assert_eq!(got.policy, "fs_read");
    assert!(got.decision.is_none(), "fresh approval is pending");
    assert_eq!(store.pending_approvals().unwrap().len(), 1);

    // First resolve applies; second is a no-op (resumed: false).
    let first = store.resolve_approval("ap-1", true, None, 99).unwrap();
    assert!(first.resumed);
    let second = store.resolve_approval("ap-1", false, None, 100).unwrap();
    assert!(!second.resumed, "second resolve must not resume");
    let after = store.get_approval("ap-1").unwrap().unwrap();
    assert_eq!(after.decision, Some(true), "first decision sticks");
    assert_eq!(store.pending_approvals().unwrap().len(), 0);

    // Unknown ids are typed NotFound.
    let err = store.resolve_approval("nope", true, None, 1).unwrap_err();
    assert!(matches!(err, PiiError::NotFound(_)), "{err:?}");
}

#[test]
fn survive_daemon_restart() {
    let dir = scratch(&format!("restart-{}", std::process::id()));
    let store_path = dir.join("store.redb");
    let fixture = ws_root().join("fixtures/phase3/model/assistant.json");

    // First "process": request an approval.
    {
        let engine = engine_in(&dir, &fixture);
        request(&engine, "ap-r", "r1", "g", "bash", "run?").unwrap();
        engine
            .approvals
            .resolve_approval("ap-r", true, None, 5)
            .unwrap();
    }
    // Drop the engine (daemon restart). The decision lives in the store.
    let dir2 = dir.clone();
    let engine2 = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir2,
        store_path,
        model: format!("scripted:{}", fixture.display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .unwrap();
    let a = engine2.approvals.get_approval("ap-r").unwrap().unwrap();
    assert_eq!(
        a.decision,
        Some(true),
        "decision read from store, not memory"
    );
}

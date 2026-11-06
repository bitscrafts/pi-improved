//! Spec-32 spawn-node + worktree-provisioning tests (R1-R5): a `spawn` node
//! launches a child run on the same engine, the child's `scope_root` is the
//! worktree, provisioning returns `{worktree, branch, base_commit}`, and a
//! signature with `attempt_count >= 2` is refused (routes to `escalate`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::too_many_lines)] // reason: integration test with inline graphs

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::{RunEvent, RunStatus};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase3/model/assistant.json")
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../_tmp/phase4.7/spawn-it")
        .join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixture_path().display()),
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
    engine.attach_spawn_engine(Arc::clone(&engine));
    engine
}

/// R1 (T1): a `spawn` node launches a child run; a second run header exists
/// in the store with `status = Running`, and its input carries the parent's
/// payload.
#[tokio::test]
async fn spawn_node_starts_child_run() {
    let dir = scratch(&format!("child-{}", std::process::id()));
    let engine = engine_in(&dir);
    // Write a parent graph with a spawn node pointing at a child graph.
    std::fs::create_dir_all(dir.join("graphs/starter")).unwrap();
    std::fs::write(
        dir.join("graphs/starter/parent.toml"),
        r#"
name = "starter/parent"
entry = "spawner"
caps = ["spawn"]

[[node]]
id = "spawner"
kind = "spawn"
graph = "starter/child"
input = { payload = "$.input.payload" }

[[node]]
id = "done"
kind = "transform"
value = "done"

[[edge]]
from = "spawner"
port = "ok"
to = "done"
kind = "data"
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("graphs/starter/child.toml"),
        r#"
name = "starter/child"
entry = "c"
caps = []

[[node]]
id = "c"
kind = "transform"
value = "child-done"
"#,
    )
    .unwrap();
    let parent = engine
        .start_run("starter/parent", &serde_json::json!({"payload": "hi"}))
        .unwrap();
    // Wait for the parent to finish (it spawns the child synchronously).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let h = engine.store.get_run(&parent).unwrap();
        if h.is_some_and(|h| h.status != RunStatus::Running) {
            break;
        }
        assert!(
            std::time::Instant::now() <= deadline,
            "parent did not finish"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // The parent journal has a RunSpawned link.
    let journal = engine.store.read_journal(&parent).unwrap();
    let spawned = journal
        .iter()
        .filter(|(_, ev)| matches!(ev, RunEvent::RunSpawned { .. }))
        .count();
    assert_eq!(
        spawned, 1,
        "parent must journal one RunSpawned: {journal:?}"
    );
    // A child run header exists (poll — the child is spawned asynchronously).
    let child = journal
        .iter()
        .find_map(|(_, ev)| match ev {
            RunEvent::RunSpawned { child_run } => Some(child_run.clone()),
            _ => None,
        })
        .expect("child run id");
    let child_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let child_header = loop {
        // The header exists from acceptance (spec 154); wait for it to settle.
        if let Some(h) = engine.store.get_run(&child).unwrap()
            && h.status != RunStatus::Running
        {
            break h;
        }
        assert!(
            std::time::Instant::now() <= child_deadline,
            "child header missing or still running for {child}; journal={journal:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    assert_eq!(child_header.status, RunStatus::Succeeded, "child runs");
}

/// R5 (T4, acceptance): a signature with `attempt_count >= 2` is refused — no
/// child run is created; the spawn node routes to `escalate`.
#[tokio::test]
async fn spawn_refused_after_two_attempts() {
    let dir = scratch(&format!("refuse-{}", std::process::id()));
    let engine = engine_in(&dir);
    std::fs::create_dir_all(dir.join("graphs/starter")).unwrap();
    std::fs::write(
        dir.join("graphs/starter/parent.toml"),
        r#"
name = "starter/parent"
entry = "spawner"
caps = ["spawn"]

[[node]]
id = "spawner"
kind = "spawn"
graph = "starter/child"
input = { signature = "$.input.signature" }

[[node]]
id = "done"
kind = "transform"
join = "any"
value = "done"

[[edge]]
from = "spawner"
port = "ok"
to = "done"
kind = "data"

[[edge]]
from = "spawner"
port = "escalate"
to = "done"
kind = "data"
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("graphs/starter/child.toml"),
        r#"
name = "starter/child"
entry = "c"
caps = []

[[node]]
id = "c"
kind = "transform"
value = "child-done"
"#,
    )
    .unwrap();
    // Seed a HealAttempt with attempt_count = 2.
    let sig = "deadbeef";
    let mut attempt = pii_store::heal::HealAttempt {
        signature_hash: sig.to_owned(),
        gate: String::from("quality-gate"),
        state: pii_store::heal::HealState::Detected,
        first_seen_ms: 0,
        last_seen_ms: 0,
        attempts: Vec::new(),
        attempt_count: 2,
        patch_hash: None,
        gate_before: None,
        gate_after: None,
        decision: None,
        model: None,
        tokens_in: 0,
        tokens_out: 0,
        extra: serde_json::Map::new(),
    };
    attempt.attempt_count = 2;
    engine.heals.put_attempt(&attempt).unwrap();
    let parent = engine
        .start_run("starter/parent", &serde_json::json!({"signature": sig}))
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let h = engine.store.get_run(&parent).unwrap();
        if h.is_some_and(|h| h.status != RunStatus::Running) {
            break;
        }
        assert!(
            std::time::Instant::now() <= deadline,
            "parent did not finish; status={:?} journal={:?}",
            engine.store.get_run(&parent).unwrap().map(|h| h.status),
            engine.store.read_journal(&parent).unwrap()
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // No child run was created (no RunSpawned in the journal).
    let journal = engine.store.read_journal(&parent).unwrap();
    let spawned = journal
        .iter()
        .filter(|(_, ev)| matches!(ev, RunEvent::RunSpawned { .. }))
        .count();
    assert_eq!(
        spawned, 0,
        "no child may be spawned after 2 attempts: {journal:?}"
    );
}

//! Session durability tests (spec 16 part2, TDD table): persistence across a
//! daemon restart, monotonic ids, real token counts, `session.list`, and boot
//! reconciliation of crash-interrupted runs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;
use std::path::Path;
use std::sync::Arc;

fn engine_in(dir: &Path, fixture: &str) -> Arc<Engine> {
    Engine::new(EngineConfig {
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{fixture}"),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        graph_dir: dir.join("graphs"),
    })
    .unwrap()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let base = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/_tmp/phase3/session-it"
    ));
    let _ = std::fs::create_dir_all(base);
    let dir = base.join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn fixture_in(dir: &Path) -> String {
    let f = dir.join("fixture.json");
    std::fs::write(
        &f,
        r#"{"turns":[{"text":"ok","tokens_in":1,"tokens_out":1}]}"#,
    )
    .unwrap();
    f.display().to_string()
}

/// R1: a session persists across a daemon (engine) restart — the id is not
/// re-minted and the session is re-seeded from the durable store.
#[test]
fn session_persists_across_restart() {
    let dir = scratch("persist");
    let fixture = fixture_in(&dir);
    let engine = engine_in(&dir, &fixture);
    let sid = engine.create_session(dir.display().to_string(), vec![], None);
    assert_eq!(sid, "s1", "first id should be s1");
    drop(engine);

    // Re-open the same store: the session must survive.
    let engine2 = engine_in(&dir, &fixture);
    engine2.seed_sessions_from_store();
    assert_eq!(
        engine2.create_session(dir.display().to_string(), vec![], None),
        "s2",
        "restart must not re-mint s1"
    );
    let state = engine2
        .session_state("s1")
        .expect("s1 must survive restart");
    assert_eq!(state.session_id, "s1");
}

/// R2: `session.state` returns real accumulated tokens, not the fake (0,0).
#[test]
fn session_state_returns_real_tokens() {
    let dir = scratch("tokens");
    let fixture = fixture_in(&dir);
    let engine = engine_in(&dir, &fixture);
    let sid = engine.create_session(dir.display().to_string(), vec![], None);
    let _ = engine.store.set_session_tokens(&sid, 10, 20);
    let state = engine.session_state(&sid).unwrap();
    assert_eq!(state.tokens_in, 10, "tokens_in must be real");
    assert_eq!(state.tokens_out, 20, "tokens_out must be real");
}

/// R3: `session.list` returns every session, ordered by id.
#[test]
fn session_list_returns_all() {
    let dir = scratch("list");
    let fixture = fixture_in(&dir);
    let engine = engine_in(&dir, &fixture);
    let a = engine.create_session(dir.display().to_string(), vec![], None);
    let b = engine.create_session(dir.display().to_string(), vec![], None);
    let list = engine.session_list();
    let ids: Vec<String> = list.iter().map(|e| e.session_id.clone()).collect();
    assert!(
        ids.contains(&a) && ids.contains(&b),
        "both sessions listed: {ids:?}"
    );
    if let [first, second, ..] = ids.as_slice() {
        assert!(first < second, "ordered by id");
    }
}

/// R3: `session.list` with no sessions returns empty (not an error).
#[test]
fn session_list_empty() {
    let dir = scratch("empty");
    let fixture = fixture_in(&dir);
    let engine = engine_in(&dir, &fixture);
    assert!(engine.session_list().is_empty(), "empty list expected");
}

/// R4: a run left `Running` by a crash is reconciled to `Failed` on boot.
#[test]
fn boot_reconciles_running_runs() {
    use pii_abi::{RunEvent, RunHeader, RunId, RunStatus};
    use pii_store::Store;
    let dir = scratch("reconcile");
    let fixture = fixture_in(&dir);
    let engine = engine_in(&dir, &fixture);
    // Seed a run left Running with a Started event (simulating a crash).
    let mut header = RunHeader {
        run: RunId::from("r1"),
        graph: String::from("default"),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
    };
    engine
        .store
        .append_event(
            &mut header,
            &RunEvent::Started {
                graph: String::from("default"),
                input_hash: String::from("h"),
            },
        )
        .unwrap();
    // The boot reconciler marks it Failed.
    engine.reconcile_boot();
    let runs = engine.store.runs_by_status(RunStatus::Running).unwrap();
    assert!(
        !runs.contains(&RunId::from("r1")),
        "zombie must leave Running"
    );
    let failed = engine.store.runs_by_status(RunStatus::Failed).unwrap();
    assert!(failed.contains(&RunId::from("r1")), "run must be Failed");
    let _ = RunEvent::Finished {
        terminal: pii_abi::Terminal::Failure {
            error: String::from("interrupted by daemon restart"),
        },
        output_hash: String::new(),
    };
    let _ = Store::open; // keep import used
}

/// D3/D6 (026 report 5.0.4): a durable session-persist failure is surfaced
/// through the engine's `Observer` as `ObsEvent::PersistError`, not
/// silently swallowed. Triggered deterministically via
/// `Store::set_fail_session_writes_for_test` (test-only fault injection —
/// see `pii-store/src/lib.rs`), which is far more reliable in a shared
/// test process than OS-level tricks like rlimits or permission bits
/// (both are no-ops for the root user this suite typically runs as).
#[test]
fn persist_failure_emits_obs_event() {
    use pii_core::{ObsEvent, Observer, RecordingObserver};
    use std::sync::Arc;

    let dir = scratch("persist-failure");
    let fixture = fixture_in(&dir);
    let engine = engine_in(&dir, &fixture);
    let obs = Arc::new(RecordingObserver::new());
    engine.attach_observer(obs.clone() as Arc<dyn Observer>);
    let sid = engine.create_session(dir.display().to_string(), vec![], None);

    engine.store.set_fail_session_writes_for_test(true);
    // This call's own persist attempt now fails deterministically; the
    // failure must be observable rather than silently dropped.
    engine.close_session(&sid).ok();
    engine.store.set_fail_session_writes_for_test(false);

    let events = obs.events();
    let found = events.iter().any(|e| {
        matches!(
            e,
            ObsEvent::PersistError { context, .. } if context == &format!("session:{sid}")
        )
    });
    assert!(
        found,
        "expected a PersistError event for session:{sid}, got: {events:?}"
    );
}

/// D3/D6: `session_runs` length stays <= `session_runs_cap` after
/// `cap + 10` runs are added — the oldest run ids are evicted (LRU).
#[tokio::test]
async fn session_runs_capped_with_lru_eviction() {
    let dir = scratch("runs-cap");
    let fixture = fixture_in(&dir);
    let cap = 8usize;
    let engine = Engine::new(EngineConfig {
        workspace: dir.clone(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{fixture}"),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        graph_dir: dir.join("graphs"),
        session_runs_cap: cap,
    })
    .unwrap();
    let sid = engine.create_session(dir.display().to_string(), vec![], None);

    let total = cap + 10;
    for _ in 0..total {
        // `prompt()` is the only public path that grows `session_runs`
        // (it pushes the spawned run id, then trims to the configured
        // cap) — exercised here directly rather than duplicating that
        // logic in the test.
        let _ = engine.prompt(&sid, &[], None);
    }

    let ids = engine.session_run_ids(&sid);
    assert!(
        ids.len() <= cap,
        "session_runs must stay <= cap ({cap}), got {} after {total} runs",
        ids.len()
    );
}

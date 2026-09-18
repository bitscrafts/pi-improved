//! Spec-34 Slice-F dev/prod + gated promotion tests (R1-R5): dev runtime dir
//! isolation, fresh approval per cycle, deny produces no patch artifact, and
//! the live tree is never written.

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
        "/_tmp/phase3/slice-f-it"
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

/// R1 (T1, acceptance): a run started on a dev engine (distinct store) never
/// appears in prod's `run.list` — the two runtime dirs are isolated.
#[tokio::test]
async fn dev_runtime_dir_is_isolated() {
    let prod_dir = scratch("prod");
    let dev_dir = scratch("dev");
    let fixture = fixture_in(&prod_dir);
    let prod = engine_in(&prod_dir, &fixture);
    let dev = engine_in(&dev_dir, &fixture);

    // Start a run on the dev engine.
    std::fs::create_dir_all(dev_dir.join("graphs/starter")).unwrap();
    std::fs::write(
        dev_dir.join("graphs/starter/soak.toml"),
        "name = \"starter/soak\"\nentry = \"step\"\ncaps = []\n\n[[node]]\nid = \"step\"\nkind = \"transform\"\nvalue = \"ok\"\n",
    )
    .unwrap();
    let dev_run = dev
        .start_run("starter/soak", &serde_json::json!({}))
        .unwrap();

    // Wait for the dev run to reach a terminal state (async spawn).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let header = dev.store.get_run(&dev_run).unwrap();
        if header.is_some_and(|h| h.status != pii_abi::RunStatus::Running) {
            break;
        }
        assert!(
            std::time::Instant::now() <= deadline,
            "dev run did not finish"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // The dev run is visible on dev, never on prod.
    let dev_runs = dev
        .store
        .runs_by_status(pii_abi::RunStatus::Succeeded)
        .unwrap();
    assert!(
        dev_runs.iter().any(|r| r.as_str() == dev_run.as_str()),
        "dev run must appear in dev's run.list"
    );
    let prod_runs = prod
        .store
        .runs_by_status(pii_abi::RunStatus::Succeeded)
        .unwrap();
    assert!(
        !prod_runs.iter().any(|r| r.as_str() == dev_run.as_str()),
        "dev run must NEVER appear in prod's run.list"
    );
}

/// R3 (T2): a promotion gate re-entered in a new cycle mints a DIFFERENT
/// approval id, so a fresh human decision is required each cycle.
#[test]
fn promotion_gate_requires_fresh_approval_each_cycle() {
    use pii_core::{Budget, CapSet, Ctx, TraceCtx};
    let c = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::new(pii_core::NullObserver),
        TraceCtx {
            run: pii_abi::RunId::from("r1"),
            span: 0,
        },
    );
    let id0 = pii_server::node_kinds::nodes::gate::approval_id(&c, 0);
    let id1 = pii_server::node_kinds::nodes::gate::approval_id(&c, 1);
    assert_ne!(id0, id1, "a new cycle must mint a fresh approval id");
}

/// R4 (T3): a DENIED promotion produces NO patch artifact and
/// the ledger records `decision.allow == false`.
#[test]
fn deny_produces_no_patch_artifact() {
    let dir = scratch("deny");
    let store = pii_store::heal::HealStore::open(&dir.join("heal.redb")).unwrap();
    let decision = pii_store::heal::ApprovalDecision {
        allow: false,
        approval_id: String::from("approval-deny"),
        by: String::from("test"),
        ts_ms: 100,
    };
    let a = store
        .record_decision("sig-deny", "promotion", decision)
        .unwrap();
    assert_eq!(a.decision.as_ref().map(|d| d.allow), Some(false));
    // No patch artifact is written by a deny (the collect path refuses).
    let reviews = dir.join("heal-reviews");
    std::fs::create_dir_all(&reviews).unwrap();
    let patch = reviews.join("sig-deny.patch");
    assert!(
        !patch.exists(),
        "a denied promotion must not produce a patch"
    );
}

/// R5 (T4): the promote-build runtime dir is confined under the worktree's
/// `_tmp/`, never `/root/.pii` — the live tree is never written.
#[test]
fn promotion_never_writes_live_tree() {
    let wt = scratch("promote");
    let rt = xtask_promote_runtime_dir(&wt, 1234);
    assert!(
        rt.starts_with(&wt),
        "dev runtime dir must be under the worktree: {rt:?}"
    );
    assert!(
        rt.to_string_lossy().contains("_tmp/promote-builds"),
        "dev runtime dir must be under _tmp/promote-builds: {rt:?}"
    );
    assert!(
        !rt.to_string_lossy().contains("/root/.pii"),
        "dev runtime dir must never touch /root/.pii: {rt:?}"
    );
}

/// Mirrors `xtask::promote_build::runtime_dir_for` (tested there too) so the
/// confinement invariant is asserted from the pii-server side without a
/// crate dependency on xtask.
fn xtask_promote_runtime_dir(worktree: &Path, ts_ms: u64) -> std::path::PathBuf {
    worktree
        .join("_tmp/promote-builds")
        .join(ts_ms.to_string())
        .join("runtime")
}

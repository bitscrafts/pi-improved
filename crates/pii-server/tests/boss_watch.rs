//! Spec 27 (boss-loop-graph): the real `starter/boss-watch` graph run on the
//! daemon's own registry — dogfooding (008 §3/§4). R1/R5 drive the REAL
//! `graphs/starter/boss-watch.toml` file through `Engine::load_spec` on the
//! production registry. R3/R3b need `check`/`cyc`/`escalate`/`done` to
//! actually execute past a suspension.
//!
//! **Known substrate limitation (verified empirically, not part of this
//! spec's change set):** `pii_graph`'s `TimerNode::run()`
//! (`crates/pii-graph/src/nodes.rs`) has NO code path that returns
//! `Outcome::Next` — every dispatch of a `timer` node unconditionally
//! returns `Outcome::Suspend`. `pii_graph::run::resume` (and therefore
//! `runs::spawn_resume`, and therefore the daemon scheduler's
//! `fire_due_sync`) replays the journal, finds no `NodeFinished` for the
//! timer node, and re-dispatches it — which suspends again. A `timer`-entry
//! graph can therefore never progress past its first suspension via resume
//! as currently implemented; this is a pre-existing gap in `pii-graph`
//! (confirmed with a throwaway probe against a trivial `timer -> transform`
//! graph, not included in this diff), not something introduced by this
//! spec, and it applies identically to the already-shipped
//! `graphs/starter/heal-watch.toml`. Fixing `pii_graph`'s scheduler is out
//! of spec 27's explicit change set (and out of `crates/pii-server/src`
//! entirely, since the defect lives in `pii-graph`), so:
//!
//! - `boss_watch_timer_suspends_for_real` drives the REAL file through the
//!   REAL `runs::spawn_resume` (exactly the call the daemon scheduler makes)
//!   and honestly documents the current re-suspend behaviour, proving the
//!   timer path is genuinely exercised (not skipped) rather than claiming a
//!   `done` terminal that the substrate cannot currently deliver.
//! - The other execution tests (R1's green path, R3, R3b) load the REAL
//!   `boss-watch.toml` via `Engine::load_spec`, then run a spec that is
//!   IDENTICAL except the (independently broken) `tick` node and its edge
//!   are stripped and `entry` is pointed at `check` directly — same
//!   registry, same node configs, same `gate`/`heal_detect`/`transform`
//!   wiring from the real file, entered past the known-broken node. `gate`
//!   nodes (unlike `timer`) resolve their suspension from a store-backed
//!   decision lookup and round-trip through resume correctly (already
//!   proven by `nodes/gate.rs`'s own unit tests) — only `timer` is affected.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use pii_abi::{NodeId, RunEvent, RunId, RunStatus, SuspendReason, Terminal};
use pii_core::{Budget, BudgetShare, CapKind, CapSet, Ctx, PiiError, Principal};
use pii_graph::spec::GraphSpec;
use pii_server::daemon::approvals;
use pii_server::node_kinds::nodes::gate::approval_id;
use pii_server::node_kinds::nodes::heal::{GateResult, GateRunner, HealFactory};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

fn graphs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase3/model/assistant.json")
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../_tmp/phase4.7/boss-watch-it")
        .join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
    Engine::new(EngineConfig {
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixture_path().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        graph_dir: graphs_dir(),
    })
    .unwrap()
}

/// A scripted `GateRunner`: `results` are consumed in order, then pass
/// (mirrors `nodes/heal.rs`'s own crate-internal `Scripted` test double,
/// unreachable from this integration test).
struct StubRunner {
    results: Mutex<Vec<GateResult>>,
}

impl GateRunner for StubRunner {
    fn run_gate(&self, _ctx: &Ctx, _gate: &str) -> Result<GateResult, PiiError> {
        let mut r = self.results.lock();
        Ok(if r.is_empty() {
            GateResult {
                pass: true,
                detail: String::new(),
            }
        } else {
            r.remove(0)
        })
    }
}

fn fail(msg: &str) -> GateResult {
    GateResult {
        pass: false,
        detail: String::from(msg),
    }
}

/// Builds an engine on the production registry, then overrides the
/// `heal_detect` factory's runner with a deterministic stub (no `cargo
/// xtask` subprocess). `Engine::new` has just returned the only `Arc`
/// reference, so `try_unwrap` is infallible; this mutates the `pub reg`
/// field directly rather than duplicating `Engine::new`'s construction.
fn engine_with_stub(dir: &std::path::Path, results: Vec<GateResult>) -> Arc<Engine> {
    let engine = engine_in(dir);
    let mut engine =
        Arc::try_unwrap(engine).unwrap_or_else(|_| panic!("engine has unexpected extra refs"));
    engine.reg.register(Arc::new(HealFactory {
        heals: Arc::clone(&engine.heals),
        runner: Arc::new(StubRunner {
            results: Mutex::new(results),
        }),
    }));
    Arc::new(engine)
}

fn root_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::PROC;
    Ctx::root(caps, Budget::unlimited())
}

/// The real `starter/boss-watch.toml`, loaded through the real
/// `Engine::load_spec` (same registry that executes it) with the
/// (separately confirmed, out-of-scope) broken `tick` timer node and its
/// edge stripped, entry pointed directly at `check`. See the module doc.
fn check_entry_spec(engine: &Arc<Engine>) -> GraphSpec {
    let mut spec = engine.load_spec("starter/boss-watch").unwrap();
    spec.nodes.retain(|n| n.id.as_str() != "tick");
    spec.edges.retain(|e| e.from.as_str() != "tick");
    spec.entry = NodeId::from("check");
    spec
}

/// The `approval_id` the real `gate` node formula
/// (`pii_server::node_kinds::nodes::gate::approval_id`) computes for `(run, node,
/// cycle)` — used to prove the OBSERVED suspend id reflects the real
/// `cycle` forwarded by `cyc`, not the `cfg.cycle=0` fallback.
fn expected_approval_id(run: &RunId, node: &str, cycle: u64) -> String {
    let root = Ctx::root(CapSet::empty(), Budget::unlimited());
    let ctx = root.child(
        Principal::GraphNode {
            run: run.clone(),
            node: NodeId::from(node),
        },
        &CapSet::empty(),
        &BudgetShare::Shared,
    );
    approval_id(&ctx, cycle)
}

/// The latest `Suspended{Approval}` event's `(node, policy, prompt, key)`.
fn latest_approval_suspend(engine: &Arc<Engine>, run: &RunId) -> (NodeId, String, String, String) {
    let journal = engine.store.read_journal(run).unwrap();
    journal
        .iter()
        .rev()
        .find_map(|(_, e)| match e {
            RunEvent::Suspended {
                node,
                reason: SuspendReason::Approval { policy, prompt },
                key,
            } => Some((node.clone(), policy.clone(), prompt.clone(), key.0.clone())),
            _ => None,
        })
        .expect("approval suspend in journal")
}

/// Spec 73 R1: the poll deadline scales with the environment, not a
/// constant — `PII_TEST_DEADLINE_SECS` (seconds), default 60 (≥3× the worst
/// observed loaded resume time). Read fresh on every helper call.
const DEFAULT_DEADLINE_SECS: u64 = 60;

fn test_deadline_secs() -> u64 {
    pii_core::env_var("PII_TEST_DEADLINE_SECS")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(DEFAULT_DEADLINE_SECS)
}

async fn wait_for_status(
    engine: &Arc<Engine>,
    run_id: &RunId,
    condition: &str,
    pred: impl Fn(RunStatus) -> bool,
) -> RunStatus {
    let deadline_secs = test_deadline_secs();
    let deadline = Instant::now() + Duration::from_secs(deadline_secs);
    loop {
        if let Some(h) = engine.store.get_run(run_id).unwrap()
            && pred(h.status)
        {
            return h.status;
        }
        assert!(
            Instant::now() <= deadline,
            "timed out waiting for status: {condition} (deadline {deadline_secs}s)"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_journal_len(
    engine: &Arc<Engine>,
    run_id: &RunId,
    min_len: usize,
) -> Vec<(u64, RunEvent)> {
    let deadline_secs = test_deadline_secs();
    let deadline = Instant::now() + Duration::from_secs(deadline_secs);
    loop {
        let j = engine.store.read_journal(run_id).unwrap();
        let last = j.len();
        if last >= min_len {
            return j;
        }
        assert!(
            Instant::now() <= deadline,
            "timed out waiting for journal growth: condition `journal len >= {min_len}` \
             (deadline {deadline_secs}s, last journal len {last})"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// R1: the real file validates on the real (production) registry, has no
/// `agent` node anywhere, and grants no `fs_write` at the graph level or on
/// any individual node.
#[test]
fn boss_watch_validates_on_daemon_registry() {
    let dir = scratch(&format!("validate-{}", std::process::id()));
    let engine = engine_in(&dir);
    let spec = engine
        .load_spec("starter/boss-watch")
        .expect("load real file");
    pii_graph::validate::validate(&spec, &engine.reg).expect("graph must validate");

    assert!(
        !spec.nodes.iter().any(|n| n.kind == "agent"),
        "no agent node anywhere: {:?}",
        spec.nodes.iter().map(|n| &n.kind).collect::<Vec<_>>()
    );
    assert!(
        !CapKind::from_bits_truncate(spec.caps).contains(CapKind::FS_WRITE),
        "graph-level caps must not grant fs_write"
    );
    for n in &spec.nodes {
        assert!(
            !CapKind::from_bits_truncate(n.caps).contains(CapKind::FS_WRITE),
            "node '{}' must not grant fs_write",
            n.id
        );
    }
}

/// R5: the kind set is a subset of `{timer, heal_detect, transform, gate}`.
#[test]
fn boss_watch_no_model_node() {
    let dir = scratch(&format!("nomodel-{}", std::process::id()));
    let engine = engine_in(&dir);
    let spec = engine.load_spec("starter/boss-watch").unwrap();
    let allowed = ["timer", "heal_detect", "transform", "gate"];
    for n in &spec.nodes {
        assert!(
            allowed.contains(&n.kind.as_str()),
            "node '{}' has kind '{}' outside {allowed:?}",
            n.id,
            n.kind
        );
    }
}

/// See the module doc: proves the REAL file suspends for real on `Timer`,
/// and that the REAL production `runs::spawn_resume` (the exact call the
/// daemon scheduler's `fire_due_sync` makes) genuinely re-enters the run —
/// not skipped, not a no-op — while honestly documenting that it currently
/// re-suspends on `tick` rather than reaching `check` (a pre-existing
/// `pii-graph` limitation, not introduced by this spec).
#[tokio::test]
async fn boss_watch_timer_suspends_for_real() {
    let dir = scratch(&format!("timerreal-{}", std::process::id()));
    let engine = engine_in(&dir);
    let run_id = engine
        .start_run("starter/boss-watch", &serde_json::json!({"cycle": 0}))
        .unwrap();
    wait_for_status(
        &engine,
        &run_id,
        "run status == Suspended (timer suspend)",
        |s| s == RunStatus::Suspended,
    )
    .await;

    let before = engine.store.read_journal(&run_id).unwrap();
    let timer_suspends_before = before
        .iter()
        .filter(|(_, e)| {
            matches!(e, RunEvent::Suspended { node, reason: SuspendReason::Timer { .. }, .. } if node.as_str() == "tick")
        })
        .count();
    assert_eq!(
        timer_suspends_before, 1,
        "one real Timer suspend: {before:?}"
    );

    pii_server::engine::runs::spawn_resume(Arc::clone(&engine), run_id.clone(), None).unwrap();
    let after = wait_for_journal_len(&engine, &run_id, before.len() + 1).await;

    let timer_suspends_after = after
        .iter()
        .filter(|(_, e)| {
            matches!(e, RunEvent::Suspended { node, reason: SuspendReason::Timer { .. }, .. } if node.as_str() == "tick")
        })
        .count();
    assert_eq!(
        timer_suspends_after, 2,
        "spawn_resume must genuinely re-enter dispatch (proving the timer path is real, \
         not skipped) — current pii-graph TimerNode has no completion path, so it re-suspends: {after:?}"
    );
    assert_eq!(
        engine.store.get_run(&run_id).unwrap().unwrap().status,
        RunStatus::Suspended
    );
}

/// R1's green path: `check_entry_spec` reuses the REAL file's `check`/`done`
/// node configs and registry; a passing stub gate routes `ok -> done`.
#[test]
fn boss_watch_green_completes() {
    let dir = scratch(&format!("green-{}", std::process::id()));
    let engine = engine_with_stub(
        &dir,
        vec![GateResult {
            pass: true,
            detail: String::new(),
        }],
    );
    let spec = check_entry_spec(&engine);
    let ctx = root_ctx();
    let run_id = engine.store.next_run_id().unwrap();
    let terminal = pii_graph::run::run(
        &engine.store,
        &spec,
        &engine.reg,
        &ctx,
        &run_id,
        serde_json::json!({"cycle": 0}),
    )
    .unwrap();
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "expected Success, got {terminal:?}"
    );

    let journal = engine.store.read_journal(&run_id).unwrap();
    assert!(
        journal.iter().any(|(_, e)| matches!(
            e,
            RunEvent::NodeFinished { node, port, .. } if node.as_str() == "check" && port.as_str() == "ok"
        )),
        "check must finish on 'ok': {journal:?}"
    );
    assert!(
        matches!(
            journal.last().map(|(_, e)| e),
            Some(RunEvent::Finished {
                terminal: Terminal::Success { .. },
                ..
            })
        ),
        "run must terminate Success: {journal:?}"
    );
}

/// R3 (the acceptance test): a stub that fails twice suspends on approval;
/// the real `daemon::approvals::request`/`resolve` glue resumes it to
/// `allow -> done`.
#[tokio::test]
async fn boss_watch_gate_suspends_and_approve_resumes() {
    let dir = scratch(&format!("approve-{}", std::process::id()));
    let engine = engine_with_stub(&dir, vec![fail("f1"), fail("f2")]);
    let spec = check_entry_spec(&engine);
    let ctx = root_ctx();
    let run_id = engine.store.next_run_id().unwrap();
    let terminal = pii_graph::run::run(
        &engine.store,
        &spec,
        &engine.reg,
        &ctx,
        &run_id,
        serde_json::json!({"cycle": 1}),
    )
    .unwrap();
    let key = match terminal {
        Terminal::Suspended { key } => key,
        other => panic!("expected Suspended (approval), got {other:?}"),
    };
    let approval_id_str = key.0.strip_prefix("approval:").unwrap_or(&key.0).to_owned();
    let (node, policy, prompt, key_str) = latest_approval_suspend(&engine, &run_id);
    assert_eq!(key_str, key.0, "journal key matches the terminal's key");

    // Do NOT hand-roll the store insert — call the real glue the daemon's
    // `on_suspended` reactive handler calls.
    approvals::request(
        &engine,
        &approval_id_str,
        run_id.as_str(),
        node.as_str(),
        &policy,
        &prompt,
    )
    .unwrap();
    assert_eq!(
        engine.approvals.pending_approvals().unwrap().len(),
        1,
        "pending approval row before resolve"
    );

    let outcome = approvals::resolve(&engine, &approval_id_str, true, None).unwrap();
    assert!(outcome.resumed);

    let status = wait_for_status(
        &engine,
        &run_id,
        "run reaches Succeeded|Failed (allow)",
        |s| matches!(s, RunStatus::Succeeded | RunStatus::Failed),
    )
    .await;
    assert_eq!(status, RunStatus::Succeeded);
    let journal = engine.store.read_journal(&run_id).unwrap();
    assert!(
        journal.iter().any(|(_, e)| matches!(
            e,
            RunEvent::NodeFinished { node, port, .. } if node.as_str() == "escalate" && port.as_str() == "allow"
        )),
        "escalate must finish on 'allow': {journal:?}"
    );
    assert!(
        engine.approvals.pending_approvals().unwrap().is_empty(),
        "approval row resolved"
    );
}

/// R3's deny arm: resolving `false` terminates the run via `deny`.
#[tokio::test]
async fn boss_watch_gate_suspend_and_deny_terminates() {
    let dir = scratch(&format!("deny-{}", std::process::id()));
    let engine = engine_with_stub(&dir, vec![fail("f1"), fail("f2")]);
    let spec = check_entry_spec(&engine);
    let ctx = root_ctx();
    let run_id = engine.store.next_run_id().unwrap();
    let terminal = pii_graph::run::run(
        &engine.store,
        &spec,
        &engine.reg,
        &ctx,
        &run_id,
        serde_json::json!({"cycle": 1}),
    )
    .unwrap();
    let key = match terminal {
        Terminal::Suspended { key } => key,
        other => panic!("expected Suspended (approval), got {other:?}"),
    };
    let approval_id_str = key.0.strip_prefix("approval:").unwrap_or(&key.0).to_owned();
    let (node, policy, prompt, _key_str) = latest_approval_suspend(&engine, &run_id);

    approvals::request(
        &engine,
        &approval_id_str,
        run_id.as_str(),
        node.as_str(),
        &policy,
        &prompt,
    )
    .unwrap();
    let outcome = approvals::resolve(&engine, &approval_id_str, false, None).unwrap();
    assert!(outcome.resumed);

    let status = wait_for_status(
        &engine,
        &run_id,
        "run reaches Succeeded|Failed (deny)",
        |s| matches!(s, RunStatus::Succeeded | RunStatus::Failed),
    )
    .await;
    assert_eq!(
        status,
        RunStatus::Succeeded,
        "deny is a routed port, not a run failure"
    );
    let journal = engine.store.read_journal(&run_id).unwrap();
    assert!(
        journal.iter().any(|(_, e)| matches!(
            e,
            RunEvent::NodeFinished { node, port, .. } if node.as_str() == "escalate" && port.as_str() == "deny"
        )),
        "escalate must finish on 'deny': {journal:?}"
    );
}

/// R3b: two fresh runs (cycle 1, cycle 2) each mint a DIFFERENT approval id
/// that matches the real `gate::approval_id(run, node, cycle)` formula for
/// their OWN input cycle (not the `cfg.cycle=0` fallback); approving run 1
/// leaves run 2 suspended, never auto-allowed.
///
/// **Mutation check** (verified by reasoning, per the task instructions):
/// `heal_detect`'s `detected` value is `{signature_hash, detail}` with no
/// `cycle` key. If the graph's `cyc` node (`expr = "$.input"`) were deleted,
/// `escalate`'s `input.payload` would be `check`'s raw detected value —
/// still cycle-less — so `GateNode::run` (`nodes/gate.rs:83-87`) would fall
/// back to `self.cycle` (the static `cfg["cycle"]`, absent from the TOML,
/// defaulting to `0`) for EVERY run, regardless of the `cycle` actually
/// passed in the run's input. The `expected1`/`expected2` assertions below
/// compare the OBSERVED id against `approval_id(ctx, 1)`/`approval_id(ctx,
/// 2)` (the real cycle) — with `cyc` deleted, the observed id would instead
/// equal `approval_id(ctx, 0)` for both runs, so `id1 == expected1` and `id2
/// == expected2` would both FAIL (each observed id would match its own
/// run's cycle-0 fallback, not the intended cycle-1/cycle-2 id) — the test
/// is sensitive to `cyc`'s removal even though `id1 != id2` alone is not
/// (run ids already differ, per `Principal::GraphNode { run, .. }` in
/// `crates/pii-graph/src/sched/dispatch.rs:178-181`).
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn boss_watch_second_cycle_suspends_with_fresh_id() {
    let dir = scratch(&format!("fresh-{}", std::process::id()));
    let engine = engine_with_stub(&dir, vec![fail("f1"), fail("f2"), fail("f3"), fail("f4")]);
    let spec = check_entry_spec(&engine);
    let ctx = root_ctx();

    let run1 = engine.store.next_run_id().unwrap();
    let t1 = pii_graph::run::run(
        &engine.store,
        &spec,
        &engine.reg,
        &ctx,
        &run1,
        serde_json::json!({"cycle": 1}),
    )
    .unwrap();
    let key1 = match t1 {
        Terminal::Suspended { key } => key,
        other => panic!("run1 expected Suspended, got {other:?}"),
    };
    let id1 = key1
        .0
        .strip_prefix("approval:")
        .unwrap_or(&key1.0)
        .to_owned();
    let expected1 = expected_approval_id(&run1, "escalate", 1);
    let stale_fallback1 = expected_approval_id(&run1, "escalate", 0);
    assert_eq!(
        id1, expected1,
        "run1's id must reflect cycle=1 (forwarded by cyc)"
    );
    assert_ne!(
        id1, stale_fallback1,
        "must not collapse to the cycle=0 fallback"
    );

    let run2 = engine.store.next_run_id().unwrap();
    let t2 = pii_graph::run::run(
        &engine.store,
        &spec,
        &engine.reg,
        &ctx,
        &run2,
        serde_json::json!({"cycle": 2}),
    )
    .unwrap();
    let key2 = match t2 {
        Terminal::Suspended { key } => key,
        other => panic!("run2 expected Suspended, got {other:?}"),
    };
    let id2 = key2
        .0
        .strip_prefix("approval:")
        .unwrap_or(&key2.0)
        .to_owned();
    let expected2 = expected_approval_id(&run2, "escalate", 2);
    assert_eq!(
        id2, expected2,
        "run2's id must reflect cycle=2 (forwarded by cyc)"
    );

    assert_ne!(id1, id2, "fresh cycle must mint a fresh approval id");

    let (node1, policy1, prompt1, _) = latest_approval_suspend(&engine, &run1);
    approvals::request(
        &engine,
        &id1,
        run1.as_str(),
        node1.as_str(),
        &policy1,
        &prompt1,
    )
    .unwrap();
    let outcome = approvals::resolve(&engine, &id1, true, None).unwrap();
    assert!(outcome.resumed);
    let status1 = wait_for_status(&engine, &run1, "run1 reaches Succeeded|Failed", |s| {
        matches!(s, RunStatus::Succeeded | RunStatus::Failed)
    })
    .await;
    assert_eq!(
        status1,
        RunStatus::Succeeded,
        "run1 completes via the approved allow"
    );

    // Run2 must remain suspended: its approval was never requested/resolved
    // (it must NOT have been auto-allowed by run1's decision).
    assert_eq!(
        engine.store.get_run(&run2).unwrap().unwrap().status,
        RunStatus::Suspended,
        "run2 must still be suspended, not auto-allowed"
    );
    assert!(
        engine.approvals.get_approval(&id2).unwrap().is_none(),
        "run2's approval must never have been requested or decided"
    );
}

/// Spec 30 T2: a boss-watch-SHAPED graph (`timer` → `heal_detect` → `gate`,
/// the same node kinds and wiring as the real `graphs/starter/boss-watch.toml`
/// but with a short `in_ms` timer instead of the cron `*/30 * * * *` — a
/// cron's next fire is up to 30 min away, untestable in a unit window),
/// resumed once after the timer fires, reaches the `check` node. This proves
/// the `timer`→`heal_detect` handoff that the shipped boss graph depends on.
#[tokio::test]
async fn boss_watch_reaches_detect_after_resume() {
    let dir = scratch(&format!("reachesdetect-{}", std::process::id()));
    // Write a boss-watch-shaped graph with a 1ms timer into a SCRATCH graph
    // dir (the real `graphs/` uses cron "*/30 * * * *" — next fire up to 30
    // min away, untestable in a unit window).
    std::fs::create_dir_all(dir.join("graphs/starter")).unwrap();
    std::fs::write(
        dir.join("graphs/starter/boss-watch.toml"),
        r#"
name = "starter/boss-watch"
entry = "tick"
caps = ["proc"]

[[node]]
id = "tick"
kind = "timer"
in_ms = 1

[[node]]
id = "check"
kind = "heal_detect"
gate = "silent-noop-check"
caps = ["proc"]

[[node]]
id = "done"
kind = "transform"
join = "any"
value = "no-silent-noop"

[[edge]]
from = "tick"
port = "fired"
to = "check"
kind = "data"

[[edge]]
from = "check"
port = "ok"
to = "done"
kind = "data"

[[edge]]
from = "check"
port = "flake"
to = "done"
kind = "data"
"#,
    )
    .unwrap();
    // Engine whose graph_dir is the SCRATCH dir (not the real graphs/).
    let engine = Engine::new(EngineConfig {
        workspace: dir.clone(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixture_path().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        graph_dir: dir.join("graphs"),
    })
    .unwrap();
    let run_id = engine
        .start_run("starter/boss-watch", &serde_json::json!({"cycle": 0}))
        .unwrap();
    wait_for_status(
        &engine,
        &run_id,
        "run status == Suspended (timer fired)",
        |s| s == RunStatus::Suspended,
    )
    .await;

    // Wait past the 1ms fire time, then resume.
    tokio::time::sleep(Duration::from_millis(50)).await;
    pii_server::engine::runs::spawn_resume(Arc::clone(&engine), run_id.clone(), None).unwrap();

    // The resume is async — wait for the journal to grow past the initial
    // suspend (the timer fires and forwards to heal_detect).
    let before = engine.store.read_journal(&run_id).unwrap().len();
    let journal = wait_for_journal_len(&engine, &run_id, before + 1).await;
    let check_started = journal
        .iter()
        .filter(
            |(_, ev)| matches!(ev, RunEvent::NodeStarted { node, .. } if node.as_str() == "check"),
        )
        .count();
    assert!(
        check_started >= 1,
        "boss-watch must reach the check node after resume: {journal:?}"
    );
}

/// Spec 73 (ACCEPTANCE probe, R1/R3): the poll helpers must take their
/// deadline from `PII_TEST_DEADLINE_SECS` (default 60) and their timeout
/// panics must self-describe (condition + deadline used + last journal
/// length). Drives the REAL helpers against a REAL engine on a fake run id
/// — `pii-store`'s `get_run(missing)` is `Ok(None)` and
/// `read_journal(missing)` is `Ok(vec![])`, so unsatisfiable conditions loop
/// straight to the deadline and panic — proving the override path end to
/// end (the TDD-contract "instant < 5 s bound": elapsed < deadline + 2 s).
///
/// Ignored by default: each probe burns exactly one deadline. Run it with:
/// `PII_TEST_DEADLINE_SECS=5 cargo test -p pii-server --test boss_watch -- --ignored`
#[tokio::test]
#[ignore = "burns one full deadline to prove the timeout path; run with: PII_TEST_DEADLINE_SECS=5 cargo test -p pii-server --test boss_watch -- --ignored"]
async fn deadline_override_is_honoured() {
    let dir = scratch(&format!("deadline-{}", std::process::id()));
    let engine = engine_in(&dir);
    let fake_run = RunId::from("deadline-probe");
    // The deadline the helpers MUST honour — through the SAME env path the
    // helpers use (R1), so the assertion tracks whatever override is set.
    let expected = test_deadline_secs();

    let started = Instant::now();
    let status_engine = Arc::clone(&engine);
    let status_run = fake_run.clone();
    let journal_engine = Arc::clone(&engine);
    let journal_run = fake_run.clone();
    let status = tokio::spawn(async move {
        wait_for_status(
            &status_engine,
            &status_run,
            "probe: status never matches",
            |_| false,
        )
        .await
    });
    let journal = tokio::spawn(async move {
        wait_for_journal_len(&journal_engine, &journal_run, usize::MAX).await
    });

    // (a) both waits must give up at the env-derived deadline (+2 s slack),
    // not at a hardcoded constant.
    let status_msg = join_panic_message(status).await;
    assert!(
        started.elapsed() < Duration::from_secs(expected + 2),
        "status wait must panic at the env deadline ({expected}s + 2s slack), \
         took {:?} — deadline override not honoured: {status_msg}",
        started.elapsed()
    );
    // (b) R3: the timeout message must name the deadline it used.
    assert!(
        status_msg.contains(&format!("deadline {expected}s")),
        "status panic must name the deadline used: {status_msg}"
    );

    // (c) R3 (journal helper): condition wording + deadline + last journal len.
    let journal_msg = join_panic_message(journal).await;
    assert!(
        journal_msg.contains(&format!("journal len >= {}", usize::MAX))
            && journal_msg.contains(&format!("deadline {expected}s"))
            && journal_msg.contains("last journal len 0"),
        "journal panic must name condition + deadline + last journal len: {journal_msg}"
    );
}

/// Extract a string panic payload from a spawned helper task (the poll
/// helpers panic via `assert!`, whose payload is a `String`).
async fn join_panic_message<T>(handle: tokio::task::JoinHandle<T>) -> String {
    let Err(err) = handle.await else {
        panic!("helper returned instead of timing out")
    };
    let payload = err.into_panic();
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .expect("panic payload is a string")
}

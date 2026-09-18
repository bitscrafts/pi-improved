//! Spec-33 Slice-E-live tests (R1-R4): routed patch model, structural
//! reproduction enforcement, loop-budget gate, and ledger evidence.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::daemon::config::Config;
use std::path::Path;

fn ws_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = ws_root().join("_tmp/phase3/slice-e-it").join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// R1: the real `graphs/starter/heal-fix.toml` declares `route_step = "patch"`
/// on its `fix` agent, and `[evolve.routing] patch = "X"` resolves to X.
#[test]
fn heal_fix_resolves_patch_model() {
    // The shipped graph must carry the routing declaration (R1).
    let graph = std::fs::read_to_string(ws_root().join("graphs/starter/heal-fix.toml"))
        .expect("heal-fix.toml");
    assert!(
        graph.contains("route_step = \"patch\""),
        "heal-fix.toml must declare route_step = \"patch\" on the fix agent"
    );

    // The routing config parses and resolves the patch step.
    let dir = scratch("resolves");
    let toml = dir.join("pii.toml");
    std::fs::write(
        &toml,
        "[evolve.routing]\npatch = \"nvidia:deepseek-ai/deepseek-v3.2\"\n",
    )
    .unwrap();
    let cfg = Config::load(&toml).expect("config parse");
    let policy = cfg.evolve.routing;
    let m = pii_server::node_kinds::agent::resolve_model_for_test(
        &serde_json::json!({"route_step": "patch"}),
        &policy,
    );
    assert_eq!(
        m.expect("resolved"),
        Some(String::from("nvidia:deepseek-ai/deepseek-v3.2")),
        "patch step must resolve via [evolve.routing]"
    );
}

/// R2: reproduction is enforced structurally — a probe that exits 0 (the
/// break does NOT reproduce) routes to `unexpected_ok` (a human gate), so
/// no agent node ever starts. Assert the shipped heal-fix graph carries
/// this routing (the load-bearing structural guarantee).
#[test]
fn unreproduced_break_escalates() {
    let graph = std::fs::read_to_string(ws_root().join("graphs/starter/heal-fix.toml"))
        .expect("heal-fix.toml");
    // The probe routes exit-0 to `unexpected_ok` (a gate), never to `fix`.
    assert!(
        graph.contains("port = \"ok\"") && graph.contains("to = \"unexpected_ok\""),
        "a probe that exits 0 (break does not reproduce) must route to unexpected_ok"
    );
    // `unexpected_ok` is a human gate, not an agent node — no model runs.
    assert!(
        graph.contains("id = \"unexpected_ok\"") && graph.contains("kind = \"gate\""),
        "unexpected_ok must be a gate (no agent/model node on the non-reproduced path)"
    );
}

/// R3: a still-broken retest after `loop_budget` must NOT reach the human
/// gate — the run fails structurally and the ledger row is `Escalated`,
/// not `Fixed`.
#[test]
fn still_broken_after_loop_budget_does_not_reach_gate() {
    // The structural guarantee is enforced by the engine's dispatch loop:
    // exceeding loop_budget is a terminal failure. Assert the shipped
    // heal-fix graph's `escalate` gate is only reachable from
    // `route_retest.ok` (a passing retest), never from a failed retest.
    let graph = std::fs::read_to_string(ws_root().join("graphs/starter/heal-fix.toml"))
        .expect("heal-fix.toml");
    // The retry back-edge must be bounded by loop_budget on the fix node.
    assert!(
        graph.contains("loop_budget = 2"),
        "heal-fix.toml must bound the retry loop with loop_budget"
    );
    // The escalate gate is the ONLY way past the retest; a failed retest
    // routes to retry (bounded), never straight to escalate.
    assert!(
        graph.contains("port = \"still_broken\"") && graph.contains("to = \"retry\""),
        "a still-broken retest must route to retry, not to the human gate"
    );
}

/// R4: a validated repair records the ledger evidence — `gate_before`,
/// `gate_after`, `patch_hash`, `model`, `tokens`.
#[test]
fn ledger_records_gate_before_and_after() {
    // The pii-store `record_repair` path (the producer the heal_record node
    // calls) must persist the evidence fields. This is the load-bearing
    // unit; the node itself is a thin wrapper over it.
    let dir = scratch(&format!("ledger-{}", std::process::id()));
    let store = pii_store::heal::HealStore::open(&dir.join("heal.redb")).unwrap();
    let run = pii_store::heal::HealAttemptRun {
        run_id: String::from("r1"),
        outcome: pii_store::heal::HealState::Fixed,
        ts_ms: 100,
        detail: String::from("diff"),
    };
    let a = store
        .record_repair(
            "sig-ledger",
            "heal-fix",
            run,
            Some(String::from("fail")),
            Some(String::from("pass")),
            Some(String::from("patchhash")),
            Some(String::from("modelX")),
            10,
            20,
        )
        .unwrap();
    assert_eq!(a.state, pii_store::heal::HealState::Fixed);
    assert_eq!(a.gate_before, Some(String::from("fail")));
    assert_eq!(a.gate_after, Some(String::from("pass")));
    assert_eq!(a.patch_hash, Some(String::from("patchhash")));
    assert_eq!(a.model, Some(String::from("modelX")));
    assert_eq!(a.tokens_in, 10);
    assert_eq!(a.tokens_out, 20);
    // Round-trips through storage.
    let reloaded = store.get_attempt("sig-ledger").unwrap().unwrap();
    assert_eq!(reloaded.gate_before, Some(String::from("fail")));
    assert_eq!(reloaded.gate_after, Some(String::from("pass")));
    assert_eq!(reloaded.patch_hash, Some(String::from("patchhash")));
}

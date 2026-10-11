//! Spec 138: the daemon is the only writer of `learn/`. A promotion asked for
//! over `learn.promote` is applied and recorded in one step, read back after a
//! restart, and joinable to the decision ledger by its promotion id; a caller
//! naming another store is refused rather than written to this one.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_learn::{
    Candidate, Evidence, Kind, Verdict, VerdictRecord, get_promotion, ledger_tail, put_verdict,
    store,
};
use pii_proto::method::Method;
use pii_proto::{Id, Request, Response};

use super::tests::test_ctx;
use crate::engine::learn_methods::{
    learn_promote, learn_revert, learn_unattended_pass, promotion_sweep_at,
};
use crate::{Engine, EngineConfig};

fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
    let fixture = dir.join("fixture.json");
    std::fs::write(&fixture, r#"{"turns":[]}"#).unwrap();
    let engine = Engine::new(EngineConfig {
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixture.display()),
        sandbox: pii_tools::sandbox::SandboxProfileName::Off,
        graph_dir: dir.to_path_buf(),
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
    })
    .unwrap();
    engine.attach_daemon_ctx(test_ctx(dir));
    engine
}

fn req(method: Method, params: serde_json::Value) -> Request {
    Request {
        jsonrpc: "2.0".into(),
        id: Id::Num(1),
        method,
        params,
    }
}

fn result(r: &Response) -> &serde_json::Value {
    r.result
        .as_ref()
        .unwrap_or_else(|| panic!("expected a result, got {:?}", r.error))
}

fn confirmed_candidate(dir: &std::path::Path) -> Candidate {
    let c = Candidate::new(
        Kind::StuckLoop,
        String::from("graph sick failed in all 3 runs"),
        String::from("stuck_loop:sick:n:other"),
        Evidence {
            runs: vec![String::from("r1")],
            paths: vec![String::from("trajectories/r1.jsonl")],
            counts: std::collections::BTreeMap::from([(String::from("runs"), 3)]),
        },
    )
    .unwrap();
    store::put_candidates(dir, std::slice::from_ref(&c)).unwrap();
    put_verdict(
        dir,
        &VerdictRecord {
            candidate: c.id.clone(),
            verdict: Verdict::Confirmed,
            reason: None,
            detail: String::new(),
            shadow: String::new(),
            shadow_runs: Vec::new(),
            numbers: std::collections::BTreeMap::new(),
            evidence_used: Vec::new(),
            missing: Vec::new(),
            bound: None,
            elapsed_ms: 0,
        },
    )
    .unwrap();
    c
}

fn scratch() -> tempfile::TempDir {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"));
    std::fs::create_dir_all(root).unwrap();
    tempfile::Builder::new()
        .prefix("learn-rpc-")
        .tempdir_in(root)
        .unwrap()
}

/// R5(c): apply over the RPC, "restart" (a second engine on the same dir), and
/// the record is there — keyed by the promotion id — and can be reverted.
#[test]
fn a_promotion_reads_back_after_a_restart() {
    let tmp = scratch();
    let dir = tmp.path().to_path_buf();
    let c = confirmed_candidate(&dir);
    let first = engine_in(&dir);
    let r = learn_promote(
        &first,
        &req(
            Method::LearnPromote,
            serde_json::json!({
                "candidate": c.id, "actor": "test:operator",
                "metric": "graph_definition_changed", "window_secs": 100, "dir": dir,
            }),
        ),
    );
    let id = result(&r)["promotion"]["id"].as_str().unwrap().to_owned();
    assert!(
        result(&r)["ledger"].is_string(),
        "the applied record's ledger id is returned"
    );
    drop(first);

    let second = engine_in(&dir);
    let p = get_promotion(&dir, &id).expect("the promotion survives a restart");
    let applied = ledger_tail(&dir, 10);
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].promotion, p.id, "joinable by promotion id");
    assert_eq!(applied[0].event, "applied");
    let r = learn_revert(
        &second,
        &req(
            Method::LearnRevert,
            serde_json::json!({"id": id, "actor": "test:operator"}),
        ),
    );
    assert_eq!(result(&r)["reverted"], id.as_str());
    let l = ledger_tail(&dir, 10);
    assert_eq!(
        l.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![1, 2],
        "one writer, contiguous seq"
    );
}

/// A caller naming another store is refused: the daemon owns exactly one.
#[test]
fn a_request_for_another_store_is_refused_not_written_here() {
    let tmp = scratch();
    let dir = tmp.path().to_path_buf();
    let c = confirmed_candidate(&dir);
    let engine = engine_in(&dir);
    let other = scratch();
    let r = learn_promote(
        &engine,
        &req(
            Method::LearnPromote,
            serde_json::json!({
                "candidate": c.id, "actor": "x", "metric": "graph_definition_changed",
                "window_secs": 100, "dir": other.path(),
            }),
        ),
    );
    let e = r.error.expect("must be refused");
    assert!(e.message.contains("the daemon owns the ledger"), "{e:?}");
    assert!(ledger_tail(&dir, 10).is_empty(), "nothing was written");
}

const SICK_V1: &str = "name = \"sick\"\nentry = \"a\"\ncaps = []\n[[node]]\nid = \"a\"\nkind = \"transform\"\nexpr = \"$.input\"\n";

/// Applies a quarantine on `sick` (its definition on disk is `SICK_V1`) and
/// returns the engine, the dir and the promotion id.
fn promoted_sick(tmp: &tempfile::TempDir) -> (Arc<Engine>, std::path::PathBuf, String) {
    let dir = tmp.path().to_path_buf();
    std::fs::write(dir.join("sick.toml"), SICK_V1).unwrap();
    let c = confirmed_candidate(&dir);
    let engine = engine_in(&dir);
    let r = learn_promote(
        &engine,
        &req(
            Method::LearnPromote,
            serde_json::json!({
                "candidate": c.id, "actor": "test:operator",
                "metric": "graph_definition_changed", "window_secs": 100, "dir": dir,
            }),
        ),
    );
    let id = result(&r)["promotion"]["id"].as_str().unwrap().to_owned();
    (engine, dir, id)
}

/// Spec 139 R2/R5(a) through the daemon's own resolver: quarantined, graph file
/// untouched -> the window closes unmet and the promotion reverts itself.
#[test]
fn the_daemon_reads_an_untouched_graph_as_unsatisfied() {
    let tmp = scratch();
    let (engine, dir, id) = promoted_sick(&tmp);
    let at = get_promotion(&dir, &id).unwrap().applied_secs;
    assert!(
        get_promotion(&dir, &id)
            .unwrap()
            .definition_at_apply
            .is_some()
    );
    let changed = promotion_sweep_at(&engine, &dir, at + 100);
    assert_eq!(changed, vec![(id, pii_learn::PromotionState::AutoReverted)]);
}

/// Spec 139 R5(b): edit the graph's definition inside the window -> met, kept.
#[test]
fn the_daemon_reads_an_edited_graph_as_satisfied() {
    let tmp = scratch();
    let (engine, dir, id) = promoted_sick(&tmp);
    let at = get_promotion(&dir, &id).unwrap().applied_secs;
    std::fs::write(
        dir.join("sick.toml"),
        SICK_V1.replace("$.input", "$.input.x"),
    )
    .unwrap();
    assert!(promotion_sweep_at(&engine, &dir, at + 100).is_empty());
    let p = get_promotion(&dir, &id).unwrap();
    assert_eq!(p.state, pii_learn::PromotionState::Active);
    assert!(p.outcome.unwrap().improved);
}

/// Spec 139 R5(c): the unattended trigger, driven without a human, counts the
/// candidates and attempts no promotion.
#[test]
fn the_unattended_pass_runs_without_an_operator_and_never_promotes() {
    let tmp = scratch();
    let dir = tmp.path().to_path_buf();
    let engine = engine_in(&dir);
    let root = dir.join("traj");
    std::fs::create_dir_all(root.join("trajectories")).unwrap();
    for r in ["r1", "r2", "r3"] {
        let l = |k: &str, f: serde_json::Value| {
            serde_json::json!({"fields":f,"kind":k,"level":"info","run":r,"session":"","span":null,"subsystem":"graph"}).to_string()
        };
        let lines = [
            l(
                "run.journal",
                serde_json::json!({"event":{"Started":{"graph":"g","input_hash":"h"}},"seq":0}),
            ),
            l(
                "run.journal",
                serde_json::json!({"event":{"NodeFailed":{"node":"fetch","error":"connection reset","retryable":false,"attempt":1}},"seq":1}),
            ),
            l(
                "run.outcome",
                serde_json::json!({"detail":"connection reset","reason":"terminal","verdict":"failure"}),
            ),
        ];
        std::fs::write(
            root.join("trajectories").join(format!("{r}.jsonl")),
            lines.join("\n") + "\n",
        )
        .unwrap();
    }
    engine.attach_learn_traj_root(root);
    let s = learn_unattended_pass(&engine, &dir).unwrap();
    assert!(s.last_candidates >= 1 && s.candidates_proposed_total == s.last_candidates);
    assert!(pii_learn::list_promotions(&dir).is_empty());
    assert!(ledger_tail(&dir, 10).is_empty());
}

/// Without a trajectory root there is nothing to learn from: a named error, not a panic.
#[test]
fn the_unattended_pass_without_a_trajectory_root_says_so() {
    let tmp = scratch();
    let engine = engine_in(tmp.path());
    let e = learn_unattended_pass(&engine, tmp.path()).unwrap_err();
    assert!(e.contains("trajectory"), "{e}");
}

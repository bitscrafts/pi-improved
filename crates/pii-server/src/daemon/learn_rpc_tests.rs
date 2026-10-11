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
use crate::engine::learn_methods::{learn_promote, learn_revert};
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
                "metric": "graph_failed_runs", "window_secs": 100, "dir": dir,
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
                "candidate": c.id, "actor": "x", "metric": "graph_failed_runs",
                "window_secs": 100, "dir": other.path(),
            }),
        ),
    );
    let e = r.error.expect("must be refused");
    assert!(e.message.contains("the daemon owns the ledger"), "{e:?}");
    assert!(ledger_tail(&dir, 10).is_empty(), "nothing was written");
}

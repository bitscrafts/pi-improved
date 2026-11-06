//! Spec 131: a refused run is a decision, and the ledger shows it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_abi::{RunId, RunStatus};
use pii_learn::{
    Candidate, Evidence, Kind, PromoteConfig, PromoteRequest, Promoter, Promotion, Verdict,
    VerdictRecord, put_verdict, store,
};

use super::tests::test_ctx;
use crate::{Engine, EngineConfig};

struct Fixture {
    _tmp: tempfile::TempDir,
    dir: std::path::PathBuf,
    engine: Arc<Engine>,
    promo: Promotion,
}

/// An engine under a daemon context whose graph `sick` is quarantined.
fn fixture() -> Fixture {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"));
    std::fs::create_dir_all(root).unwrap();
    let tmp = tempfile::Builder::new()
        .prefix("refusal-")
        .tempdir_in(root)
        .unwrap();
    let dir = tmp.path().to_path_buf();
    let fx = dir.join("fixture.json");
    std::fs::write(&fx, r#"{"turns":[]}"#).unwrap();
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.clone(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fx.display()),
        sandbox: pii_tools::sandbox::SandboxProfileName::Off,
        graph_dir: dir.clone(),
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models: crate::daemon::empty_model_roles(),
        mqtt_tool_url: None,
    })
    .unwrap();
    engine.attach_daemon_ctx(test_ctx(&dir));
    let promo = promote_graph(&dir, "sick");
    Fixture {
        _tmp: tmp,
        dir,
        engine,
        promo,
    }
}

/// Quarantines `graph` through the real promoter and returns the promotion.
fn promote_graph(dir: &std::path::Path, graph: &str) -> Promotion {
    let c = Candidate::new(
        Kind::StuckLoop,
        format!("graph {graph} failed in all 3 runs"),
        format!("stuck_loop:{graph}:n:other"),
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
    let promoter = Promoter::new(
        dir,
        PromoteConfig::default(),
        Arc::new(pii_observe::Recorder::new()),
    );
    promoter
        .promote(
            &PromoteRequest {
                candidate: c.id,
                kind: String::from("quarantine_graph"),
                actor: String::from("test"),
                metric: Some(String::from("graph_definition_changed")),
                window_secs: Some(3_600),
                ttl_secs: None,
            },
            crate::engine::runs::learn_now_secs(),
        )
        .unwrap()
}

/// Starts `name` and waits (a condition, not a sleep) for the run to end.
fn start(fx: &Fixture, name: &str) -> (RunId, RunStatus) {
    let spec = pii_graph::toml_load::load_toml(&format!(
        "name = \"{name}\"\nentry = \"a\"\ncaps = []\n[[node]]\nid = \"a\"\nkind = \"transform\"\nexpr = \"$.input\"\n"
    ))
    .unwrap();
    let id = fx.engine.store.next_run_id().unwrap();
    crate::engine::runs::spawn_run(
        Arc::clone(&fx.engine),
        spec,
        id.clone(),
        String::new(),
        String::from("s"),
        None,
    )
    .unwrap();
    for _ in 0..6000 {
        if let Ok(Some(h)) = fx.engine.store.get_run(&id)
            && h.status != RunStatus::Running
        {
            return (id, h.status);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("run {id} never finished within 60s");
}

fn heal_path(fx: &Fixture) -> std::path::PathBuf {
    fx.dir.join("heal.redb")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_run_is_recorded_as_a_deny_decision() {
    let fx = fixture();
    let (id, status) = start(&fx, "sick");
    assert_eq!(status, RunStatus::Failed);
    let a = fx
        .engine
        .heals
        .get_attempt(&fx.promo.id)
        .unwrap()
        .expect("a deny decision keyed by the promotion id");
    let d = a.decision.expect("decision");
    assert!(!d.allow, "a denial");
    assert_eq!(d.by, "engine");
    assert!(d.approval_id.is_empty(), "the run id is not an approval");
    assert_eq!(a.gate, "quarantine");
    assert_eq!(a.extra["graph"], "sick");
    assert_eq!(a.extra["run_id"], id.as_str());
    assert_eq!(
        a.extra["reason"],
        format!(
            "learn: graph sick is quarantined by promotion {}",
            fx.promo.id
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_recorded_decision_survives_a_restart() {
    let fx = fixture();
    let (id, _) = start(&fx, "sick");
    let path = heal_path(&fx);
    let promo_id = fx.promo.id.clone();
    // The run's blocking task holds a clone until it unregisters; wait it out.
    let mut engine = fx.engine;
    for _ in 0..6000 {
        match Arc::try_unwrap(engine) {
            Ok(e) => {
                drop(e);
                break;
            }
            Err(still) => {
                engine = still;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
    // A fresh reader: the engine (and its handle on the redb) is gone.
    let reader = pii_store::heal::HealStore::open(&path).unwrap();
    let a = reader.get_attempt(&promo_id).unwrap().expect("persisted");
    assert!(!a.decision.unwrap().allow);
    assert_eq!(a.extra["run_id"], id.as_str());
    assert_eq!(a.extra["graph"], "sick");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_allowed_run_writes_no_decision() {
    let fx = fixture();
    let (_, status) = start(&fx, "healthy");
    assert_ne!(status, RunStatus::Failed);
    assert_eq!(
        fx.engine.heals.list_attempts().unwrap(),
        [] as [pii_store::heal::HealAttempt; 0]
    );
    assert_eq!(fx.engine.refusals.refused(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ledger_write_failure_does_not_swallow_the_refusal() {
    let fx = fixture();
    let rec = Arc::new(pii_core::RecordingObserver::new());
    fx.engine.attach_observer(rec.clone());
    fx.engine.refusals.set_fail_record_for_test(true);
    let (id, status) = start(&fx, "sick");
    assert_eq!(status, RunStatus::Failed, "the refusal stands");
    let journal = format!("{:?}", fx.engine.store.read_journal(&id).unwrap());
    assert!(journal.contains("quarantined by promotion"), "{journal}");
    assert_eq!(fx.engine.refusals.refused(), 1);
    assert_eq!(fx.engine.refusals.recorded(), 0);
    assert_eq!(
        fx.engine.heals.list_attempts().unwrap(),
        [] as [pii_store::heal::HealAttempt; 0]
    );
    assert!(
        format!("{:?}", rec.events()).contains("quarantine.refusal.record"),
        "the failure is visible: {:?}",
        rec.events()
    );
}

/// `sys.status` (what `pii metrics` renders) carries both counters, and an
/// ordinary refusal leaves them equal.
#[tokio::test(flavor = "multi_thread")]
async fn status_counts_refusals_and_recorded_refusals_equal_after_a_refusal() {
    let fx = fixture();
    let _ = start(&fx, "sick");
    let mut out = pii_proto::sys::StatusResult::default();
    super::identity::attach(&fx.engine, None, &mut out);
    assert_eq!(out.counts.quarantine_refusals, 1);
    assert_eq!(out.counts.quarantine_refusals_recorded, 1);
}

fn row(fx: &Fixture, id: &str) -> pii_store::heal::HealAttempt {
    fx.engine.heals.get_attempt(id).unwrap().expect("row")
}

/// The defect measured live: N refusals must leave N entries, not the last one.
#[tokio::test(flavor = "multi_thread")]
async fn every_refusal_under_a_promotion_stays_in_its_row() {
    let fx = fixture();
    let ids: Vec<String> = (0..3)
        .map(|_| start(&fx, "sick").0.as_str().to_owned())
        .collect();
    let a = row(&fx, &fx.promo.id);
    let held: Vec<&str> = a.attempts.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(held, ids.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(a.attempt_count, 3);
    assert!(
        a.attempts
            .iter()
            .all(|r| r.outcome == pii_store::heal::HealState::Denied)
    );
    assert_eq!(a.extra["dropped"], 0);
    // The counter agrees with the row.
    assert_eq!(fx.engine.refusals.refused(), u64::from(a.attempt_count));
    assert_eq!(fx.engine.refusals.recorded(), u64::from(a.attempt_count));
}

/// Near-miss control: one refusal is exactly one entry.
#[tokio::test(flavor = "multi_thread")]
async fn one_refusal_is_exactly_one_entry() {
    let fx = fixture();
    let (id, _) = start(&fx, "sick");
    let a = row(&fx, &fx.promo.id);
    assert_eq!(a.attempt_count, 1);
    assert_eq!(a.attempts.len(), 1);
    assert_eq!(a.attempts[0].run_id, id.as_str());
}

/// The join still works: another promotion owns another row.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_promotion_has_its_own_row() {
    let fx = fixture();
    let other = promote_graph(&fx.dir, "ill");
    assert_ne!(other.id, fx.promo.id);
    let (a_run, _) = start(&fx, "sick");
    let (b_run, _) = start(&fx, "ill");
    let a = row(&fx, &fx.promo.id);
    let b = row(&fx, &other.id);
    assert_eq!(a.attempts.len(), 1);
    assert_eq!(b.attempts.len(), 1);
    assert_eq!(a.attempts[0].run_id, a_run.as_str());
    assert_eq!(b.attempts[0].run_id, b_run.as_str());
    assert_eq!(fx.engine.heals.list_attempts().unwrap().len(), 2);
}

/// Seeds a run of the graph `name` that has started but not finished: the
/// shape a resume picks up.
fn seed_started(fx: &Fixture, name: &str) -> RunId {
    use pii_abi::{RunEvent, RunHeader};
    let spec = pii_graph::toml_load::load_toml(&format!(
        "name = \"{name}\"\nentry = \"a\"\ncaps = []\n[[node]]\nid = \"a\"\nkind = \"transform\"\nexpr = \"$.input\"\n"
    ))
    .unwrap();
    let store = &fx.engine.store;
    let graph = store
        .put_graph(&serde_json::to_vec(&spec).unwrap())
        .unwrap();
    let input_hash = store.put_graph(br#"{"prompt":"resume"}"#).unwrap();
    let id = store.next_run_id().unwrap();
    let mut header = RunHeader {
        run: id.clone(),
        graph: graph.clone(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    };
    store
        .append_event(&mut header, &RunEvent::Started { graph, input_hash })
        .unwrap();
    id
}

/// A run created before the quarantine, resumed after it: refused, terminal,
/// recorded, and not deleted.
#[tokio::test(flavor = "multi_thread")]
async fn a_resumed_run_of_a_quarantined_graph_is_refused() {
    let fx = fixture();
    let id = seed_started(&fx, "sick");
    let err = crate::engine::runs::spawn_resume(Arc::clone(&fx.engine), id.clone(), None)
        .expect_err("a quarantined graph must not resume");
    assert!(matches!(err, pii_abi::PiiError::Denied(_)), "{err:?}");
    let header = fx.engine.store.get_run(&id).unwrap().expect("not deleted");
    assert_eq!(header.status, RunStatus::Failed, "terminal, not in flight");
    let journal = format!("{:?}", fx.engine.store.read_journal(&id).unwrap());
    assert!(journal.contains("quarantined by promotion"), "{journal}");
    assert_eq!(fx.engine.refusals.refused(), 1);
    assert_eq!(fx.engine.refusals.recorded(), 1);
    assert!(fx.engine.runs.lock().is_empty(), "no handle left behind");
}

/// The record answers "which promotion" without reading code.
#[tokio::test(flavor = "multi_thread")]
async fn the_refusal_names_the_promotion() {
    let fx = fixture();
    let id = seed_started(&fx, "sick");
    let _ = crate::engine::runs::spawn_resume(Arc::clone(&fx.engine), id.clone(), None);
    let want = format!(
        "learn: graph sick is quarantined by promotion {}",
        fx.promo.id
    );
    let journal = format!("{:?}", fx.engine.store.read_journal(&id).unwrap());
    assert!(journal.contains(&want), "{journal}");
    let a = row(&fx, &fx.promo.id);
    assert_eq!(a.extra["graph"], "sick");
    assert_eq!(a.extra["run_id"], id.as_str());
    assert_eq!(a.extra["reason"], want);
}

/// Near-miss control: a graph with no quarantine resumes as before.
#[tokio::test(flavor = "multi_thread")]
async fn a_healthy_resume_still_works() {
    let fx = fixture();
    let id = seed_started(&fx, "healthy");
    crate::engine::runs::spawn_resume(Arc::clone(&fx.engine), id.clone(), None).unwrap();
    let mut status = RunStatus::Running;
    for _ in 0..6000 {
        status = fx.engine.store.get_run(&id).unwrap().unwrap().status;
        if status != RunStatus::Running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_ne!(status, RunStatus::Failed, "a healthy resume continues");
    assert_eq!(fx.engine.refusals.refused(), 0);
}

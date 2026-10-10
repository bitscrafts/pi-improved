//! Spec 125: promotion and lifecycle. Each test names the rule it holds; the
//! mutations in the spec report were run against exactly these assertions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::many_single_char_names
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use pii_learn::{
    Candidate, Evidence, Kind, LedgerEntry, Metric, PromoteConfig, PromoteRequest, Promoter,
    Promotion, PromotionKind, PromotionRefusal, PromotionState, RunSummary, Verdict, VerdictRecord,
    get_promotion, ledger_tail, list_promotions, promotion_counts, quarantine_reason, store,
    write_scoped,
};
use pii_observe::{Recorder, names};
use tempfile::TempDir;

const T0: u64 = 1_000_000;

fn candidate(graph: &str) -> Candidate {
    Candidate::new(
        Kind::StuckLoop,
        format!("graph {graph} failed in all 3 runs"),
        format!("stuck_loop:{graph}:n:other"),
        Evidence {
            runs: vec!["r1".into(), "r2".into(), "r3".into()],
            paths: vec!["trajectories/r1.jsonl".into(); 3],
            counts: BTreeMap::from([("runs".to_owned(), 3)]),
        },
    )
    .unwrap()
}

fn verdict_rec(c: &Candidate, v: Verdict) -> VerdictRecord {
    VerdictRecord {
        candidate: c.id.clone(),
        verdict: v,
        reason: None,
        detail: "fixture".into(),
        shadow: "s-1".into(),
        shadow_runs: vec![],
        numbers: BTreeMap::new(),
        evidence_used: vec![],
        missing: vec![],
        bound: None,
        elapsed_ms: 0,
    }
}

/// A store dir holding a candidate for `graph`, with `verdict` recorded if given.
fn fixture(graph: &str, verdict: Option<Verdict>) -> (TempDir, Candidate) {
    let d = TempDir::new().unwrap();
    let c = candidate(graph);
    store::put_candidates(d.path(), std::slice::from_ref(&c)).unwrap();
    if let Some(v) = verdict {
        pii_learn::put_verdict(d.path(), &verdict_rec(&c, v)).unwrap();
    }
    (d, c)
}

fn promoter(dir: &Path) -> (Promoter, Arc<Recorder>) {
    let rec = Arc::new(Recorder::new());
    (
        Promoter::new(dir, PromoteConfig::default(), rec.clone()),
        rec,
    )
}

fn req(c: &Candidate) -> PromoteRequest {
    PromoteRequest {
        candidate: c.id.clone(),
        kind: "quarantine_graph".into(),
        actor: "test:operator".into(),
        metric: Some("graph_failed_runs".into()),
        window_secs: Some(100),
        ttl_secs: None,
    }
}

fn refusal_of(dir: &Path, r: &PromoteRequest, now: u64) -> PromotionRefusal {
    promoter(dir).0.promote(r, now).unwrap_err().reason
}

fn run(graph: &str, verdict: &str, mtime: u64) -> RunSummary {
    RunSummary {
        run: format!("x{mtime}"),
        graph: graph.into(),
        verdict: verdict.into(),
        mtime_secs: mtime,
        ..RunSummary::default()
    }
}

fn nothing_changed(dir: &Path) {
    assert!(list_promotions(dir).is_empty(), "a promotion was stored");
    assert_eq!(quarantine_reason(dir, "g", T0), None);
}

#[test]
fn r2_no_verdict_refuses_and_changes_nothing() {
    let (d, c) = fixture("g", None);
    assert_eq!(
        refusal_of(d.path(), &req(&c), T0),
        PromotionRefusal::NoVerdict
    );
    nothing_changed(d.path());
    assert_eq!(promotion_counts(d.path())["refused"], 1);
}

#[test]
fn r2_refuted_and_inconclusive_are_refused_with_their_reason() {
    for (v, want) in [
        (Verdict::Refuted, PromotionRefusal::Refuted),
        (Verdict::Inconclusive, PromotionRefusal::Inconclusive),
    ] {
        let (d, c) = fixture("g", Some(v));
        assert_eq!(refusal_of(d.path(), &req(&c), T0), want);
        nothing_changed(d.path());
    }
}

#[test]
fn r2_the_verdict_is_reread_so_a_later_refutation_blocks_a_promotion() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    pii_learn::put_verdict(d.path(), &verdict_rec(&c, Verdict::Refuted)).unwrap();
    assert_eq!(
        refusal_of(d.path(), &req(&c), T0),
        PromotionRefusal::Refuted
    );
}

#[test]
fn r1_r3_r4_a_confirmed_candidate_is_applied_with_ledger_and_rollback() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    let (p, rec) = promoter(d.path());
    let applied = p.promote(&req(&c), T0).unwrap();
    assert_eq!(applied.state, PromotionState::Active);
    assert_eq!(applied.kind, PromotionKind::QuarantineGraph);
    assert_eq!(applied.graph, "g");
    assert_eq!(applied.expires_secs, T0 + 86_400, "default ttl is 24h");
    // R4: rollback data is on disk, not only in memory.
    let stored = get_promotion(d.path(), &applied.id).unwrap();
    let rb = stored.rollback.expect("rollback stored");
    assert!(!rb.was_quarantined);
    assert_eq!(rb.graph, "g");
    // R3: the ledger holds the id, kind, graph, candidate+verdict, evidence, actor, transition.
    let l: Vec<LedgerEntry> = ledger_tail(d.path(), 10);
    assert_eq!(l.len(), 1);
    let e = &l[0];
    assert_eq!(e.event, "applied");
    assert_eq!(e.promotion, applied.id);
    assert_eq!(e.kind, "quarantine_graph");
    assert_eq!(e.graph, "g");
    assert_eq!(e.candidate, c.id);
    assert_eq!(e.verdict, Some(Verdict::Confirmed));
    assert_eq!(e.evidence.as_ref().unwrap(), &c.evidence);
    assert_eq!(e.actor, "test:operator");
    assert_eq!((e.from, e.to), (None, Some(PromotionState::Active)));
    let ev = rec.events();
    assert!(ev.iter().any(|e| e.kind == names::LEARN_PROMOTION_APPLIED));
}

#[test]
fn r1_quarantine_actually_refuses_that_graph_with_the_named_reason() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    let a = promoter(d.path()).0.promote(&req(&c), T0).unwrap();
    assert_eq!(
        quarantine_reason(d.path(), "g", T0 + 1),
        Some(format!(
            "learn: graph g is quarantined by promotion {}",
            a.id
        ))
    );
    assert_eq!(quarantine_reason(d.path(), "other", T0 + 1), None);
}

#[test]
fn r4_revert_restores_the_graph_and_records_it() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    let (p, rec) = promoter(d.path());
    let a = p.promote(&req(&c), T0).unwrap();
    let lid = p.revert(&a.id, "test:operator", T0 + 5).unwrap();
    assert_eq!(lid, "L2");
    assert_eq!(quarantine_reason(d.path(), "g", T0 + 6), None);
    assert_eq!(
        get_promotion(d.path(), &a.id).unwrap().state,
        PromotionState::Reverted
    );
    let l = ledger_tail(d.path(), 10);
    assert_eq!(l[1].event, "reverted");
    assert_eq!(
        (l[1].from, l[1].to),
        (Some(PromotionState::Active), Some(PromotionState::Reverted))
    );
    assert!(
        rec.events()
            .iter()
            .any(|e| e.kind == names::LEARN_PROMOTION_REVERTED)
    );
    assert!(p.revert(&a.id, "x", T0 + 7).is_err(), "no double revert");
    assert!(p.revert("p-0000000000000000", "x", T0).is_err());
}

#[test]
fn r4_ttl_lapses_the_promotion_and_the_graph_is_usable_again() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    let (p, rec) = promoter(d.path());
    let mut r = req(&c);
    r.ttl_secs = Some(50);
    r.window_secs = Some(1_000_000);
    let a = p.promote(&r, T0).unwrap();
    assert!(quarantine_reason(d.path(), "g", T0 + 49).is_some());
    // Past the TTL the graph is usable even before any sweep has recorded it.
    assert_eq!(quarantine_reason(d.path(), "g", T0 + 50), None);
    let changed = p.sweep(T0 + 60, Vec::new);
    assert_eq!(changed, vec![(a.id.clone(), PromotionState::Expired)]);
    assert_eq!(
        get_promotion(d.path(), &a.id).unwrap().state,
        PromotionState::Expired
    );
    assert_eq!(ledger_tail(d.path(), 10)[1].event, "expired");
    assert!(
        rec.events()
            .iter()
            .any(|e| e.kind == names::LEARN_PROMOTION_EXPIRED)
    );
}

#[test]
fn r5_a_window_that_closes_without_improvement_reverts_automatically_with_both_numbers() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    let (p, rec) = promoter(d.path());
    let a = p.promote(&req(&c), T0).unwrap();
    // 1 failure before the promotion, 3 after it: worse.
    let runs = vec![
        run("g", "failure", T0 - 10),
        run("g", "failure", T0 + 10),
        run("g", "failure", T0 + 20),
        run("g", "failure", T0 + 30),
        run("other", "failure", T0 + 40),
    ];
    assert!(
        p.sweep(T0 + 99, || runs.clone()).is_empty(),
        "window still open"
    );
    assert!(quarantine_reason(d.path(), "g", T0 + 99).is_some());
    let changed = p.sweep(T0 + 100, || runs.clone());
    assert_eq!(changed, vec![(a.id.clone(), PromotionState::AutoReverted)]);
    let s: Promotion = get_promotion(d.path(), &a.id).unwrap();
    assert_eq!(s.state, PromotionState::AutoReverted);
    let o = s.outcome.unwrap();
    assert_eq!((o.before, o.after, o.improved), (1, 3, false));
    assert_eq!(quarantine_reason(d.path(), "g", T0 + 101), None);
    let l = ledger_tail(d.path(), 10);
    assert_eq!(l[1].event, "auto_reverted");
    assert_eq!((l[1].before, l[1].after), (Some(1), Some(3)));
    let ev = rec.events();
    let e = ev
        .iter()
        .find(|e| e.kind == names::LEARN_PROMOTION_REVERTED)
        .expect("reverted event");
    assert!(e.fields.get("before").is_some() && e.fields.get("after").is_some());
}

#[test]
fn r5_a_window_that_improves_keeps_the_promotion_and_records_the_numbers() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    let (p, _) = promoter(d.path());
    let a = p.promote(&req(&c), T0).unwrap();
    let runs = vec![run("g", "failure", T0 - 30), run("g", "failure", T0 - 20)];
    assert!(p.sweep(T0 + 100, || runs.clone()).is_empty());
    let s = get_promotion(d.path(), &a.id).unwrap();
    assert_eq!(s.state, PromotionState::Active);
    let o = s.outcome.unwrap();
    assert_eq!((o.before, o.after, o.improved), (2, 0, true));
    assert!(quarantine_reason(d.path(), "g", T0 + 101).is_some());
    assert_eq!(ledger_tail(d.path(), 10)[1].event, "outcome_kept");
}

#[test]
fn r5_a_window_with_nothing_to_improve_is_not_a_win() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    let (p, _) = promoter(d.path());
    let a = p.promote(&req(&c), T0).unwrap();
    // The graph was not failing in the window before: 0 -> 0 is not an improvement.
    let changed = p.sweep(T0 + 100, Vec::new);
    assert_eq!(changed, vec![(a.id, PromotionState::AutoReverted)]);
}

#[test]
fn r5_an_unmeasurable_promotion_is_refused_at_apply_time() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    let mut no_metric = req(&c);
    no_metric.metric = None;
    let mut bad_metric = req(&c);
    bad_metric.metric = Some("vibes".into());
    let mut no_window = req(&c);
    no_window.window_secs = None;
    let mut zero_window = req(&c);
    zero_window.window_secs = Some(0);
    for r in [no_metric, bad_metric, no_window, zero_window] {
        assert_eq!(refusal_of(d.path(), &r, T0), PromotionRefusal::Unmeasurable);
    }
    nothing_changed(d.path());
    assert_eq!(
        Metric::parse("graph_failed_runs"),
        Some(Metric::GraphFailedRuns)
    );
}

#[test]
fn r6_the_whitelist_has_one_kind_and_refuses_the_rest_by_name() {
    assert_eq!(PromotionKind::ALL.len(), 1);
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    for kind in [
        "edit_config",
        "rotate_secret",
        "change_gate",
        "grant_capability",
        "",
    ] {
        let mut r = req(&c);
        r.kind = kind.into();
        let e = promoter(d.path()).0.promote(&r, T0).unwrap_err();
        assert_eq!(e.reason, PromotionRefusal::UnknownKind);
        assert_eq!(
            e.message,
            format!("learn: promotion kind {kind} is not whitelisted (only quarantine_graph)")
        );
    }
    nothing_changed(d.path());
}

#[test]
fn r6_every_write_is_confined_to_the_promotion_area() {
    let (d, _) = fixture("g", None);
    let outside = d.path().join("config.toml");
    let e = write_scoped(d.path(), &outside, b"x").unwrap_err();
    assert!(e.0.contains("outside its own store area"), "{e}");
    assert!(!outside.exists());
    let dotdot = d.path().join("learn").join("..").join("gate.sh");
    assert!(write_scoped(d.path(), &dotdot, b"x").is_err());
    assert!(!d.path().join("gate.sh").exists());
    // A store dir that is itself spelled with `..` is not an escape (found by the engine test).
    let spelled = d.path().join("learn").join("x").join("..").join("..");
    write_scoped(&spelled, &spelled.join("learn").join("ok.json"), b"{}").unwrap();
    let inside = d.path().join("learn").join("promotions").join("p-x.json");
    write_scoped(d.path(), &inside, b"{}").unwrap();
    // A symlink inside the area that points outside is refused too.
    let target = d.path().join("secret.env");
    fs::write(&target, "K=1").unwrap();
    let link = d.path().join("learn").join("link.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(write_scoped(d.path(), &link, b"overwrite").is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "K=1");
}

#[test]
fn r3_a_ledger_that_cannot_be_written_means_nothing_is_applied() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    // A directory where the ledger file belongs: the append must fail.
    fs::create_dir_all(d.path().join("learn").join("ledger.jsonl")).unwrap();
    assert_eq!(refusal_of(d.path(), &req(&c), T0), PromotionRefusal::Store);
    assert!(list_promotions(d.path()).is_empty());
    assert_eq!(quarantine_reason(d.path(), "g", T0 + 1), None);
}

#[test]
fn r4_a_promotion_without_rollback_data_is_never_in_effect() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    let (p, _) = promoter(d.path());
    let mut a = p.promote(&req(&c), T0).unwrap();
    a.rollback = None;
    let path = d
        .path()
        .join("learn/promotions")
        .join(format!("{}.json", a.id));
    fs::write(&path, serde_json::to_vec(&a).unwrap()).unwrap();
    assert_eq!(quarantine_reason(d.path(), "g", T0 + 1), None);
    assert!(p.revert(&a.id, "x", T0 + 1).is_err());
}

#[test]
fn r1_a_second_promotion_on_the_same_graph_is_refused_while_one_is_in_effect() {
    let (d, c) = fixture("g", Some(Verdict::Confirmed));
    let (p, _) = promoter(d.path());
    p.promote(&req(&c), T0).unwrap();
    assert_eq!(
        refusal_of(d.path(), &req(&c), T0 + 1),
        PromotionRefusal::AlreadyActive
    );
}

#[test]
fn r7_refusals_are_events_and_counters_by_state_are_complete() {
    let (d, c) = fixture("g", None);
    let (p, rec) = promoter(d.path());
    p.promote(&req(&c), T0).unwrap_err();
    let ev = rec.events();
    let e = ev
        .iter()
        .find(|e| e.kind == names::LEARN_PROMOTION_REFUSED)
        .expect("refused event");
    assert!(e.fields.get("reason").is_some());
    let counts = promotion_counts(d.path());
    for k in ["active", "reverted", "expired", "auto_reverted", "refused"] {
        assert!(counts.contains_key(k), "{k}");
    }
    for n in [
        names::LEARN_PROMOTION_APPLIED,
        names::LEARN_PROMOTION_REVERTED,
        names::LEARN_PROMOTION_EXPIRED,
        names::LEARN_PROMOTION_REFUSED,
    ] {
        assert!(names::ALL.contains(&n), "{n} not in ALL");
    }
}

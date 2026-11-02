//! Spec 138 R1/R5(b): `pii learn promote|promotions --sweep` write
//! nothing themselves — the daemon is the ledger's only writer — so with no
//! daemon they fail closed, name the daemon, and leave `learn/` untouched.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads its stderr.
#![allow(clippy::disallowed_methods)]

fn pii(args: &[&str], runtime: &std::path::Path) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .env("PII_RUNTIME_DIR", runtime)
        // State (and its durable stores) must be this test's own: the operator's are
        // held by any daemon already running on the machine.
        .env("PII_STATE_DIR", runtime)
        .output()
        .unwrap()
}

#[test]
fn the_cli_refuses_to_write_the_ledger_without_the_daemon() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().to_str().unwrap();
    for args in [
        vec![
            "learn",
            "promote",
            "c-0",
            "--metric",
            "graph_definition_changed",
            "--window-secs",
            "10",
            "--dir",
            dir,
        ],
        vec!["learn", "promotions", "--sweep", "--dir", dir],
    ] {
        let out = pii(&args, d.path());
        assert_ne!(out.status.code(), Some(0), "{args:?} must fail: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("the daemon owns the ledger"),
            "{args:?} must name the daemon as the writer, got: {err}"
        );
    }
    assert!(
        !d.path().join("learn/ledger.jsonl").exists(),
        "no ledger may be written without the daemon"
    );
}

fn promoted(dir: &std::path::Path) -> String {
    use pii_learn::{Candidate, Evidence, Kind, PromoteConfig, PromoteRequest, Promoter, store};
    let c = Candidate::new(
        Kind::StuckLoop,
        "graph g failed in all 3 runs".into(),
        "stuck_loop:g:n:other".into(),
        Evidence {
            runs: vec!["r1".into(), "r2".into(), "r3".into()],
            paths: vec!["trajectories/r1.jsonl".into(); 3],
            counts: std::collections::BTreeMap::from([("runs".to_owned(), 3)]),
        },
    )
    .unwrap();
    store::put_candidates(dir, std::slice::from_ref(&c)).unwrap();
    pii_learn::put_verdict(
        dir,
        &pii_learn::VerdictRecord {
            candidate: c.id.clone(),
            verdict: pii_learn::Verdict::Confirmed,
            reason: None,
            detail: "fixture".into(),
            shadow: "s-1".into(),
            shadow_runs: vec![],
            numbers: std::collections::BTreeMap::new(),
            evidence_used: vec![],
            missing: vec![],
            bound: None,
            elapsed_ms: 0,
        },
    )
    .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    Promoter::new(
        dir,
        PromoteConfig::default(),
        std::sync::Arc::new(pii_observe::Recorder::new()),
    )
    .promote(
        &PromoteRequest {
            candidate: c.id,
            kind: "quarantine_graph".into(),
            actor: "test:operator".into(),
            metric: Some("graph_definition_changed".into()),
            window_secs: Some(100),
            ttl_secs: None,
        },
        now,
    )
    .unwrap()
    .id
}

/// Spec 149 R2/R3/R4: with no daemon the CLI applies the revert itself, says
/// which path it took, records itself as the `cli` principal, and a second
/// revert is "already reverted" with no second ledger line.
#[test]
fn a_cli_revert_applies_directly_without_a_daemon() {
    let d = tempfile::tempdir().unwrap();
    let id = promoted(d.path());
    let dir = d.path().to_str().unwrap();
    let out = pii(&["learn", "revert", &id, "--dir", dir], d.path());
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(so.contains("path=direct"), "must name the path taken: {so}");
    let l = pii_learn::ledger_tail(d.path(), 10);
    assert_eq!(l.len(), 2);
    let last = l.last().unwrap();
    assert_eq!(last.event, "reverted");
    assert!(last.actor.starts_with("cli"), "{}", last.actor);
    let again = pii(&["learn", "revert", &id, "--dir", dir], d.path());
    assert_ne!(again.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&again.stderr).contains("already reverted"));
    assert_eq!(pii_learn::ledger_tail(d.path(), 10).len(), 2);
}

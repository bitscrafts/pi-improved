//! Spec 139: extraction and verification run without an operator, and never promote.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use pii_learn::{
    LearnConfig, UnattendedConfig, VerifyConfig, get_verdict, ledger_tail, list_promotions,
    read_unattended_status, run_unattended, store,
};
use pii_observe::{Recorder, names};
use serde_json::{Value, json};
use tempfile::TempDir;

fn line(run: &str, kind: &str, fields: &Value) -> String {
    json!({"fields":fields,"kind":kind,"level":"info","run":run,"session":"","span":null,"subsystem":"graph"})
        .to_string()
}

/// A failing run of graph `g` at node `fetch`.
fn failing(root: &Path, run: &str) {
    let dir = root.join("trajectories");
    fs::create_dir_all(&dir).unwrap();
    let lines = [
        line(
            run,
            "run.journal",
            &json!({"event":{"Started":{"graph":"g","input_hash":"h"}},"seq":0}),
        ),
        line(
            run,
            "run.journal",
            &json!({"event":{"NodeFailed":{"node":"fetch","error":"connection reset","retryable":false,"attempt":1}},"seq":1}),
        ),
        line(
            run,
            "run.outcome",
            &json!({"detail":"connection reset","reason":"terminal","verdict":"failure"}),
        ),
    ];
    fs::write(dir.join(format!("{run}.jsonl")), lines.join("\n") + "\n").unwrap();
}

fn cfg(t: &TempDir, root: &Path) -> UnattendedConfig {
    let mut v = VerifyConfig::new(root.to_path_buf(), t.path().join("graphs"));
    v.scratch_dir = t.path().join("scratch");
    UnattendedConfig {
        learn: LearnConfig::default(),
        verify: v,
    }
}

#[test]
fn extraction_and_verification_run_without_an_operator() {
    let t = TempDir::new().unwrap();
    let (root, dir) = (t.path().join("traj"), t.path().join("store"));
    for r in ["r1", "r2", "r3"] {
        failing(&root, r);
    }
    let rec = Arc::new(Recorder::new());
    let obs: Arc<dyn pii_observe::Observer> = rec.clone();
    let s = run_unattended(&root, &dir, &cfg(&t, &root), &obs, 1_000).unwrap();
    // Candidates are counted, each one verified, and nothing was promoted.
    assert_eq!(
        (s.runs, s.last_candidates, s.candidates_proposed_total),
        (1, 2, 2)
    );
    assert_eq!(s.last_verified, 2);
    let c = store::list(&dir).0;
    assert_eq!(c.len(), 2);
    for k in &c {
        assert!(
            get_verdict(&dir, &k.id).is_some(),
            "verified, not just proposed"
        );
    }
    assert!(
        list_promotions(&dir).is_empty(),
        "the loop must not promote"
    );
    assert!(
        ledger_tail(&dir, 10).is_empty(),
        "no ledger line without a human"
    );
    assert!(
        rec.events()
            .iter()
            .any(|e| e.kind == names::LEARN_UNATTENDED_RUN)
    );
    assert_eq!(read_unattended_status(&dir), Some(s));
}

#[test]
fn a_second_pass_over_the_same_evidence_proposes_nothing_new_and_says_so() {
    let t = TempDir::new().unwrap();
    let (root, dir) = (t.path().join("traj"), t.path().join("store"));
    for r in ["r1", "r2", "r3"] {
        failing(&root, r);
    }
    let rec = Arc::new(Recorder::new());
    let obs: Arc<dyn pii_observe::Observer> = rec.clone();
    run_unattended(&root, &dir, &cfg(&t, &root), &obs, 1_000).unwrap();
    let s = run_unattended(&root, &dir, &cfg(&t, &root), &obs, 2_000).unwrap();
    // The same lesson is not "proposed" twice: a quiet fortnight reads as a flat zero.
    assert_eq!(
        (s.runs, s.candidates_proposed_total, s.last_verified),
        (2, 2, 0)
    );
    assert_eq!(s.last_at_secs, 2_000);
    let ev = rec
        .events()
        .into_iter()
        .filter(|e| e.kind == names::LEARN_UNATTENDED_RUN)
        .count();
    assert_eq!(ev, 2, "one event per pass, even a silent one");
}

//! Spec 185: the criteria decide a worker node's verdict; outcomes are learned.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_server::node_kinds::nodes::agent_worker::{CriterionRun, Verdict, decide, learn_fact};

fn c(cmd: &str, rc: i32) -> CriterionRun {
    CriterionRun {
        command: cmd.into(),
        rc,
    }
}

#[test]
fn worker_exit_zero_with_failing_criterion_is_failed_and_names_it() {
    let v = decide(0, "report", &[c("cargo test -p a", 0), c("grep -q x f", 1)]);
    match v {
        Verdict::Failed { reason, failing } => {
            assert_eq!(failing.as_deref(), Some("grep -q x f"));
            assert!(reason.contains("grep -q x f"));
        }
        Verdict::Passed { .. } => panic!("must fail"),
    }
}

#[test]
fn all_criteria_green_passes_and_carries_report_and_runs() {
    match decide(0, "done", &[c("a", 0)]) {
        Verdict::Passed { report, criteria } => {
            assert_eq!(report, "done");
            assert_eq!(criteria.len(), 1);
        }
        Verdict::Failed { .. } => panic!("must pass"),
    }
}

#[test]
fn nonzero_worker_exit_fails_even_if_criteria_pass() {
    assert!(matches!(
        decide(2, "r", &[c("a", 0)]),
        Verdict::Failed { .. }
    ));
}

#[test]
fn no_criteria_cannot_pass() {
    assert!(matches!(decide(0, "r", &[]), Verdict::Failed { .. }));
}

#[test]
fn learn_writes_one_item_carrying_agent_model_and_failing_criterion() {
    let v = decide(0, "r", &[c("cargo test", 101)]);
    let rows = learn_fact("185", "pi", "free-model", &v, 1000);
    assert!(rows.edges.iter().any(|e| e.0 == "pi"));
    let text = String::from_utf8_lossy(&rows.doc).into_owned();
    assert!(text.contains("pi"));
    assert!(text.contains("free-model"));
    assert!(text.contains("cargo test"));
    assert!(text.contains("185"));
}

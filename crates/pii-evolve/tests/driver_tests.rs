//! R5: the boss's outer-loop skeleton — green gates return `Ok`, a red
//! gate suspends for human review, budget exhaustion stops the cycle, and
//! `state.md` always persists.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_core::Budget;
use pii_evolve::driver::{CycleOutcome, Gate, GatePolicy, run_cycle};
use pii_evolve::state::StateDoc;

fn shell_gate(name: &str, exit_code: u8) -> Gate {
    Gate {
        name: String::from(name),
        argv: vec![
            String::from("/bin/sh"),
            String::from("-c"),
            format!("exit {exit_code}"),
        ],
    }
}

/// A budget with enough room for the gates in a test (10 calls).
fn test_budget() -> Budget {
    Budget::new(10_000, 10, 0, 100_000)
}

/// A zero tool-call budget: the first gate charge must exhaust it.
fn zero_budget() -> Budget {
    Budget::new(10_000, 0, 0, 100_000)
}

#[test]
fn green_fixture_returns_ok() {
    let dir = tempfile::tempdir().expect("tempdir");
    let policy = GatePolicy {
        gates: vec![shell_gate("fake-check-a", 0), shell_gate("fake-check-b", 0)],
        workspace_root: dir.path().to_path_buf(),
    };

    let outcome = run_cycle(dir.path(), &policy, test_budget()).expect("run_cycle");
    assert_eq!(outcome, CycleOutcome::Ok);

    let state = StateDoc::load(dir.path()).expect("load");
    assert_eq!(
        state.properties.get("cycle_count").map(String::as_str),
        Some("1"),
        "cycle counter must bump"
    );
    assert!(
        !state.properties.contains_key("needs_attention"),
        "a green cycle must not need attention"
    );
}

#[test]
fn red_fixture_suspends_for_human() {
    let dir = tempfile::tempdir().expect("tempdir");
    let policy = GatePolicy {
        gates: vec![shell_gate("fake-check-red", 1)],
        workspace_root: dir.path().to_path_buf(),
    };

    let outcome = run_cycle(dir.path(), &policy, test_budget()).expect("run_cycle");
    assert_eq!(
        outcome,
        CycleOutcome::PendingHuman(String::from("fake-check-red"))
    );

    let state = StateDoc::load(dir.path()).expect("load");
    assert_eq!(
        state.properties.get("needs_attention").map(String::as_str),
        Some("true"),
        "a red gate must persist needs_attention"
    );
    assert_eq!(
        state.properties.get("cycle_count").map(String::as_str),
        Some("1"),
        "cycle counter must bump even when the cycle suspends"
    );
}

/// R4 (spec 25): a zero tool-call budget must stop the cycle as
/// `BudgetExhausted`, never spend unboundedly.
#[test]
fn run_cycle_zero_budget_exhausts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let policy = GatePolicy {
        gates: vec![shell_gate("fake-check-a", 0)],
        workspace_root: dir.path().to_path_buf(),
    };

    let outcome = run_cycle(dir.path(), &policy, zero_budget()).expect("run_cycle");
    assert_eq!(
        outcome,
        CycleOutcome::BudgetExhausted,
        "a zero tool-call budget must exhaust on the first gate charge"
    );

    let state = StateDoc::load(dir.path()).expect("load");
    assert_eq!(
        state.properties.get("cycle_count").map(String::as_str),
        Some("1"),
        "cycle counter must bump even on budget exhaustion"
    );
    assert_eq!(
        state.phase,
        String::from("budget_exhausted"),
        "state.md records the budget stop"
    );
}

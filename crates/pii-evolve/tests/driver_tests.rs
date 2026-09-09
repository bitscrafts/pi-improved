//! R5: the boss's outer-loop skeleton — green gates return `Ok`, a red
//! gate suspends for human review, and `state.md` always persists.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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

#[test]
fn green_fixture_returns_ok() {
    let dir = tempfile::tempdir().expect("tempdir");
    let policy = GatePolicy {
        gates: vec![shell_gate("fake-check-a", 0), shell_gate("fake-check-b", 0)],
        workspace_root: dir.path().to_path_buf(),
    };

    let outcome = run_cycle(dir.path(), &policy).expect("run_cycle");
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

    let outcome = run_cycle(dir.path(), &policy).expect("run_cycle");
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

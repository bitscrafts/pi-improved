//! A spend-limit message from an agent is journalled as a quota signal (spec 237). The fixtures are the CLI's own words,
//! copied from `scripts/autonomy/window.sh`. The controls must stay quiet: text that only mentions quota, a plain error,
//! and no output at all are not quota exhaustion.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::node_kinds::nodes::agent_quota::quota_signal;
use pii_server::node_kinds::nodes::coder::report::RunReport;

const SESSION_LIMIT: &str = "You've hit your session limit · resets 4:50pm (UTC)";

#[test]
fn a_session_limit_is_a_quota_signal() {
    let signal = quota_signal(SESSION_LIMIT, "").unwrap();
    assert_eq!(signal.reset_text, "resets 4:50pm (UTC)");
}

#[test]
fn a_weekly_and_a_monthly_limit_are_too() {
    assert!(quota_signal("hit your weekly limit", "").is_some());
    assert!(quota_signal("hit your monthly limit", "").is_some());
}

#[test]
fn a_429_status_is_a_signal() {
    let signal = quota_signal(r#"{"api_error_status":429}"#, "").unwrap();
    assert_eq!(signal.reset_text, "");
}

#[test]
fn a_rate_limit_error_is_a_signal() {
    assert!(quota_signal(r#"{"type":"rate_limit_error"}"#, "").is_some());
}

#[test]
fn the_signal_is_found_in_stderr_too() {
    let signal = quota_signal("", SESSION_LIMIT).unwrap();
    assert_eq!(signal.reset_text, "resets 4:50pm (UTC)");
}

#[test]
fn a_brief_that_mentions_quota_is_not_a_signal() {
    assert!(quota_signal("Reply about the quota and the rate limit policy", "").is_none());
}

#[test]
fn a_plain_error_is_not_a_signal() {
    assert!(quota_signal("Error: Provider error: HTTP 404 model not found", "").is_none());
}

#[test]
fn empty_output_is_not_a_signal() {
    assert!(quota_signal("", "").is_none());
}

/// The `coder.exited` event for a run whose captured stdout is `output` and whose stderr is empty.
fn exited_for(output: &str) -> pii_observe::Event {
    let report = RunReport {
        tree: None,
        requested: None,
        runs: &[],
        failure_tail: "",
        routes: &[],
        agent_output: output,
        agent_stderr: "",
    };
    report.exited_event(42, 1, 1000, "end_turn")
}

#[test]
fn the_event_carries_the_fields() {
    let event = exited_for(SESSION_LIMIT);
    assert_eq!(
        event.fields.get("quota_exhausted"),
        Some(&serde_json::json!("yes"))
    );
    assert_eq!(
        event.fields.get("quota_reset"),
        Some(&serde_json::json!("resets 4:50pm (UTC)"))
    );
}

#[test]
fn a_clean_run_says_no() {
    let event = exited_for("Done: 3 files changed");
    assert_eq!(
        event.fields.get("quota_exhausted"),
        Some(&serde_json::json!("no"))
    );
    assert_eq!(
        event.fields.get("quota_reset"),
        Some(&serde_json::json!(""))
    );
}

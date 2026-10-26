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
        pre_passed: None,
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

// ---- spec 281: text the agent READ is not the CLI's refusal ----

/// One JSON line the way the claude stream prints a file the agent read: a `tool_result` that quotes the phrase.
fn echoed_file_line() -> String {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": [ {
            "type": "tool_result",
            "content": "scripts/autonomy/selftest.sh:51: echo \"You have hit your session limit - resets 8:20pm (UTC)\" >&2"
        } ] }
    })
    .to_string()
}

/// The closing line of a real refusal: `result`, `is_error` true, the CLI's words in `result`.
fn refusal_result_line() -> String {
    serde_json::json!({
        "type": "result", "subtype": "success", "is_error": true, "api_error_status": 429,
        "result": "You've hit your session limit · resets 8:20pm (UTC)"
    })
    .to_string()
}

#[test]
fn a_file_the_agent_read_is_not_a_signal() {
    // THE BUG: the quoted phrase sits in a tool_result of a stream that ends cleanly.
    let stream = format!(
        "{}\n{}\n{}",
        r#"{"type":"system","subtype":"init"}"#,
        echoed_file_line(),
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done"}"#
    );
    assert!(quota_signal(&stream, "").is_none());
}

#[test]
fn a_real_refusal_at_the_end_of_a_stream_is_a_signal() {
    // CONTROL: the same stream with the CLI's own closing line still reports, and reads the reset time from it.
    let stream = format!("{}\n{}", echoed_file_line(), refusal_result_line());
    let signal = quota_signal(&stream, "").unwrap();
    assert_eq!(signal.reset_text, "resets 8:20pm (UTC)");
}

#[test]
fn a_refusal_is_found_whatever_lines_surround_it() {
    // CONTROL: the refusal line first, a clean line after it.
    let stream = format!("{}\n{}", refusal_result_line(), r#"{"type":"system"}"#);
    assert!(quota_signal(&stream, "").is_some());
}

#[test]
fn an_error_free_result_that_quotes_the_phrase_is_not_a_signal() {
    // NEAR MISS: a result line, but not an error: the agent's summary quoting the phrase.
    let line = serde_json::json!({
        "type": "result", "is_error": false,
        "result": "I read selftest.sh which says: You've hit your session limit · resets 8:20pm (UTC)"
    })
    .to_string();
    assert!(quota_signal(&line, "").is_none());
}

#[test]
fn a_nested_rate_limit_error_is_a_signal() {
    let line = r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#;
    assert!(quota_signal(line, "").is_some());
}

#[test]
fn plain_text_output_is_still_scanned_whole() {
    // CONTROL: not a JSON stream, so the whole-text rule of spec 237 stands.
    assert!(quota_signal("progress...\nYou've hit your weekly limit\n", "").is_some());
    assert!(quota_signal("progress...\nall fine\n", "").is_none());
}

#[test]
fn the_event_says_no_for_a_run_that_only_read_the_phrase() {
    let stream = format!(
        "{}\n{}",
        echoed_file_line(),
        r#"{"type":"result","is_error":false,"result":"ok"}"#
    );
    let event = exited_for(&stream);
    assert_eq!(
        event.fields.get("quota_exhausted"),
        Some(&serde_json::json!("no"))
    );
    assert_eq!(
        event.fields.get("quota_reset"),
        Some(&serde_json::json!(""))
    );
}

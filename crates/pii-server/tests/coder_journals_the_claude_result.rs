//! Spec 253: a failed claude run journals the text claude gave for its own failure, and the HTTP status it carried. The
//! `result` event is read for `result` and `api_error_status`; a non-claude run journals both as empty. Stand-in
//! scripts stand for `claude` and `pi`: no real call, no network, no spend.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::NodeInput;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_observe::{Event, Observer};
use pii_server::node_kinds::nodes::agent_claude::run_facts;
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

const SYSTEM_INIT: &str = r#"{"type":"system","subtype":"init","session_id":"9f8cea95-a50d-4cf3-bc54-12603578df48","model":"claude-haiku-5-5"}"#;

fn result_line(extra: &str) -> String {
    format!(
        r#"{{"type":"result","subtype":"success","is_error":true,"num_turns":0,"total_cost_usd":0{extra}}}"#
    )
}

#[test]
fn run_facts_reads_the_result_text() {
    let out = result_line(r#","result":"Failed to authenticate""#);
    let facts = run_facts(&out);
    assert_eq!(facts.result_text.as_deref(), Some("Failed to authenticate"));
}

#[test]
fn run_facts_reads_the_api_error_status() {
    let out = result_line(r#","api_error_status":401"#);
    let facts = run_facts(&out);
    assert_eq!(facts.api_error_status, Some(401));
}

#[test]
fn absent_fields_are_none() {
    let facts = run_facts(&result_line(""));
    assert_eq!(facts.result_text, None, "absent is None, not Some(\"\")");
    assert_eq!(facts.api_error_status, None, "absent is None, not Some(0)");
}

fn root_ctx(bin: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&BTreeMap::new()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// A stand-in agent named `name` in its own bin dir. The script body is a heredoc, so nothing in it expands.
fn stand_in(name: &str, body: &str) -> tempfile::TempDir {
    let bin =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "claude-result-bin-").unwrap();
    let agent = bin.path().join(name);
    std::fs::write(&agent, format!("#!/bin/sh\ncat <<'EOJ'\n{body}\nEOJ\n")).unwrap();
    std::fs::set_permissions(&agent, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    bin
}

/// Runs a `coder` node on the stand-in in `bin` with `extra` merged into its config; returns the journal.
fn run_agent(bin: &Path, extra: &serde_json::Value) -> Vec<Event> {
    let work =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "claude-result-work-").unwrap();
    let log = Arc::new(Rec::default());
    let env = BTreeMap::from([(
        "CLAUDE_CODE_OAUTH_TOKEN".to_owned(),
        "placeholder".to_owned(),
    )]);
    let factory = CoderFactory::with_env(bin.to_path_buf(), env, log.clone());
    let mut cfg = serde_json::json!({
        "agent": "claude",
        "model": "haiku",
        "brief": "do it",
        "spec": "specs/253-x.md",
        "workdir": work.path().display().to_string(),
        "env_pass": ["CLAUDE_CODE_OAUTH_TOKEN"],
    });
    for (k, v) in extra.as_object().unwrap() {
        cfg[k] = v.clone();
    }
    let node = pii_graph::node::NodeFactory::build(&factory, &cfg).unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    node.run(&root_ctx(bin), &input).unwrap();
    log.0.lock().clone()
}

fn exited(events: &[Event]) -> &Event {
    events
        .iter()
        .find(|e| e.kind == "coder.exited")
        .expect("coder.exited journalled")
}

fn text_field(e: &Event, key: &str) -> String {
    e.fields
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn failing_claude(result: &str) -> String {
    format!(
        "{SYSTEM_INIT}\n{}",
        result_line(&format!(r#","api_error_status":401,"result":"{result}""#))
    )
}

#[test]
fn the_journal_carries_the_result_and_status() {
    let bin = stand_in(
        "claude",
        &failing_claude("Failed to authenticate: OAuth token revoked"),
    );
    let events = run_agent(bin.path(), &serde_json::json!({}));
    let e = exited(&events);
    assert_eq!(
        text_field(e, "claude_result"),
        "Failed to authenticate: OAuth token revoked"
    );
    assert_eq!(text_field(e, "claude_api_error_status"), "401");
}

#[test]
fn a_long_result_is_cut_at_300_characters() {
    let long = "a".repeat(1000);
    let bin = stand_in("claude", &failing_claude(&long));
    let events = run_agent(bin.path(), &serde_json::json!({}));
    assert_eq!(
        text_field(exited(&events), "claude_result").chars().count(),
        300
    );
}

#[test]
fn a_secret_in_the_result_is_redacted() {
    let secret = "sk-ant-api03-AbCdEf0123456789XyZ";
    let bin = stand_in("claude", &failing_claude(&format!("bad key {secret}")));
    let events = run_agent(bin.path(), &serde_json::json!({}));
    let journalled = text_field(exited(&events), "claude_result");
    assert!(!journalled.contains(secret), "journalled: {journalled}");
    assert!(
        journalled.contains("[REDACTED]"),
        "journalled: {journalled}"
    );
}

#[test]
fn a_non_claude_run_leaves_both_fields_empty() {
    let bin = stand_in("pi", "pi says hello\nno json here");
    let events = run_agent(
        bin.path(),
        &serde_json::json!({ "agent": "pi", "model": "m-one" }),
    );
    let e = exited(&events);
    assert_eq!(text_field(e, "claude_result"), "");
    assert_eq!(text_field(e, "claude_api_error_status"), "");
}

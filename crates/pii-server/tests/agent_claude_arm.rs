//! The claude arm of the coder, before it ever runs through pii. Five gaps were read from the code and the working
//! launcher (`scripts/run-batchC-claude.sh`): no permission mode (a headless `-p` run cannot write), no stream-json
//! output (route, session and cost invisible), no session id kept, the env-only credential check (the login is a file)
//! and an unverified `haiku` alias. These tests pin the argv and the facts read back. The fixture lines are the real
//! ones from a haiku worker log (`_tmp/claude-batchC/234-a-skill-edit-goes-through-the-daemon.*.log`, trimmed to the
//! fields read), so the input is what claude printed and not a hand-written shape.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use pii_observe::NullObserver;
use pii_server::node_kinds::nodes::agent_claude::run_facts;
use pii_server::node_kinds::nodes::agent_worker::{WorkerSpec, plan, reported_routes};

const INIT: &str = r#"{"type":"system","subtype":"init","cwd":"/projects/pi-improved","session_id":"9f8cea95-a50d-4cf3-bc54-12603578df48","model":"claude-haiku-5-5","permissionMode":"bypassPermissions"}"#;
const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":40,"session_id":"9f8cea95-a50d-4cf3-bc54-12603578df48","total_cost_usd":0.16802829,"stop_reason":"end_turn","terminal_reason":"completed"}"#;

fn spec(sid: Option<&str>) -> WorkerSpec {
    let mut cfg = serde_json::json!({
        "agent": "claude", "model": "haiku", "spec": "specs/x.md", "workdir": "/projects/pi-improved"
    });
    if let Some(sid) = sid {
        cfg["sid"] = serde_json::json!(sid);
    }
    WorkerSpec::from_cfg(&cfg).unwrap()
}

fn argv(sid: Option<&str>) -> Vec<String> {
    let mut env = BTreeMap::new();
    env.insert(
        "CLAUDE_CODE_OAUTH_TOKEN".to_owned(),
        "placeholder".to_owned(),
    );
    plan(&spec(sid), "do the thing", &env, "direct", &NullObserver)
        .unwrap()
        .argv
}

fn has_pair(v: &[String], flag: &str, value: &str) -> bool {
    v.windows(2).any(|w| w[0] == flag && w[1] == value)
}

#[test]
fn a_headless_claude_may_write() {
    assert!(has_pair(
        &argv(None),
        "--permission-mode",
        "bypassPermissions"
    ));
}

#[test]
fn claude_output_is_stream_json_so_route_session_and_cost_are_readable() {
    let v = argv(None);
    assert!(has_pair(&v, "--output-format", "stream-json"));
    assert!(
        v.iter().any(|a| a == "--verbose"),
        "stream-json under -p needs --verbose"
    );
}

#[test]
fn partial_messages_are_not_requested() {
    // The measured log had 3,779 stream_event lines from --include-partial-messages; the coder keeps 256 KiB.
    assert!(!argv(None).iter().any(|a| a == "--include-partial-messages"));
}

#[test]
fn the_brief_is_the_value_of_dash_p_and_the_model_is_passed_through() {
    let v = argv(None);
    assert!(has_pair(&v, "-p", "do the thing"));
    assert!(has_pair(&v, "--model", "haiku"));
}

#[test]
fn a_resume_names_the_session() {
    let v = argv(Some("9f8cea95-a50d-4cf3-bc54-12603578df48"));
    assert!(has_pair(
        &v,
        "--resume",
        "9f8cea95-a50d-4cf3-bc54-12603578df48"
    ));
}

#[test]
fn a_fresh_run_has_no_resume() {
    assert!(!argv(None).iter().any(|a| a == "--resume"));
}

#[test]
fn claude_never_gains_a_provider_flag_or_a_key_on_argv() {
    let v = argv(None);
    assert!(!v.iter().any(|a| a == "--provider" || a == "--api-key"));
    assert!(!v.iter().any(|a| a == "placeholder"));
}

#[test]
fn the_session_id_cost_and_turns_are_read_from_the_result() {
    let out = format!("{INIT}\n{RESULT}\n");
    let f = run_facts(&out);
    assert_eq!(
        f.session_id.as_deref(),
        Some("9f8cea95-a50d-4cf3-bc54-12603578df48")
    );
    assert_eq!(f.turns, Some(40));
    assert!((f.cost_usd.unwrap() - 0.168_028_29).abs() < 1e-9);
    assert_eq!(f.is_error, Some(false));
}

#[test]
fn the_session_id_survives_a_run_that_never_reached_a_result() {
    // A killed or timed-out run has the init line but no result: the resume handle must still be there.
    let f = run_facts(&format!("{INIT}\n"));
    assert_eq!(
        f.session_id.as_deref(),
        Some("9f8cea95-a50d-4cf3-bc54-12603578df48")
    );
    assert_eq!(f.cost_usd, None);
}

#[test]
fn claude_reports_its_route_from_the_init_line() {
    let routes = reported_routes(&format!("{INIT}\n{RESULT}\n"));
    assert_eq!(
        routes,
        vec![(String::from("claude"), String::from("claude-haiku-5-5"))]
    );
}

#[test]
fn output_that_is_not_claude_json_yields_no_facts() {
    let f = run_facts("plain text, no json\n");
    assert_eq!(f.session_id, None);
    assert_eq!(f.cost_usd, None);
    assert_eq!(f.turns, None);
    assert!(reported_routes("plain text, no json\n").is_empty());
}

#[test]
fn a_pi_session_header_is_still_read_as_before() {
    // Near-miss for the new route rule: pi's header names provider + modelId and must keep working.
    let pi =
        r#"{"type":"session","provider":"ollama-cloud","modelId":"deepseek-v4.1-flash:cloud"}"#;
    assert_eq!(
        reported_routes(pi),
        vec![(
            String::from("ollama-cloud"),
            String::from("deepseek-v4.1-flash:cloud")
        )]
    );
}

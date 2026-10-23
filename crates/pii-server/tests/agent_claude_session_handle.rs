//! Spec 244: pii chooses a fresh claude run's session id and journals it in `coder.spawned` before the agent starts, so a
//! run that dies mid-agent still names the session it can be continued from. The handle is the one on the argv that was
//! spawned, and a malformed `sid` from the graph is refused before any spawn. Stand-in scripts stand for `claude`: no
//! real call, no network, no spend.

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
use pii_abi::{NodeInput, Outcome};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_observe::{Event, NullObserver, Observer};
use pii_server::node_kinds::nodes::agent_claude::{self, is_session_uuid};
use pii_server::node_kinds::nodes::agent_worker::{WorkerSpec, plan};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries, session_of};
use pii_server::node_kinds::nodes::criteria;

const SID: &str = "9f8cea95-a50d-4cf3-bc54-12603578df48";
const NEW: &str = "0b7c3e2a-6d41-4f8e-9a53-2c1d7e4b9f10";

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

fn has_pair(v: &[String], flag: &str, value: &str) -> bool {
    v.windows(2).any(|w| w[0] == flag && w[1] == value)
}

fn pair_list(sid: Option<&str>, new: Option<&str>) -> Vec<String> {
    agent_claude::argv(sid, new, "haiku", None, "the brief")
}

#[test]
fn a_first_claude_run_carries_a_session_id() {
    let v = pair_list(None, Some(NEW));
    assert!(has_pair(&v, "--session-id", NEW), "argv: {v:?}");
}

#[test]
fn a_resumed_claude_run_carries_resume_and_no_session_id() {
    let v = pair_list(Some(SID), None);
    assert!(has_pair(&v, "--resume", SID), "argv: {v:?}");
    assert!(!v.iter().any(|a| a == "--session-id"), "argv: {v:?}");
}

#[test]
fn the_two_flags_are_never_both_present() {
    for (sid, new) in [
        (None, None),
        (Some(SID), None),
        (None, Some(NEW)),
        (Some(SID), Some(NEW)),
    ] {
        let v = pair_list(sid, new);
        let both = v.iter().any(|a| a == "--resume") && v.iter().any(|a| a == "--session-id");
        assert!(!both, "both flags for {sid:?}/{new:?}: {v:?}");
    }
}

#[test]
fn the_brief_is_still_last() {
    for (sid, new) in [(None, None), (Some(SID), None), (None, Some(NEW))] {
        let v = pair_list(sid, new);
        assert_eq!(v.last().map(String::as_str), Some("the brief"), "{v:?}");
    }
}

fn pi_spec() -> WorkerSpec {
    WorkerSpec::from_cfg(&serde_json::json!({
        "agent": "pi", "model": "m-one", "spec": "specs/x.md", "workdir": "/projects/pi-improved"
    }))
    .unwrap()
}

#[test]
fn pi_never_gains_the_flag() {
    let d = plan(&pi_spec(), "x", &BTreeMap::new(), "direct", &NullObserver).unwrap();
    assert!(!d.argv.iter().any(|a| a == "--session-id"), "{:?}", d.argv);
}

#[test]
fn session_of_reads_claude_flags() {
    let fresh = vec![
        "claude".to_owned(),
        "--session-id".to_owned(),
        SID.to_owned(),
    ];
    assert_eq!(session_of(&fresh), SID);
    let resumed = vec!["claude".to_owned(), "--resume".to_owned(), SID.to_owned()];
    assert_eq!(session_of(&resumed), SID);
    let pi = vec![
        "pi".to_owned(),
        "--session".to_owned(),
        "/w/f.jsonl".to_owned(),
    ];
    assert_eq!(session_of(&pi), "/w/f.jsonl");
}

#[test]
fn session_of_an_argv_without_one_is_empty() {
    let v = vec![
        "claude".to_owned(),
        "--model".to_owned(),
        "haiku".to_owned(),
    ];
    assert_eq!(session_of(&v), "");
}

#[test]
fn a_session_id_is_a_lowercase_uuid_v4() {
    assert!(is_session_uuid(SID));
    assert!(is_session_uuid(NEW));
    assert!(!is_session_uuid("../etc"));
    assert!(!is_session_uuid("sid-1"));
    assert!(
        !is_session_uuid("9F8CEA95-A50D-4CF3-BC54-12603578DF48"),
        "uppercase is refused"
    );
    assert!(
        !is_session_uuid("9f8cea95-a50d-1cf3-bc54-12603578df48"),
        "a v1 uuid is refused"
    );
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

/// A stand-in `claude`: records the session id it received (from `--session-id` or `--resume`) in a file beside
/// itself, prints it on its `init` line, and exits 0.
const ECHO_CLAUDE: &str = r#"#!/bin/sh
sid=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--session-id" ] || [ "$prev" = "--resume" ]; then sid="$a"; fi
  prev="$a"
done
printf '%s' "$sid" > "$(dirname "$0")/received"
echo "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"$sid\",\"model\":\"claude-haiku-5-5\"}"
echo "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"num_turns\":1,\"session_id\":\"$sid\"}"
exit 0
"#;

/// A stand-in `claude` that never prints and never exits, so only the node's timeout ends it.
const HANG_CLAUDE: &str = "#!/bin/sh\nsleep 100000\n";

fn stand_in(script: &str) -> tempfile::TempDir {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "claude-bin-").unwrap();
    let claude = bin.path().join("claude");
    std::fs::write(&claude, script).unwrap();
    std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    bin
}

/// Runs a `coder` node on `claude` with `extra` merged into its config, and returns the outcome and the journal.
fn run_claude(bin: &Path, extra: &serde_json::Value) -> (Outcome, Vec<Event>) {
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "claude-work-").unwrap();
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
        "spec": "specs/244-x.md",
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
    let outcome = node.run(&root_ctx(bin), &input).unwrap();
    let events = log.0.lock().clone();
    (outcome, events)
}

fn event<'a>(events: &'a [Event], kind: &str) -> Option<&'a Event> {
    events.iter().find(|e| e.kind == kind)
}

fn text_field(e: &Event, key: &str) -> String {
    e.fields
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn the_spawned_event_names_the_session() {
    let bin = stand_in(ECHO_CLAUDE);
    let (_, events) = run_claude(bin.path(), &serde_json::json!({}));
    let spawned = event(&events, "coder.spawned").expect("coder.spawned journalled");
    let session = text_field(spawned, "session");
    assert!(is_session_uuid(&session), "spawned session: {session:?}");
    let received = std::fs::read_to_string(bin.path().join("received")).unwrap();
    assert_eq!(
        session, received,
        "the journal names the id the agent got on argv"
    );
}

#[test]
fn the_exited_event_agrees() {
    let bin = stand_in(ECHO_CLAUDE);
    let (_, events) = run_claude(bin.path(), &serde_json::json!({}));
    let spawned = event(&events, "coder.spawned").expect("coder.spawned journalled");
    let exited = event(&events, "coder.exited").expect("coder.exited journalled");
    assert_eq!(
        text_field(exited, "claude_session_id"),
        text_field(spawned, "session"),
        "claude printed the id pii chose"
    );
}

#[test]
fn a_run_killed_mid_agent_still_names_its_session() {
    let bin = stand_in(HANG_CLAUDE);
    let (_, events) = run_claude(bin.path(), &serde_json::json!({ "timeout_ms": 400 }));
    let spawned = event(&events, "coder.spawned").expect("coder.spawned journalled");
    let exited = event(&events, "coder.exited").expect("coder.exited journalled");
    assert!(
        is_session_uuid(&text_field(spawned, "session")),
        "the handle is journalled before the agent starts"
    );
    assert_eq!(
        text_field(exited, "claude_session_id"),
        "",
        "no init line, so claude printed no id"
    );
}

#[test]
fn a_resumed_run_journals_the_sid_it_resumes() {
    let bin = stand_in(ECHO_CLAUDE);
    let (_, events) = run_claude(bin.path(), &serde_json::json!({ "sid": SID }));
    let spawned = event(&events, "coder.spawned").expect("coder.spawned journalled");
    assert_eq!(text_field(spawned, "session"), SID);
}

#[test]
fn a_malformed_sid_is_refused_before_spawn() {
    let bin = stand_in(ECHO_CLAUDE);
    let (outcome, events) = run_claude(bin.path(), &serde_json::json!({ "sid": "../etc" }));
    match outcome {
        Outcome::Next { port, value } => {
            assert_eq!(port.as_str(), "err");
            let reason = value
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            assert!(
                reason.contains("sid"),
                "the reason names the field: {reason}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(event(&events, "coder.spawned").is_none(), "no spawn");
    assert!(!bin.path().join("received").exists(), "claude never ran");
}

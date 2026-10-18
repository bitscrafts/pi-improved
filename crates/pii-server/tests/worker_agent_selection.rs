//! Spec 185: a `worker` node picks its coding agent and model from graph data.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use parking_lot::Mutex;
use std::collections::BTreeMap;

use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::agent_worker::{
    AttemptEvidence, Ladder, WorkerSpec, escalate, plan,
};

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

fn cfg(agent: &str, model: &str) -> serde_json::Value {
    serde_json::json!({
        "agent": agent, "model": model,
        "spec": "specs/185-x.md", "workdir": "/tmp/w",
    })
}

fn spec(agent: &str, model: &str) -> WorkerSpec {
    WorkerSpec::from_cfg(&cfg(agent, model)).unwrap()
}

const LAUNCHER: &str = "scripts/agent-worker.sh";

#[test]
fn pi_selected_argv_begins_pi_session_and_names_no_claude() {
    let rec = Rec::default();
    let d = plan(
        &spec("pi", "deepseek-v4-flash"),
        "brief",
        &BTreeMap::new(),
        LAUNCHER,
        &rec,
    )
    .unwrap();
    assert_eq!(d.argv[0], "pi");
    assert_eq!(d.argv[1], "--session");
    assert!(d.argv.iter().any(|a| a == "-p"));
    assert!(!d.argv.iter().any(|a| a.contains("claude")));
    assert!(!d.argv.iter().any(|a| a == "--session-dir"));
    assert_eq!(d.launcher, LAUNCHER);
}

#[test]
fn claude_selected_argv_begins_claude_with_model() {
    let rec = Rec::default();
    let env = BTreeMap::from([("ANTHROPIC_API_KEY".to_string(), "k".to_string())]);
    let d = plan(&spec("claude", "haiku-5-5"), "brief", &env, LAUNCHER, &rec).unwrap();
    assert_eq!(d.argv[0], "claude");
    assert!(d.argv.iter().any(|a| a == "--resume") || d.argv.iter().any(|a| a == "-p"));
    let i = d.argv.iter().position(|a| a == "--model").expect("--model");
    assert_eq!(d.argv[i + 1], "haiku-5-5");

    let mut s = spec("claude", "haiku-5-5");
    s.sid = Some("sid-1".into());
    let d = plan(&s, "brief", &env, LAUNCHER, &rec).unwrap();
    assert_eq!(&d.argv[..3], ["claude", "--resume", "sid-1"]);
}

#[test]
fn model_is_data_two_runs_two_argvs_one_binary() {
    let rec = Rec::default();
    let a = plan(&spec("pi", "m-one"), "b", &BTreeMap::new(), LAUNCHER, &rec).unwrap();
    let b = plan(&spec("pi", "m-two"), "b", &BTreeMap::new(), LAUNCHER, &rec).unwrap();
    assert_ne!(a.argv, b.argv);
    assert_eq!(a.argv[0], b.argv[0]);
    assert!(a.argv.iter().any(|x| x == "m-one"));
    assert!(b.argv.iter().any(|x| x == "m-two"));
}

#[test]
fn missing_credentials_refuse_with_event_and_no_dispatch() {
    let rec = Rec::default();
    let r = plan(
        &spec("claude", "haiku-5-5"),
        "b",
        &BTreeMap::new(),
        LAUNCHER,
        &rec,
    );
    assert!(r.is_err());
    let evs = rec.0.lock().clone();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, names::WORKER_START_REFUSED);
    assert_eq!(
        evs[0].fields.get("agent").and_then(|v| v.as_str()),
        Some("claude")
    );
    assert_eq!(
        evs[0].fields.get("model").and_then(|v| v.as_str()),
        Some("haiku-5-5")
    );
}

#[test]
fn unknown_agent_is_invalid_not_defaulted() {
    assert!(WorkerSpec::from_cfg(&cfg("gemini", "m")).is_err());
    assert!(WorkerSpec::from_cfg(&serde_json::json!({"agent": "pi"})).is_err());
}

#[test]
fn escalation_waits_for_evidence_and_never_runs_both() {
    let ladder = Ladder::from_cfg(&serde_json::json!([
        {"agent": "pi", "model": "free"},
        {"agent": "claude", "model": "haiku-5-5"},
    ]))
    .unwrap();
    // First attempt: no evidence yet -> rung 0 only.
    assert_eq!(escalate(&ladder, &[]).unwrap().agent, "pi");
    // Pi failed with evidence -> claude.
    let ev = [AttemptEvidence {
        failing_criterion: Some("cargo test".into()),
    }];
    assert_eq!(escalate(&ladder, &ev).unwrap().agent, "claude");
    // An attempt that failed WITHOUT evidence does not buy an escalation.
    let none = [AttemptEvidence {
        failing_criterion: None,
    }];
    assert!(escalate(&ladder, &none).is_none());
    // A passing attempt ends the ladder.
    let ok = [
        AttemptEvidence {
            failing_criterion: Some("x".into()),
        },
        AttemptEvidence {
            failing_criterion: None,
        },
    ];
    assert!(escalate(&ladder, &ok).is_none());
}

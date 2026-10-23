//! A paid agent needs a spend cap that lives in the graph. `claude -p` has `--max-budget-usd`, and the coder had no
//! way to pass it, so a probe could only be bounded by a polite brief. The node key is `max_budget_usd`. Controls:
//! absent means no flag (the old argv), a non-positive or non-numeric value is refused and never becomes a flag, and
//! pi (which has no such flag) never gains one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use pii_observe::NullObserver;
use pii_server::node_kinds::nodes::agent_worker::{WorkerSpec, plan};

fn spec(agent: &str, extra: &serde_json::Value) -> Result<WorkerSpec, String> {
    let mut cfg = serde_json::json!({
        "agent": agent, "model": "haiku", "spec": "specs/x.md", "workdir": "/projects/pi-improved"
    });
    for (k, v) in extra.as_object().unwrap() {
        cfg[k] = v.clone();
    }
    WorkerSpec::from_cfg(&cfg).map_err(|e| e.to_string())
}

fn argv(agent: &str, extra: &serde_json::Value) -> Vec<String> {
    let env = BTreeMap::from([
        (
            "CLAUDE_CODE_OAUTH_TOKEN".to_owned(),
            "placeholder".to_owned(),
        ),
        ("HOME".to_owned(), "/nonexistent".to_owned()),
    ]);
    plan(
        &spec(agent, extra).unwrap(),
        "x",
        &env,
        "direct",
        &NullObserver,
    )
    .unwrap()
    .argv
}

fn value_of(v: &[String], flag: &str) -> Option<String> {
    v.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

#[test]
fn a_budget_becomes_the_claude_flag() {
    let v = argv("claude", &serde_json::json!({"max_budget_usd": 0.1}));
    assert_eq!(value_of(&v, "--max-budget-usd").as_deref(), Some("0.1"));
}

#[test]
fn the_flag_comes_before_the_brief_which_stays_last() {
    let v = argv("claude", &serde_json::json!({"max_budget_usd": 0.25}));
    assert_eq!(v[v.len() - 2], "-p");
    assert_eq!(v.last().map(String::as_str), Some("x"));
    assert!(v.iter().position(|a| a == "--max-budget-usd").unwrap() < v.len() - 2);
}

#[test]
fn no_budget_means_no_flag() {
    let v = argv("claude", &serde_json::json!({}));
    assert!(!v.iter().any(|a| a == "--max-budget-usd"));
}

#[test]
fn a_zero_budget_is_refused() {
    assert!(spec("claude", &serde_json::json!({"max_budget_usd": 0.0})).is_err());
}

#[test]
fn a_negative_budget_is_refused() {
    assert!(spec("claude", &serde_json::json!({"max_budget_usd": -1.0})).is_err());
}

#[test]
fn a_budget_that_is_not_a_number_is_refused() {
    assert!(spec("claude", &serde_json::json!({"max_budget_usd": "ten"})).is_err());
}

#[test]
fn pi_never_gains_the_flag() {
    let mut cfg = serde_json::json!({"max_budget_usd": 0.1});
    cfg["provider"] = serde_json::json!("ollama-cloud");
    let env = BTreeMap::from([("OLLAMA_API_KEY".to_owned(), "placeholder".to_owned())]);
    let mut c = serde_json::json!({
        "agent": "pi", "model": "m", "spec": "specs/x.md", "workdir": "/projects/pi-improved"
    });
    for (k, v) in cfg.as_object().unwrap() {
        c[k] = v.clone();
    }
    let s = WorkerSpec::from_cfg(&c).unwrap();
    let v = plan(&s, "x", &env, "direct", &NullObserver).unwrap().argv;
    assert!(!v.iter().any(|a| a == "--max-budget-usd"));
}

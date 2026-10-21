//! Spec 185 (provider): a `pi` dispatch names its provider on the argv, so `pi` cannot fall back to its
//! own default route. Without a provider the argv is unchanged. Controls: a `pi` node with no provider keeps
//! today's argv exactly (no `--provider`), and a `claude` node never gains one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::node_kinds::nodes::agent_worker::{Dispatch, WorkerSpec, plan};

fn cfg(agent: &str, provider: Option<&str>) -> serde_json::Value {
    let mut v = serde_json::json!({
        "agent": agent,
        "model": "deepseek-v4-flash",
        "spec": "specs/207-the-coder-node-spawns-an-agent.md",
        "workdir": "/projects/pi-improved",
    });
    if let Some(p) = provider {
        v["provider"] = serde_json::Value::String(p.to_owned());
    }
    v
}

fn argv_of(agent: &str, provider: Option<&str>) -> Vec<String> {
    let spec = WorkerSpec::from_cfg(&cfg(agent, provider)).unwrap();
    // The planner refuses a model with no credential in its env (the guard stays on). Supply the key NAME
    // the planner looks for, with a placeholder value that is never printed or sent anywhere.
    let mut env = std::collections::BTreeMap::new();
    env.insert(
        String::from("ANTHROPIC_API_KEY"),
        String::from("test-placeholder"),
    );
    env.insert(
        String::from("DEEPSEEK_API_KEY"),
        String::from("test-placeholder"),
    );
    let d: Dispatch = plan(&spec, "brief", &env, "direct", &NullObs).unwrap();
    d.argv
}

struct NullObs;
impl pii_observe::Observer for NullObs {}

#[test]
fn pi_with_a_provider_names_it_on_the_argv() {
    let argv = argv_of("pi", Some("deepseek"));
    let at = argv
        .iter()
        .position(|a| a == "--provider")
        .expect("--provider present");
    assert_eq!(argv[at + 1], "deepseek", "the provider follows the flag");
    assert_eq!(argv[0], "pi", "still the pi agent");
}

#[test]
fn pi_without_a_provider_keeps_the_argv_unchanged() {
    let argv = argv_of("pi", None);
    assert!(
        !argv.iter().any(|a| a == "--provider"),
        "no provider flag when none is set"
    );
}

#[test]
fn claude_never_gains_a_provider_flag() {
    let argv = argv_of("claude", Some("deepseek"));
    assert!(
        !argv.iter().any(|a| a == "--provider"),
        "claude takes no --provider"
    );
}

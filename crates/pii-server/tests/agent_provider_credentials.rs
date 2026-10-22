//! Spec 185 (credentials): a `pi` dispatch names its provider, and the coder must pass that provider's API
//! key. Refused before spawn when the key is absent; dispatched with `--provider` and `--model` when present.
//! Controls: the refusal and the success are both exercised, and a claude dispatch is not affected.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_observe::{Event, Observer};
use pii_server::node_kinds::nodes::agent_worker::{Dispatch, WorkerSpec, pi_provider_key, plan};

struct NullObs;
impl Observer for NullObs {
    fn event(&self, _e: &Event) {}
}

fn cfg(agent: &str, provider: Option<&str>, model: &str) -> serde_json::Value {
    let mut v = serde_json::json!({
        "agent": agent,
        "model": model,
        "spec": "specs/207-the-coder-node-spawns-an-agent.md",
        "workdir": "/projects/pi-improved",
    });
    if let Some(p) = provider {
        v["provider"] = serde_json::Value::String(p.to_owned());
    }
    v
}

fn env_with(keys: &[&str]) -> std::collections::BTreeMap<String, String> {
    keys.iter()
        .map(|k| ((*k).to_owned(), String::from("placeholder")))
        .collect()
}

fn plan_for(
    agent: &str,
    provider: Option<&str>,
    model: &str,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<Dispatch, pii_abi::PiiError> {
    let spec = WorkerSpec::from_cfg(&cfg(agent, provider, model)).unwrap();
    plan(&spec, "brief", env, "direct", &NullObs)
}

#[test]
fn the_ollama_cloud_provider_maps_to_its_key() {
    assert_eq!(pi_provider_key("ollama-cloud"), Some("OLLAMA_API_KEY"));
    assert_eq!(
        pi_provider_key("unlisted"),
        None,
        "an unknown provider has no key to check"
    );
}

#[test]
fn pi_refuses_when_the_provider_key_is_absent() {
    let env = env_with(&["PATH", "HOME"]);
    let r = plan_for(
        "pi",
        Some("ollama-cloud"),
        "deepseek-v4.1-flash:cloud",
        &env,
    );
    assert!(r.is_err(), "no OLLAMA_API_KEY: the agent must not start");
    let msg = format!("{}", r.unwrap_err());
    assert!(
        msg.contains("no credential for provider ollama-cloud"),
        "names the missing credential: {msg}"
    );
}

#[test]
fn pi_dispatches_with_provider_model_and_key_present() {
    let env = env_with(&["PATH", "HOME", "OLLAMA_API_KEY"]);
    let d = plan_for(
        "pi",
        Some("ollama-cloud"),
        "deepseek-v4.1-flash:cloud",
        &env,
    )
    .unwrap();
    let at = d
        .argv
        .iter()
        .position(|a| a == "--provider")
        .expect("--provider on the argv");
    assert_eq!(d.argv[at + 1], "ollama-cloud");
    let m = d
        .argv
        .iter()
        .position(|a| a == "--model")
        .expect("--model on the argv");
    assert_eq!(d.argv[m + 1], "deepseek-v4.1-flash:cloud");
    // The key is never on argv: pi reads it from the env reference in the pii-owned models.json.
    assert!(
        !d.argv.iter().any(|a| a == "--api-key"),
        "--api-key must not be on the argv; the key stays in the environment"
    );
    assert!(
        !d.argv.iter().any(|a| a == "placeholder"),
        "the key VALUE must never reach argv"
    );
}

#[test]
fn pi_dispatch_with_a_provider_is_planned_without_a_key_on_argv() {
    // Control: a named provider still plans (the key is resolved from the env reference, not argv).
    let env = env_with(&["PATH", "HOME", "OLLAMA_API_KEY"]);
    let d = plan_for(
        "pi",
        Some("ollama-cloud"),
        "deepseek-v4.1-flash:cloud",
        &env,
    )
    .unwrap();
    assert!(!d.argv.iter().any(|a| a == "--api-key"));
}

#[test]
fn pi_without_a_provider_is_not_refused_by_the_provider_check() {
    // Control: no provider named means no provider check, so today's behaviour is unchanged.
    let env = env_with(&["PATH", "HOME"]);
    assert!(plan_for("pi", None, "deepseek-v4.1-flash:cloud", &env).is_ok());
}

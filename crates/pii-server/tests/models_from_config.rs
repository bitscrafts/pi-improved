//! Spec 362 (one place names the models): a node names a role (`model = "@workhorse"`) and the daemon resolves it
//! to the `[models.<role>]` model and provider when the node is built. An explicit model id is untouched, and an
//! unknown role is refused by name before any agent is planned.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use pii_server::daemon::config::Config;
use pii_server::daemon::resolve_role;
use pii_server::node_kinds::nodes::agent_worker::{Dispatch, WorkerSpec, plan};

const ROLES: &str = r#"
[models.workhorse]
model = "deepseek-v4.1-flash"
provider = "ollama-cloud"

[models.planner]
model = "glm-5.3"
provider = "ollama-cloud"
"#;

fn node(model: &str) -> serde_json::Value {
    serde_json::json!({
        "agent": "pi",
        "model": model,
        "spec": "specs/362-one-place-names-the-models.md",
        "workdir": "/projects/pi-improved",
    })
}

fn argv_of(cfg: &serde_json::Value) -> Vec<String> {
    let spec = WorkerSpec::from_cfg(cfg).unwrap();
    // The planner refuses a model with no credential in its env; the placeholder is never printed or sent.
    let mut env = BTreeMap::new();
    env.insert(
        String::from("OLLAMA_API_KEY"),
        String::from("test-placeholder"),
    );
    let d: Dispatch = plan(&spec, "brief", &env, "direct", &NullObs).unwrap();
    d.argv
}

struct NullObs;
impl pii_observe::Observer for NullObs {}

fn value_after(argv: &[String], flag: &str) -> String {
    let at = argv
        .iter()
        .position(|a| a == flag)
        .unwrap_or_else(|| panic!("{flag} missing from {argv:?}"));
    argv[at + 1].clone()
}

#[test]
fn a_role_resolves_to_the_configured_model() {
    let config = Config::parse(ROLES).unwrap();
    let mut cfg = node("@workhorse");
    resolve_role(&mut cfg, &config.models).unwrap();
    let argv = argv_of(&cfg);
    assert_eq!(value_after(&argv, "--model"), "deepseek-v4.1-flash");
    assert_eq!(value_after(&argv, "--provider"), "ollama-cloud");
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for, as the spec names it
fn CONTROL_an_explicit_model_is_untouched() {
    let config = Config::parse(ROLES).unwrap();
    let mut cfg = node("some-model");
    resolve_role(&mut cfg, &config.models).unwrap();
    assert_eq!(cfg, node("some-model"), "an explicit id is not rewritten");
    let argv = argv_of(&cfg);
    assert_eq!(value_after(&argv, "--model"), "some-model");
}

#[test]
fn an_unknown_role_is_refused_by_name() {
    let config = Config::parse(ROLES).unwrap();
    let mut cfg = node("@nope");
    let err = resolve_role(&mut cfg, &config.models)
        .unwrap_err()
        .to_string();
    assert!(err.contains("nope"), "names the role: {err}");
    assert!(err.contains("workhorse"), "names a configured role: {err}");
    assert!(err.contains("planner"), "names a configured role: {err}");
    assert_eq!(cfg, node("@nope"), "nothing was resolved or planned");
}

// ---- the data path: a role that arrives through `$.input.model` is not present at load, so it resolves at spawn ----
// (measured 2026-11-04: `queue.toml` passes `@workhorse` into the spec graph, whose docs coder reads
// `model = "$.input.model"`; the literal string reached the agent and it never started.)

use std::sync::Arc;

use pii_abi::{NodeInput, Outcome};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_server::node_kinds::nodes::coder::CoderFactory;

/// A coder node whose model is read from the run input, a fake `pi` that records its argv, and the config's roles.
struct Spawned {
    outcome: Outcome,
    argv: Option<Vec<String>>,
}

fn spawn_with_input_model(model: &str) -> Spawned {
    spawn_with_input(model, None)
}

/// As above, with the run input also carrying `provider` (the queue passes `@workhorse` for both keys).
fn spawn_with_input(model: &str, provider: Option<&str>) -> Spawned {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "roles-bin-").unwrap();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "roles-work-").unwrap();
    let pi = bin.path().join("pi");
    std::fs::write(
        &pi,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$PWD/argv.out\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let env = BTreeMap::from([(
        String::from("OLLAMA_API_KEY"),
        String::from("test-placeholder"),
    )]);
    let roles = Config::parse(ROLES).unwrap().models;
    let factory = CoderFactory::with_env(
        bin.path().to_path_buf(),
        env,
        Arc::new(pii_observe::NullObserver),
    )
    .with_models(Arc::new(roles));
    let cfg = serde_json::json!({
        "agent": "pi", "model": "$.input.model", "provider": "$.input.provider", "brief": "a usable brief for the agent to act on",
        "spec": "specs/362-one-place-names-the-models.md", "workdir": work.path().display().to_string(),
        "env_pass": ["OLLAMA_API_KEY"],
    });
    let node = pii_graph::node::NodeFactory::build(&factory, &cfg).unwrap();
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(vec![String::from("/bin/sh"), pi.display().to_string()]);
    let input = NodeInput {
        input: provider.map_or_else(
            || serde_json::json!({ "model": model }),
            |p| serde_json::json!({ "model": model, "provider": p }),
        ),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    let outcome = node
        .run(&Ctx::root(caps, Budget::unlimited()), &input)
        .unwrap();
    let argv = std::fs::read_to_string(work.path().join("argv.out"))
        .ok()
        .map(|t| t.lines().map(str::to_owned).collect());
    Spawned { outcome, argv }
}

#[test]
fn a_role_arriving_through_the_run_input_is_resolved_at_spawn() {
    let r = spawn_with_input_model("@workhorse");
    let argv = r
        .argv
        .unwrap_or_else(|| panic!("the agent never started: {:?}", r.outcome));
    assert_eq!(
        value_after(&argv, "--model"),
        "deepseek-v4.1-flash",
        "{argv:?}"
    );
    assert_eq!(value_after(&argv, "--provider"), "ollama-cloud", "{argv:?}");
    assert!(
        !argv.iter().any(|a| a.contains('@')),
        "the literal role reached the agent: {argv:?}"
    );
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for, as the spec names it
fn CONTROL_an_explicit_model_through_the_run_input_is_untouched() {
    let r = spawn_with_input_model("some-model");
    let argv = r
        .argv
        .unwrap_or_else(|| panic!("the agent never started: {:?}", r.outcome));
    assert_eq!(value_after(&argv, "--model"), "some-model", "{argv:?}");
}

#[test]
fn an_unknown_role_through_the_run_input_is_refused_by_name_and_spawns_nothing() {
    let r = spawn_with_input_model("@nope");
    assert!(
        r.argv.is_none(),
        "an agent was spawned for an unknown role: {:?}",
        r.argv
    );
    let Outcome::Next { port, value } = &r.outcome else {
        panic!("expected a refusal on a port: {:?}", r.outcome)
    };
    assert_eq!(port.as_str(), "err", "{value}");
    let reason = value["reason"].as_str().unwrap_or_default();
    for want in ["nope", "workhorse", "planner"] {
        assert!(
            reason.contains(want),
            "the refusal names `{want}`: {reason}"
        );
    }
}

#[test]
fn a_role_passed_as_both_model_and_provider_resolves_both_from_the_table() {
    // `queue.toml` passes `@workhorse` for `model` AND `provider` (the callee's `needs` demands the second key).
    let r = spawn_with_input("@workhorse", Some("@workhorse"));
    let argv = r
        .argv
        .unwrap_or_else(|| panic!("the agent never started: {:?}", r.outcome));
    assert_eq!(
        value_after(&argv, "--model"),
        "deepseek-v4.1-flash",
        "{argv:?}"
    );
    assert_eq!(
        value_after(&argv, "--provider"),
        "ollama-cloud",
        "the table's provider, not the literal role: {argv:?}"
    );
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for, as the spec names it
fn CONTROL_an_explicit_model_keeps_the_provider_the_input_names() {
    let r = spawn_with_input("some-model", Some("ollama-cloud"));
    let argv = r
        .argv
        .unwrap_or_else(|| panic!("the agent never started: {:?}", r.outcome));
    assert_eq!(value_after(&argv, "--model"), "some-model", "{argv:?}");
    assert_eq!(value_after(&argv, "--provider"), "ollama-cloud", "{argv:?}");
}

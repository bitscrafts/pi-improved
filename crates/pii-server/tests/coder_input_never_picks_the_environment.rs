//! Spec 258: the run input never picks the agent's environment. `env_pass` is never resolved from the input, not even in its
//! STRING form (which would hand the node an array of env names); `criteria`, `container_image` and `container_runtime` stay
//! literal too. The literal `env_pass` array is the control: the fake agent sees a name exactly when the config allows it.
//! Stand-in `pi` scripts stand for the agent: no real call, no network, no spend.

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
use pii_observe::{Event, Observer};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// A scratch bin directory holding a fake `pi`: its body runs with `$0` beside it, so it can write what it saw there.
fn fake_pi(body: &str) -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-bin-").unwrap();
    let pi = dir.path().join("pi");
    std::fs::write(&pi, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    dir
}

/// The argv the fake `pi` recorded, one argument per line.
const RECORD_ARGV: &str = r#"printf '%s\n' "$@" > "${0%/*}/argv""#;

/// The fake `pi` writes the value of `OLLAMA_API_KEY` it sees, or `unset`.
const RECORD_ENV: &str = r#"printf '%s' "${OLLAMA_API_KEY-unset}" > "${0%/*}/seen""#;

fn root_ctx(bin: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&BTreeMap::new()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// Builds a `coder` node from `cfg` on the fake agent in `bin` and runs it on `input`.
fn run_node(
    bin: &Path,
    env: BTreeMap<String, String>,
    cfg: &serde_json::Value,
    input: &serde_json::Value,
) -> (Outcome, Vec<Event>) {
    let log = Arc::new(Rec::default());
    let factory = CoderFactory::with_env(bin.to_path_buf(), env, log.clone());
    let node = pii_graph::node::NodeFactory::build(&factory, cfg).unwrap();
    let input = NodeInput {
        input: input.clone(),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    let outcome = node.run(&root_ctx(bin), &input).unwrap();
    let events = log.0.lock().clone();
    (outcome, events)
}

/// The config a graph carries when it reads the run input: five required keys and the two optional ones.
fn input_cfg() -> serde_json::Value {
    serde_json::json!({
        "agent": "$.input.agent",
        "model": "$.input.model",
        "provider": "$.input.provider",
        "spec": "$.input.spec",
        "workdir": "$.input.workdir",
        "brief": "$.input.brief",
        "timeout_ms": 60000,
    })
}

fn full_input(workdir: &Path) -> serde_json::Value {
    serde_json::json!({
        "agent": "pi",
        "model": "m-one",
        "spec": "specs/x.md",
        "workdir": workdir.display().to_string(),
        "brief": "the brief text",
    })
}

#[test]
fn a_string_env_pass_is_not_resolved_from_the_input() {
    let bin = fake_pi(RECORD_ENV);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-work-").unwrap();
    let mut cfg = input_cfg();
    cfg["env_pass"] = serde_json::json!("$.input.names");
    let mut input = full_input(work.path());
    input["names"] = serde_json::json!(["OLLAMA_API_KEY"]);
    let env = BTreeMap::from([("OLLAMA_API_KEY".to_owned(), "placeholder".to_owned())]);
    let (_, _) = run_node(bin.path(), env, &cfg, &input);
    let seen = std::fs::read_to_string(bin.path().join("seen")).expect("the fake pi ran");
    assert_eq!(seen, "unset", "the input cannot choose the env names");
}

#[test]
fn a_literal_env_pass_still_passes_its_names() {
    let bin = fake_pi(RECORD_ENV);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-work-").unwrap();
    let mut cfg = input_cfg();
    cfg["env_pass"] = serde_json::json!(["OLLAMA_API_KEY"]);
    let input = full_input(work.path());
    let env = BTreeMap::from([("OLLAMA_API_KEY".to_owned(), "placeholder".to_owned())]);
    let (_, _) = run_node(bin.path(), env, &cfg, &input);
    let seen = std::fs::read_to_string(bin.path().join("seen")).expect("the fake pi ran");
    assert_eq!(seen, "placeholder", "a literal env_pass reaches the agent");
}

#[test]
fn criteria_and_image_and_runtime_strings_are_not_resolved() {
    let bin = fake_pi(RECORD_ARGV);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-work-").unwrap();
    let mut cfg = input_cfg();
    cfg["criteria"] = serde_json::json!("$.input.c");
    cfg["container_image"] = serde_json::json!("$.input.i");
    cfg["container_runtime"] = serde_json::json!("$.input.r");
    let mut input = full_input(work.path());
    input["c"] = serde_json::json!("workspace");
    input["i"] = serde_json::json!("input-image:9");
    input["r"] = serde_json::json!("socket");
    let (outcome, events) = run_node(bin.path(), BTreeMap::new(), &cfg, &input);
    let journal = format!("{events:?}");
    let shown = format!("{outcome:?}{journal}");
    // Each string stays literal: the runtime text is refused as named, so the input's `socket` is never chosen.
    let reason = match &outcome {
        Outcome::Next { value, .. } => value
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        other => panic!("expected a refusal on err, got {other:?}"),
    };
    assert!(
        reason.contains("'$.input.r'"),
        "the runtime stays the literal text: {reason}"
    );
    assert!(
        !shown.contains("input-image"),
        "the input's image never reached the run: {shown}"
    );
    assert!(
        !shown.contains("socket'"),
        "the runtime was not chosen from the input: {shown}"
    );
    assert!(
        !bin.path().join("argv").exists(),
        "no agent is launched: the literal runtime refuses before any spawn"
    );
    assert!(!journal.contains("criteria"), "no criterion ran: {journal}");
}

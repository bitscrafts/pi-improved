//! Spec 257: the coder node resolves `agent`, `model`, `provider`, `spec`, `workdir`, `brief` and the optional session keys
//! from the run input (`$.input.<path>`). Every other key is literal: the input never picks the env names, the image, or the
//! time bound. A missing or empty required key refuses before anything spawns. Stand-in `pi` scripts stand for the agent:
//! no real call, no network, no spend.

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
use pii_abi::{NodeInput, Outcome, PortName};
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

/// A scratch bin directory holding a fake `pi`: its body runs with `$0` beside it, so it can write its argv there.
fn fake_pi(body: &str) -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-bin-").unwrap();
    let pi = dir.path().join("pi");
    std::fs::write(&pi, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    dir
}

/// The argv the fake `pi` recorded, one argument per line.
const RECORD_ARGV: &str = r#"printf '%s\n' "$@" > "${0%/*}/argv""#;

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

fn reason_of(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Next { port, value } => {
            assert_eq!(port, &PortName::from("err"), "a refusal routes to err");
            value
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        }
        other => panic!("expected a refusal on err, got {other:?}"),
    }
}

#[test]
fn agent_model_spec_workdir_and_brief_come_from_the_input() {
    let bin = fake_pi(RECORD_ARGV);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-work-").unwrap();
    let input = full_input(work.path());
    let (_, _) = run_node(bin.path(), BTreeMap::new(), &input_cfg(), &input);
    let argv = std::fs::read_to_string(bin.path().join("argv")).expect("the fake pi ran");
    assert!(argv.contains("m-one"), "model from input: {argv}");
    assert!(argv.contains("the brief text"), "brief from input: {argv}");
    assert!(
        argv.contains(&work.path().display().to_string()),
        "workdir from input: {argv}"
    );
}

#[test]
fn a_missing_required_key_refuses_and_names_it() {
    let bin = fake_pi(&format!("touch \"${{0%/*}}/spawned\"\n{RECORD_ARGV}"));
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-work-").unwrap();
    let mut input = full_input(work.path());
    input.as_object_mut().unwrap().remove("agent");
    let (outcome, _) = run_node(bin.path(), BTreeMap::new(), &input_cfg(), &input);
    let reason = reason_of(&outcome);
    assert!(reason.contains("agent"), "reason: {reason}");
    assert!(reason.contains("$.input.agent"), "reason: {reason}");
    assert!(reason.contains("resolved to nothing"), "reason: {reason}");
    assert!(!bin.path().join("spawned").exists(), "no agent is launched");
}

#[test]
fn an_empty_string_required_key_refuses() {
    let bin = fake_pi(&format!("touch \"${{0%/*}}/spawned\"\n{RECORD_ARGV}"));
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-work-").unwrap();
    let mut input = full_input(work.path());
    input["model"] = serde_json::json!("");
    let (outcome, _) = run_node(bin.path(), BTreeMap::new(), &input_cfg(), &input);
    let reason = reason_of(&outcome);
    assert!(reason.contains("model"), "reason: {reason}");
    assert!(reason.contains("resolved to nothing"), "reason: {reason}");
    assert!(!bin.path().join("spawned").exists(), "no agent is launched");
}

#[test]
fn a_null_optional_key_is_absent() {
    let bin = fake_pi(RECORD_ARGV);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-work-").unwrap();
    let mut input = full_input(work.path());
    input["provider"] = serde_json::Value::Null;
    let (_, _) = run_node(bin.path(), BTreeMap::new(), &input_cfg(), &input);
    let argv = std::fs::read_to_string(bin.path().join("argv")).expect("the fake pi ran");
    assert!(!argv.lines().any(|l| l == "--provider"), "argv: {argv}");
}

#[test]
fn a_literal_config_is_untouched() {
    let bin = fake_pi(RECORD_ARGV);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-work-").unwrap();
    let cfg = serde_json::json!({
        "agent": "pi",
        "model": "literal-model",
        "spec": "specs/x.md",
        "workdir": work.path().display().to_string(),
        "brief": "literal brief",
    });
    let (outcome, _) = run_node(bin.path(), BTreeMap::new(), &cfg, &serde_json::json!({}));
    assert!(
        matches!(&outcome, Outcome::Next { port, .. } if port == &PortName::from("ok")),
        "a literal run still succeeds: {outcome:?}"
    );
    let argv = std::fs::read_to_string(bin.path().join("argv")).expect("the fake pi ran");
    assert!(argv.contains("literal-model"), "argv: {argv}");
}

#[test]
fn env_pass_is_never_resolved() {
    let bin = fake_pi(r#"printf '%s' "${OLLAMA_API_KEY-unset}" > "${0%/*}/seen""#);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-work-").unwrap();
    let mut cfg = input_cfg();
    cfg["env_pass"] = serde_json::json!(["$.input.names"]);
    let input = serde_json::json!({
        "agent": "pi",
        "model": "m-one",
        "spec": "specs/x.md",
        "workdir": work.path().display().to_string(),
        "brief": "b",
        "names": ["OLLAMA_API_KEY"],
    });
    let env = BTreeMap::from([("OLLAMA_API_KEY".to_owned(), "placeholder".to_owned())]);
    let (_, _) = run_node(bin.path(), env, &cfg, &input);
    let seen = std::fs::read_to_string(bin.path().join("seen")).expect("the fake pi ran");
    assert_eq!(seen, "unset", "the input cannot widen env_pass");
}

#[test]
fn the_input_cannot_pick_the_image() {
    let bin = fake_pi(RECORD_ARGV);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "input-work-").unwrap();
    let mut cfg = input_cfg();
    cfg["container_image"] = serde_json::json!("literal-image:1");
    let mut input = full_input(work.path());
    input["container_image"] = serde_json::json!("input-image:9");
    let (outcome, events) = run_node(bin.path(), BTreeMap::new(), &cfg, &input);
    // The container path ran (its image probe journals a digest slot); the image name itself is not in the refusal,
    // so the check is that the input's image appears nowhere the run wrote.
    let journal = format!("{events:?}");
    assert!(
        journal.contains("image_digest"),
        "the container path ran: {journal}"
    );
    assert!(
        !format!("{outcome:?}{journal}").contains("input-image"),
        "the input's image never reached the run: {outcome:?} {journal}"
    );
}

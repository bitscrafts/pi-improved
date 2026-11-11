//! Spec 410: a `refuse` node's `reason` may hold `{{path}}` placeholders, rendered at run time from the node's own eval context
//! with the same renderer a prompt uses. A reason that cannot be rendered is used as written (the refusal still refuses), a
//! reason without braces is unchanged, the arriving verdict's reason is still appended, and a called graph's refuse reason
//! follows the inliner's renaming. `system` stays data.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_graph::fragment::{self, Woven};
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::validate;

/// Stand-in for the agent kind: the real one lives in `pii-server`, and load checks only its ports.
struct Agent;

impl NodeFactory for Agent {
    fn kind(&self) -> &'static str {
        "agent"
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Err(PiiError::Internal("not built in this test".into()))
    }
}

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(Agent));
    reg
}

/// Builds a `refuse` node from `cfg` and runs it on `input`.
fn run_node(
    cfg: &serde_json::Value,
    input: serde_json::Value,
    payload: serde_json::Value,
) -> Outcome {
    let node = NodeRegistry::builtin()
        .factory("refuse")
        .unwrap()
        .build(cfg)
        .unwrap();
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    let input = NodeInput {
        input,
        payload,
        nodes: serde_json::json!({}),
    };
    node.run(&ctx, &input).unwrap()
}

/// The refusal text of a refuse node with no `returns`; any other outcome fails the test.
fn refusal(
    cfg: &serde_json::Value,
    input: serde_json::Value,
    payload: serde_json::Value,
) -> String {
    match run_node(cfg, input, payload) {
        Outcome::Fail { error } => error,
        other => panic!("a refusal with no `returns` must fail the run, got {other:?}"),
    }
}

fn cfg(reason: &str) -> serde_json::Value {
    serde_json::json!({ "hook": "review", "reason": reason })
}

#[test]
fn a_reason_placeholder_is_rendered_from_the_run_input() {
    let error = refusal(
        &cfg("rubric {{input.rubric}} did not pass"),
        serde_json::json!({ "rubric": "omega" }),
        serde_json::Value::Null,
    );
    assert!(
        error.contains("rubric omega did not pass"),
        "the placeholder must be rendered from the run input: {error}"
    );
}

#[test]
fn a_reason_placeholder_reads_the_arriving_payload() {
    let error = refusal(
        &cfg("bad: {{payload.name}}"),
        serde_json::json!({}),
        serde_json::json!({ "name": "ada" }),
    );
    assert!(
        error.contains("ada"),
        "the payload's name must appear in the refusal: {error}"
    );
}

// CONTROL: a placeholder that resolves to nothing does not turn the refusal into another error.
#[test]
fn control_an_unresolvable_placeholder_keeps_the_reason_as_written() {
    let error = refusal(
        &cfg("x {{input.nope}} y"),
        serde_json::json!({}),
        serde_json::Value::Null,
    );
    assert!(
        error.contains("x {{input.nope}} y"),
        "the reason is used as written: {error}"
    );
}

// CONTROL: an unclosed `{{` does not turn the refusal into another error.
#[test]
fn control_an_unclosed_placeholder_keeps_the_reason_as_written() {
    let error = refusal(
        &cfg("x {{input.rubric"),
        serde_json::json!({ "rubric": "omega" }),
        serde_json::Value::Null,
    );
    assert!(
        error.contains("x {{input.rubric"),
        "the reason is used as written: {error}"
    );
}

// CONTROL: a reason without braces is byte-identical to the old output, with and without `returns`.
#[test]
fn control_a_reason_without_braces_is_unchanged() {
    let error = refusal(
        &cfg("the rubric did not pass"),
        serde_json::json!({ "rubric": "omega" }),
        serde_json::Value::Null,
    );
    assert_eq!(
        error,
        "run.refused{hook=review, reason=the rubric did not pass}"
    );

    let with_returns = serde_json::json!({
        "hook": "review",
        "reason": "the rubric did not pass",
        "returns": "refused_port",
    });
    match run_node(
        &with_returns,
        serde_json::json!({ "rubric": "omega" }),
        serde_json::Value::Null,
    ) {
        Outcome::Next { port, value } => {
            assert_eq!(port.as_str(), "ok");
            assert_eq!(value["verdict"], "refused");
            assert_eq!(value["port"], "refused_port");
            assert_eq!(value["reason"], "refused_port: the rubric did not pass");
            assert!(value["criterion"].is_null(), "no criterion: {value}");
        }
        other => panic!("a refusal with `returns` must answer on ok, got {other:?}"),
    }
}

#[test]
fn the_arriving_reason_is_still_appended() {
    let error = refusal(
        &cfg("rubric {{input.rubric}} did not pass"),
        serde_json::json!({ "rubric": "omega" }),
        serde_json::json!({ "reason": "boom" }),
    );
    assert_eq!(
        error,
        "run.refused{hook=review, reason=rubric omega did not pass: boom}"
    );
}

fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

/// The host: `start -> docs(call) -> done`. `PIN` is the sub-graph's real digest.
const HOST: &str = r#"
name = "host"
entry = "start"

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "docs"
kind = "call"
graph = "docs"
sha256 = "PIN"
caps = []

[node.in]
title = "$.input.x"

[[node]]
id = "done"
kind = "transform"
expr = "$.payload"

[[edge]]
from = "start"
port = "ok"
to = "docs"
kind = "data"

[[edge]]
from = "docs"
port = "ok"
to = "done"
kind = "data"
"#;

fn host_for(sub: &str) -> String {
    HOST.replace("PIN", &fragment::sha256_hex(sub.as_bytes()))
}

/// Writes the named graph files under a fresh temp dir, weaves `host` against them, and validates the woven graph
/// the way `pii graph lint` does. A refusal from either step comes back as its text.
fn load(host: &str, graphs: &[(&str, &str)]) -> Result<Woven, String> {
    let root = tempfile::tempdir_in(tmp_root()).expect("tempdir under _tmp");
    let graph_dir = root.path().join("graphs");
    std::fs::create_dir_all(&graph_dir).expect("mkdir graphs");
    for (name, src) in graphs {
        std::fs::write(graph_dir.join(format!("{name}.toml")), src).expect("write graph");
    }
    let fragments: PathBuf = root.path().join("fragments");
    let reg = registry();
    let woven =
        fragment::load_host_with_graphs(host, &fragments, std::slice::from_ref(&graph_dir), &reg)
            .map_err(|e| e.to_string())?;
    match validate(&woven.spec, &reg) {
        Ok(_) => Ok(woven),
        Err(errors) => Err(errors.join("\n")),
    }
}

/// The woven node `id`'s config.
fn cfg_of<'a>(woven: &'a Woven, id: &str) -> &'a serde_json::Value {
    &woven
        .spec
        .nodes
        .iter()
        .find(|n| n.id.as_str() == id)
        .unwrap_or_else(|| panic!("no woven node {id}"))
        .cfg
}

#[test]
fn a_called_graphs_reason_placeholder_follows_the_renaming() {
    let sub = "name = \"docs\"\nexit = \"b\"\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"alpha\"\n\n\
               [[node]]\nid = \"b\"\nkind = \"refuse\"\nhook = \"review\"\nreturns = \"refused\"\nreason = \"bad: {{nodes.a.x}}\"\n\n\
               [[edge]]\nfrom = \"a\"\nport = \"ok\"\nto = \"b\"\nkind = \"data\"\n";
    let woven = load(&host_for(sub), &[("docs", sub)]).expect("load");
    assert_eq!(
        cfg_of(&woven, "docs__b")
            .get("reason")
            .and_then(serde_json::Value::as_str),
        Some("bad: {{nodes.docs__a.x}}")
    );
}

// CONTROL: a `system` field holding braces is data; the reason rule does not touch it.
#[test]
fn control_a_system_field_holding_braces_is_not_rewritten_by_the_reason_rule() {
    let sub = "name = \"docs\"\nexit = \"r\"\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"alpha\"\n\n\
               [[node]]\nid = \"b\"\nkind = \"agent\"\nprompt = \"plain words\"\nsystem = \"see {{nodes.a.x}}\"\n\n\
               [[node]]\nid = \"r\"\nkind = \"refuse\"\nhook = \"review\"\nreturns = \"refused\"\nreason = \"no {{nodes.a.x}}\"\n\n\
               [[edge]]\nfrom = \"a\"\nport = \"ok\"\nto = \"b\"\nkind = \"data\"\n\n\
               [[edge]]\nfrom = \"b\"\nport = \"ok\"\nto = \"r\"\nkind = \"data\"\n";
    let woven = load(&host_for(sub), &[("docs", sub)]).expect("load");
    assert_eq!(
        cfg_of(&woven, "docs__b")
            .get("system")
            .and_then(serde_json::Value::as_str),
        Some("see {{nodes.a.x}}")
    );
}

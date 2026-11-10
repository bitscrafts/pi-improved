//! Spec 404: a `{{path}}` placeholder in an inlined node's `prompt` is renamed by the same rule as the `$.path` expression
//! it stands for. `{{nodes.m}}` follows the namespaced copy, `{{input.key}}` follows the call's input mapping, and only
//! `prompt` is rewritten: a `system` or a `value` that happens to contain braces is data and stays as written.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::{Effects, PiiError, PortName};
use pii_core::CapKind;
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

/// A sub-graph `a` (transform) then `b` (agent). `b`'s prompt is the body each test sets.
fn sub_with_prompt(prompt: &str) -> String {
    format!(
        "name = \"docs\"\nexit = \"b\"\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"alpha\"\n\n\
         [[node]]\nid = \"b\"\nkind = \"agent\"\nprompt = \"{prompt}\"\n\n\
         [[edge]]\nfrom = \"a\"\nport = \"ok\"\nto = \"b\"\nkind = \"data\"\n"
    )
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

fn pin(src: &str) -> String {
    fragment::sha256_hex(src.as_bytes())
}

/// The host with its `PIN` replaced by the real digest of `sub`.
fn host_for(sub: &str) -> String {
    HOST.replace("PIN", &pin(sub))
}

/// Writes the named graph files under a fresh temp dir, weaves `host` against them, and validates the woven graph
/// the way `pii graph lint` does. A refusal from either step comes back as its text.
fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

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

fn load_err(host: &str, graphs: &[(&str, &str)]) -> String {
    load(host, graphs).expect_err("load must refuse")
}

fn ids(woven: &Woven) -> Vec<&str> {
    woven.spec.nodes.iter().map(|n| n.id.as_str()).collect()
}

/// The `prompt` of the woven node `id`.
fn prompt_of<'a>(woven: &'a Woven, id: &str) -> &'a str {
    let node = woven
        .spec
        .nodes
        .iter()
        .find(|n| n.id.as_str() == id)
        .unwrap_or_else(|| panic!("no woven node {id}: {:?}", ids(woven)));
    node.cfg
        .get("prompt")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("node {id} has no prompt: {}", node.cfg))
}

#[test]
fn a_prompt_placeholder_naming_a_subgraph_node_is_renamed_with_it() {
    let sub = sub_with_prompt("Rubric: {{nodes.a}}");
    let woven = load(&host_for(&sub), &[("docs", &sub)]).expect("load");
    assert_eq!(prompt_of(&woven, "docs__b"), "Rubric: {{nodes.docs__a}}");
}

#[test]
fn control_a_placeholder_naming_no_subgraph_node_is_still_refused() {
    let sub = sub_with_prompt("Rubric: {{nodes.zzz}}");
    let msg = load_err(&host_for(&sub), &[("docs", &sub)]);
    assert!(msg.contains("V12"), "names the rule: {msg}");
    assert!(
        msg.contains("{{nodes.zzz}}"),
        "names the placeholder: {msg}"
    );
}

#[test]
fn a_prompt_input_placeholder_follows_the_call_mapping() {
    let sub = sub_with_prompt("T: {{input.title}}");
    let woven = load(&host_for(&sub), &[("docs", &sub)]).expect("load");
    // `$.input.title` reaches the call's mapped input node `docs__in__title`; the placeholder must name the same one.
    assert!(
        ids(&woven).contains(&"docs__in__title"),
        "the mapped input node exists: {:?}",
        ids(&woven)
    );
    assert_eq!(prompt_of(&woven, "docs__b"), "T: {{nodes.docs__in__title}}");
}

#[test]
fn control_a_system_and_a_value_containing_braces_are_not_rewritten() {
    let sub = "name = \"docs\"\nexit = \"b\"\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"{{nodes.a}}\"\n\n\
               [[node]]\nid = \"b\"\nkind = \"agent\"\nprompt = \"plain words\"\nsystem = \"see {{nodes.a}}\"\n\n\
               [[edge]]\nfrom = \"a\"\nport = \"ok\"\nto = \"b\"\nkind = \"data\"\n";
    let woven = load(&host_for(sub), &[("docs", sub)]).expect("load");
    let a = woven
        .spec
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "docs__a")
        .expect("docs__a");
    let b = woven
        .spec
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "docs__b")
        .expect("docs__b");
    assert_eq!(
        a.cfg.get("value").and_then(serde_json::Value::as_str),
        Some("{{nodes.a}}")
    );
    assert_eq!(
        b.cfg.get("system").and_then(serde_json::Value::as_str),
        Some("see {{nodes.a}}")
    );
}

#[test]
fn control_an_unrelated_placeholder_keeps_its_spacing() {
    let sub = sub_with_prompt("{{ payload.text }} and {{nodes.a}}");
    let woven = load(&host_for(&sub), &[("docs", &sub)]).expect("load");
    assert_eq!(
        prompt_of(&woven, "docs__b"),
        "{{ payload.text }} and {{nodes.docs__a}}"
    );
}

#[test]
fn a_graph_without_a_prompt_placeholder_weaves_as_before() {
    // The `SUB` and `HOST` of spec 255's `call_inlines_a_graph.rs`: no prompt, so nothing here may change.
    let sub = "name = \"docs\"\nexit = \"b\"\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\nexpr = \"$.input.title\"\n\n\
               [[node]]\nid = \"b\"\nkind = \"transform\"\nexpr = \"$.nodes.a\"\n\n\
               [[edge]]\nfrom = \"a\"\nport = \"ok\"\nto = \"b\"\nkind = \"data\"\n";
    let woven = load(&host_for(sub), &[("docs", sub)]).expect("load");
    let ids = ids(&woven);
    for want in ["docs__in", "docs__a", "docs__b"] {
        assert!(ids.contains(&want), "missing {want}: {ids:?}");
    }
    let b = woven
        .spec
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "docs__b")
        .expect("docs__b");
    assert_eq!(
        b.cfg.get("expr").and_then(serde_json::Value::as_str),
        Some("$.nodes.docs__a")
    );
}

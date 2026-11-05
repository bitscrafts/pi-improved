//! Spec 386: a node's `prompt` is checked at load. Its `{{path}}` placeholders must name a path the graph can supply, it may not
//! share a node with `brief`, it is bounded in size, and only an agent node may carry one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pii_abi::{Effects, PiiError, PortName};
use pii_core::CapKind;
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::{load_toml_with_registry, validate};

/// Stand-in for the coder kind: the real one lives in `pii-server`, and load checks only its ports.
struct Coder;

impl NodeFactory for Coder {
    fn kind(&self) -> &'static str {
        "coder"
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
    reg.register(Arc::new(Coder));
    reg
}

/// A graph with a `transform` node `inventory` and a `coder` node `write`; `write_body` holds the coder's own keys.
fn graph(write_body: &str) -> String {
    format!(
        "name = \"t\"\nentry = \"write\"\ncaps = [\"spawn\", \"proc\"]\n\n\
         [[node]]\nid = \"inventory\"\nkind = \"transform\"\nvalue = \"x\"\n\n\
         [[node]]\nid = \"write\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\ntimeout_ms = 60000\n{write_body}\n"
    )
}

/// Loads and validates `src` the way `pii graph lint` does, and returns the refusals joined.
fn load_error(src: &str) -> String {
    let reg = registry();
    let spec = load_toml_with_registry(src, &reg).unwrap();
    match validate(&spec, &reg) {
        Ok(_) => panic!("the graph should be refused"),
        Err(errors) => errors.join("\n"),
    }
}

#[test]
fn a_placeholder_naming_a_declared_node_loads() {
    let src = graph("prompt = \"Chapters: {{nodes.inventory.value}} {{input.chapter}}\"");
    let reg = registry();
    let spec = load_toml_with_registry(&src, &reg).unwrap();
    assert!(validate(&spec, &reg).is_ok());
}

#[test]
fn a_placeholder_naming_an_undeclared_node_is_refused_at_load() {
    let err = load_error(&graph("prompt = \"{{nodes.nope.stdout}}\""));
    assert!(err.contains("nodes.nope.stdout"), "{err}");
    assert!(err.contains("write"), "names the node: {err}");
}

#[test]
fn an_unclosed_placeholder_is_refused_at_load() {
    let err = load_error(&graph("prompt = \"x {{nodes.inventory.value\""));
    assert!(err.contains("write"), "names the node: {err}");
}

#[test]
fn prompt_and_brief_together_are_refused_naming_the_node() {
    let err = load_error(&graph("prompt = \"x\"\nbrief = \"y\""));
    assert!(err.contains("write"), "{err}");
    assert!(err.contains("brief"), "{err}");
}

#[test]
fn an_oversized_prompt_is_refused_naming_the_cap() {
    let big = "a".repeat(65 * 1024);
    let err = load_error(&graph(&format!("prompt = \"{big}\"")));
    assert!(err.contains("65536"), "names the cap: {err}");
}

#[test]
fn a_non_agent_node_with_a_prompt_is_refused() {
    let src = "name = \"t\"\nentry = \"inventory\"\n\n\
               [[node]]\nid = \"inventory\"\nkind = \"transform\"\nvalue = \"x\"\nprompt = \"hello\"\n";
    let err = load_error(src);
    assert!(err.contains("inventory"), "{err}");
    assert!(err.contains("prompt"), "{err}");
}

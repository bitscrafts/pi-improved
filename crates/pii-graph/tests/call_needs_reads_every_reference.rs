//! Spec 409 R7: the load-time checks on a called graph's expressions (`graph.call.leak`, `graph.call.undeclared`) read every
//! `$.` reference of an expression, not only the first. A `$.` inside a string literal is data and is not checked. A
//! single-reference expression is checked exactly as before. Weaves the host against the sub-graph, as `call_needs` does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_graph::fragment::{self, Woven};
use pii_graph::node::NodeRegistry;

/// Escapes `s` as the body of a TOML basic string.
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A sub-graph `docs` with the given header line (`needs = [...]` or empty), a transform `a` holding `alpha`, and a
/// transform `b` computing `expr`. The edge a -> b carries the value, and `b` is the exit.
fn sub(header: &str, expr: &str) -> String {
    format!(
        "name = \"docs\"\nexit = \"b\"\n{header}\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"alpha\"\n\n\
         [[node]]\nid = \"b\"\nkind = \"transform\"\nexpr = \"{}\"\n\n\
         [[edge]]\nfrom = \"a\"\nport = \"ok\"\nto = \"b\"\nkind = \"data\"\n",
        toml_escape(expr),
    )
}

/// The host: `start -> docs(call) -> done`, with a host node `hostnode` the call does not read through `in`. `PIN` is
/// the sub-graph's real digest and `TAIL` holds extra call-table lines (`[node.in]` and its keys).
const HOST: &str = r#"
name = "host"
entry = "start"

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "hostnode"
kind = "transform"
value = "host-value"

[[node]]
id = "docs"
kind = "call"
graph = "docs"
sha256 = "PIN"
caps = []
TAIL

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

/// The host calling `sub`, with `tail` as the call's extra table lines.
fn host_for(sub: &str, tail: &str) -> String {
    HOST.replace("PIN", &pin(sub)).replace("TAIL", tail)
}

/// Writes `sub` as graph `docs` under a fresh temp dir and weaves `host` against it. Validation is not run: these tests
/// read the load-time refusals of the call checks only.
fn load(host: &str, sub: &str) -> Result<Woven, String> {
    let root = tempfile::tempdir().expect("tempdir");
    let graph_dir = root.path().join("graphs");
    std::fs::create_dir_all(&graph_dir).expect("mkdir graphs");
    std::fs::write(graph_dir.join("docs.toml"), sub).expect("write graph");
    let fragments = root.path().join("fragments");
    fragment::load_host_with_graphs(
        host,
        &fragments,
        std::slice::from_ref(&graph_dir),
        &NodeRegistry::builtin(),
    )
    .map_err(|e| e.to_string())
}

#[test]
fn a_host_node_read_by_a_later_reference_is_a_leak() {
    let sub = sub("", "$.nodes.a + $.nodes.hostnode");
    let Err(err) = load(&host_for(&sub, ""), &sub) else {
        panic!("load must refuse");
    };
    assert!(err.contains("graph.call.leak"), "{err}");
    assert!(err.contains("hostnode"), "{err}");
}

#[test]
fn an_undeclared_input_in_a_later_reference_is_refused() {
    // `k` is declared, `secret` is not: the second reference is the one refused.
    let sub = sub("needs = [\"k\"]", "$.input.k + $.input.secret");
    let Err(err) = load(&host_for(&sub, ""), &sub) else {
        panic!("load must refuse");
    };
    assert!(err.contains("graph.call.undeclared"), "{err}");
    assert!(err.contains("input 'secret'"), "{err}");
}

#[test]
fn a_declared_input_in_a_later_reference_is_accepted() {
    let sub = sub("needs = [\"k\", \"secret\"]", "$.input.k + $.input.secret");
    load(&host_for(&sub, ""), &sub).expect("load");
}

#[test]
fn a_mapped_input_in_a_later_reference_is_accepted() {
    let sub = sub("needs = [\"k\"]", "$.nodes.a + $.input.k");
    let tail = "\n[node.in]\nk = \"$.input.x\"\n";
    load(&host_for(&sub, tail), &sub).expect("load");
}

#[test]
fn control_a_host_node_named_inside_a_string_literal_is_not_a_leak() {
    let sub = sub("", "\"$.nodes.hostnode\" + $.nodes.a");
    load(&host_for(&sub, ""), &sub).expect("load");
}

#[test]
fn control_a_single_reference_is_checked_as_before() {
    let sub = sub("", "$.nodes.hostnode");
    let Err(err) = load(&host_for(&sub, ""), &sub) else {
        panic!("load must refuse");
    };
    assert!(err.contains("graph.call.leak"), "{err}");
    assert!(err.contains("hostnode"), "{err}");
}

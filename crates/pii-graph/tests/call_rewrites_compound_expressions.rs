//! Spec 409: every `$.path` reference inside an inlined sub-graph's expression is renamed by the single-reference rule, wherever
//! it stands. Operators, spacing and string literals are copied byte for byte, a `$.` inside a literal is data, and a
//! reference that names nothing of the sub-graph keeps its text. Only the woven `expr` is read here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_abi::{NodeInput, Outcome};
use pii_core::{Budget, CapSet, Ctx};
use pii_graph::fragment::{self, Woven};
use pii_graph::node::NodeRegistry;
use serde_json::{Value, json};

fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

/// The host: `start -> docs(call) -> done`, mapping the inputs `k` and `rubric`. `PIN` is the sub-graph's real digest.
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
k = "$.input.x"
rubric = "$.input.x"

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

/// Escapes `s` as the body of a TOML basic string.
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A sub-graph `docs` with an optional `header` line (`needs = [...]`), a node `a` holding `a_value`, and a node `b`
/// computing `expr`. `b` reads `a` by its id, and the edge a -> b carries the value.
fn sub(header: &str, a: &str, a_value: &str, expr: &str) -> String {
    format!(
        "name = \"docs\"\nexit = \"b\"\n{header}\n\n[[node]]\nid = \"{a}\"\nkind = \"transform\"\nvalue = \"{}\"\n\n\
         [[node]]\nid = \"b\"\nkind = \"transform\"\nexpr = \"{}\"\n\n\
         [[edge]]\nfrom = \"{a}\"\nport = \"ok\"\nto = \"b\"\nkind = \"data\"\n",
        toml_escape(a_value),
        toml_escape(expr),
    )
}

/// The sub-graph `docs` whose `b` computes `expr`, with `a` as its only other node.
fn sub_expr(expr: &str) -> String {
    sub("", "a", "alpha", expr)
}

fn pin(src: &str) -> String {
    fragment::sha256_hex(src.as_bytes())
}

/// Writes the named graph files under a fresh temp dir and weaves `host` against them. Validation is not run: these
/// tests read the woven text only.
fn weave(host: &str, graphs: &[(&str, &str)]) -> Result<Woven, String> {
    let root = tempfile::tempdir_in(tmp_root()).expect("tempdir under _tmp");
    let graph_dir = root.path().join("graphs");
    std::fs::create_dir_all(&graph_dir).expect("mkdir graphs");
    for (name, src) in graphs {
        std::fs::write(graph_dir.join(format!("{name}.toml")), src).expect("write graph");
    }
    let fragments: PathBuf = root.path().join("fragments");
    let reg = NodeRegistry::builtin();
    fragment::load_host_with_graphs(host, &fragments, std::slice::from_ref(&graph_dir), &reg)
        .map_err(|e| e.to_string())
}

/// Weaves the host against `sub`, with the host's `PIN` set to the real digest of `sub`.
fn weave_sub(sub: &str) -> Woven {
    let host = HOST.replace("PIN", &pin(sub));
    weave(&host, &[("docs", sub)]).expect("weave")
}

/// The woven `expr` of the node `id`.
fn expr_of(woven: &Woven, id: &str) -> String {
    let node = woven
        .spec
        .nodes
        .iter()
        .find(|n| n.id.as_str() == id)
        .unwrap_or_else(|| panic!("no woven node {id}"));
    node.cfg
        .get("expr")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("node {id} has no expr: {}", node.cfg))
        .to_string()
}

/// Weaves `expr` as the sub-graph's `b` and returns the woven `b` expression.
fn woven_expr(expr: &str) -> String {
    expr_of(&weave_sub(&sub_expr(expr)), "docs__b")
}

#[test]
fn a_concatenation_of_a_node_and_an_input_is_renamed_reference_by_reference() {
    let sub = sub(
        "",
        "root",
        "/w",
        r#"$.nodes.root + "/rubrics/" + $.input.rubric + ".md""#,
    );
    let woven = weave_sub(&sub);
    assert_eq!(
        expr_of(&woven, "docs__b"),
        r#"$.nodes.docs__root + "/rubrics/" + $.nodes.docs__in__rubric + ".md""#
    );
}

#[test]
fn a_comparison_renames_both_sides() {
    assert_eq!(
        woven_expr("$.nodes.a == $.input.k"),
        "$.nodes.docs__a == $.nodes.docs__in__k"
    );
}

#[test]
fn a_function_argument_is_renamed() {
    assert_eq!(woven_expr("lower($.nodes.a)"), "lower($.nodes.docs__a)");
    assert_eq!(
        woven_expr("starts_with($.nodes.a, $.input.k)"),
        "starts_with($.nodes.docs__a, $.nodes.docs__in__k)"
    );
}

#[test]
fn a_reference_inside_parentheses_is_renamed() {
    assert_eq!(
        woven_expr(r#"($.nodes.a + "x")"#),
        r#"($.nodes.docs__a + "x")"#
    );
}

#[test]
fn control_a_reference_inside_a_string_literal_is_data() {
    assert_eq!(
        woven_expr(r#""$.nodes.a" + $.nodes.a"#),
        r#""$.nodes.a" + $.nodes.docs__a"#
    );
}

#[test]
fn control_an_escaped_quote_does_not_end_the_literal() {
    // The expression text is: "\" $.nodes.a" + $.nodes.a
    assert_eq!(
        woven_expr(r#""\" $.nodes.a" + $.nodes.a"#),
        r#""\" $.nodes.a" + $.nodes.docs__a"#
    );
}

#[test]
fn control_a_reference_to_no_subgraph_node_keeps_its_text() {
    assert_eq!(
        woven_expr("$.nodes.zzz + $.nodes.a"),
        "$.nodes.zzz + $.nodes.docs__a"
    );
}

#[test]
fn a_host_needs_key_is_left_to_the_host_inside_a_compound() {
    // `secret` is declared in `needs` and the call does not map it: it reads the host run's input.
    let sub = sub(
        "needs = [\"secret\"]",
        "a",
        "alpha",
        "$.input.secret + $.nodes.a",
    );
    let woven = weave_sub(&sub);
    assert_eq!(
        expr_of(&woven, "docs__b"),
        "$.input.secret + $.nodes.docs__a"
    );
}

#[test]
fn a_single_reference_renames_exactly_as_before() {
    assert_eq!(woven_expr("$.nodes.a.rest"), "$.nodes.docs__a.rest");
}

#[test]
fn a_hyphenated_node_id_is_one_reference() {
    let sub = sub("", "method-judge", "alpha", r#"$.nodes.method-judge + "x""#);
    let woven = weave_sub(&sub);
    assert_eq!(
        expr_of(&woven, "docs__b"),
        r#"$.nodes.docs__method-judge + "x""#
    );
}

#[test]
fn the_review_graphs_path_node_evaluates_after_inlining() {
    // The shape of `graphs/review-one.toml`'s `path` node: `root` is a node, `rubric` an input the call maps.
    let sub = sub(
        "",
        "root",
        "/w",
        r#"$.nodes.root + "/rubrics/" + $.input.rubric + ".md""#,
    );
    let woven = weave_sub(&sub);
    let node = woven
        .spec
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "docs__b")
        .expect("docs__b");
    let reg = NodeRegistry::builtin();
    let built = reg
        .factory("transform")
        .expect("transform factory")
        .build(&node.cfg)
        .expect("build");
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    let ni = NodeInput {
        input: json!({}),
        payload: json!({}),
        nodes: json!({"docs__root": "/w", "docs__in__rubric": "alpha"}),
    };
    match built.run(&ctx, &ni).expect("run") {
        Outcome::Next { port, value } => {
            assert_eq!(port.as_str(), "ok", "the path node must not fail: {value}");
            assert_eq!(value, json!("/w/rubrics/alpha.md"));
        }
        _ => panic!("expected Next"),
    }
}

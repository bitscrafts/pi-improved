//! Spec 383 step 1: `transform` computes. `let` binds, `out` produces; integers and
//! strings only; a bad expression fails at load naming the node and the expression.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_abi::{NodeInput, Outcome, PiiError};
use pii_core::{CapSet, Ctx};
use pii_graph::node::{GraphNode, NodeRegistry};
use serde_json::{Value, json};

fn build(cfg: &Value) -> Result<Box<dyn GraphNode>, PiiError> {
    NodeRegistry::builtin().factory("transform")?.build(cfg)
}

fn run(cfg: &Value, input: &Value) -> (String, Value) {
    let node = build(cfg).unwrap();
    let ctx = Ctx::root(CapSet::empty(), pii_core::Budget::unlimited());
    let ni = NodeInput {
        input: input.clone(),
        payload: json!({}),
        nodes: json!({}),
    };
    match node.run(&ctx, &ni).unwrap() {
        Outcome::Next { port, value } => (port.as_str().to_string(), value),
        _ => panic!("expected Next"),
    }
}

fn input() -> Value {
    json!({"branch": "a", "n": 3})
}

#[test]
fn let_binds_and_out_produces_arithmetic() {
    let cfg = json!({"node-id": "calc", "let": ["n = $.input.n", "m = n + 1"], "out": "m"});
    assert_eq!(run(&cfg, &input()), ("ok".into(), json!(4)));
}

#[test]
fn comparisons_produce_booleans() {
    let base = |out: &str| json!({"node-id": "c", "let": ["n = $.input.n"], "out": out});
    assert_eq!(run(&base("n < 5"), &input()), ("ok".into(), json!(true)));
    assert_eq!(run(&base("n > 5"), &input()), ("ok".into(), json!(false)));
    assert_eq!(run(&base("n == 3"), &input()), ("ok".into(), json!(true)));
    assert_eq!(run(&base("n != 3"), &input()), ("ok".into(), json!(false)));
}

#[test]
fn concatenation_and_length() {
    let cfg = json!({"node-id": "s", "let": ["s = $.input.branch + \"-x\""], "out": "s"});
    assert_eq!(run(&cfg, &input()), ("ok".into(), json!("a-x")));
    let cfg = json!({"node-id": "s", "let": ["s = $.input.branch + \"-x\""], "out": "length(s)"});
    assert_eq!(run(&cfg, &input()), ("ok".into(), json!(3)));
}

#[test]
fn overflow_is_refused_at_run() {
    let cfg = json!({"node-id": "o", "let": ["m = $.input.n + 1"], "out": "m"});
    let big = json!({"n": i64::MAX});
    let (port, value) = run(&cfg, &big);
    assert_eq!(port, "err");
    assert!(
        value
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("overflow")
    );
}

#[test]
fn floats_are_refused_at_run() {
    let cfg = json!({"node-id": "f", "out": "$.input.f"});
    let (port, value) = run(&cfg, &json!({"f": 1.5}));
    assert_eq!(port, "err");
    assert!(
        value
            .get("error")
            .and_then(Value::as_str)
            .unwrap()
            .contains("float")
    );
}

#[test]
fn bad_expression_fails_at_load_naming_node_and_expression() {
    let cfg = json!({"node-id": "calc", "let": ["x = $.input.n +"], "out": "x"});
    let err = build(&cfg).err().expect("load must refuse").to_string();
    assert!(err.contains("calc"), "names node: {err}");
    assert!(err.contains("$.input.n +"), "names expression: {err}");
}

#[test]
fn unbound_name_fails_at_load() {
    let cfg = json!({"node-id": "u", "out": "nope + 1"});
    let err = build(&cfg).err().expect("load must refuse").to_string();
    assert!(err.contains("nope + 1"), "names expression: {err}");
}

#[test]
fn literal_overflow_fails_at_load() {
    let cfg = json!({"node-id": "k", "out": "99999999999999999999"});
    let err = build(&cfg).err().expect("load must refuse").to_string();
    assert!(err.contains("overflow"), "{err}");
}

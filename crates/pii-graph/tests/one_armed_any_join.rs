//! Spec 380 guardrail: a `join = "any"` node with ONE inbound data edge runs correctly, so the lint must not refuse it.
//!
//! The engine's readiness for such a node is the same as an `all` join with one arm: ready once the arm fires, dead once
//! the arm is dead, waiting before either. The `validate` assertion below fails if V9 still refuses one-armed `any`
//! joins, which is the proof that the lint verdict was the defect and not the graph.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_abi::{NodeId, PortName};
use pii_graph::sched::{Readiness, RunState, apply_completion, readiness};
use pii_graph::{NodeRegistry, load_toml, validate};

const GRAPH: &str = r#"
name = "one-armed-any"
entry = "start"

[[node]]
id = "start"
kind = "transform"

[[node]]
id = "join"
kind = "transform"
join = "any"

[[edge]]
from = "start"
port = "ok"
to = "join"
kind = "data"
"#;

#[test]
fn one_armed_any_join_validates() {
    let g = load_toml(GRAPH).unwrap();
    let reg = NodeRegistry::builtin();
    assert!(validate(&g, &reg).is_ok(), "{:?}", validate(&g, &reg));
}

#[test]
fn one_armed_any_join_waits_then_runs_or_dies_like_all() {
    let g = load_toml(GRAPH).unwrap();
    let join = NodeId::from("join");
    let start = NodeId::from("start");
    let value = serde_json::json!(1);

    let mut st = RunState::from_spec(&g);
    assert_eq!(readiness(&g, &st, &join), Readiness::NotReady);

    apply_completion(&g, &mut st, &start, &PortName::from("ok"), &value).unwrap();
    assert_eq!(readiness(&g, &st, &join), Readiness::Ready);

    let mut st = RunState::from_spec(&g);
    apply_completion(&g, &mut st, &start, &PortName::from("err"), &value).unwrap();
    assert_eq!(readiness(&g, &st, &join), Readiness::DeadNode);
}

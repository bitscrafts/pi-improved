//! Spec 191: a success terminal must not carry an error payload. A run whose
//! last finished node took the `err` port with `{"error": ...}` is a failure,
//! driven through the engine; a genuinely successful run is untouched.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{ctx, scratch};
use pii_abi::{NodeId, PortName, Terminal};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::spec::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy};
use pii_store::Store;

fn transform(id: &str, cfg: serde_json::Value) -> NodeSpec {
    NodeSpec {
        id: NodeId::from(id),
        kind: "transform".to_owned(),
        join: JoinPolicy::All,
        loop_budget: None,
        retry: RetryPolicy::default(),
        caps: 0,
        cfg,
    }
}

/// `a -> b` over `ok`; `b` is the last node.
fn spec(b_cfg: serde_json::Value) -> GraphSpec {
    GraphSpec {
        name: "success-carries-no-error".into(),
        caps: 0,
        nodes: vec![
            transform("a", serde_json::json!({"value": {"fine": true}})),
            transform("b", b_cfg),
        ],
        edges: vec![Edge {
            from: NodeId::from("a"),
            port: PortName::from("ok"),
            to: NodeId::from("b"),
            kind: EdgeKind::Data,
        }],
        entry: NodeId::from("a"),
        limits: GraphLimits::default(),
    }
}

fn run_terminal(spec: &GraphSpec) -> Terminal {
    let dir = scratch("success-no-error-");
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let id = store.next_run_id().unwrap();
    run::run(
        &store,
        spec,
        &NodeRegistry::builtin(),
        &ctx(id.clone()),
        &id,
        serde_json::Value::Null,
    )
    .unwrap()
}

#[test]
fn a_last_node_on_the_err_port_is_not_a_success_carrying_its_error() {
    // A malformed expression makes the transform node take `err` with
    // `{"error": "expression must start with '$.' ..."}`.
    let terminal = run_terminal(&spec(serde_json::json!({"expr": "no-dollar"})));
    match terminal {
        Terminal::Failure { error } => assert!(error.contains("expression"), "{error}"),
        other => panic!("a success terminal must not carry an error: {other:?}"),
    }
}

#[test]
fn a_genuinely_successful_run_is_still_a_success_with_its_value() {
    let terminal = run_terminal(&spec(serde_json::json!({"value": {"answer": 42}})));
    assert_eq!(
        terminal,
        Terminal::Success {
            output: serde_json::json!({"answer": 42})
        }
    );
}

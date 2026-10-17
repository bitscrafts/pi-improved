//! Spec 162: a node that can block on a process says it is alive
//! (`NodeProgress`) and must bound itself (`timeout_ms` is mandatory).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_abi::{Effects, NodeId, PiiError, PortName, RunEvent};
use pii_core::CapKind;
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::{GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy, validate};

struct ProcFactory;

impl NodeFactory for ProcFactory {
    fn kind(&self) -> &'static str {
        "procnode"
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }
    fn required_caps(&self) -> CapKind {
        CapKind::PROC
    }
    fn effects(&self) -> Effects {
        Effects::SPAWNS_PROC
    }
    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Err(PiiError::Internal("not built in this test".into()))
    }
}

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(ProcFactory));
    reg
}

fn spec_with(cfg: serde_json::Value) -> GraphSpec {
    GraphSpec {
        name: "t".into(),
        caps: CapKind::PROC.bits(),
        nodes: vec![NodeSpec {
            id: NodeId::from("p"),
            kind: "procnode".into(),
            join: JoinPolicy::All,
            loop_budget: None,
            retry: RetryPolicy::default(),
            caps: 0,
            cfg,
        }],
        edges: vec![],
        entry: NodeId::from("p"),
        limits: GraphLimits::default(),
    }
}

#[test]
fn a_process_node_without_timeout_ms_is_rejected() {
    let errs = validate(&spec_with(serde_json::json!({})), &registry()).unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.starts_with("V10") && e.contains("timeout_ms")),
        "{errs:?}"
    );
}

#[test]
fn a_zero_or_non_numeric_timeout_ms_is_rejected() {
    for bad in [serde_json::json!(0), serde_json::json!("5000")] {
        let errs = validate(
            &spec_with(serde_json::json!({ "timeout_ms": bad })),
            &registry(),
        )
        .unwrap_err();
        assert!(errs.iter().any(|e| e.starts_with("V10")), "{errs:?}");
    }
}

#[test]
fn a_process_node_with_timeout_ms_validates() {
    validate(
        &spec_with(serde_json::json!({ "timeout_ms": 5000 })),
        &registry(),
    )
    .unwrap();
}

#[test]
fn a_non_process_node_needs_no_timeout_ms() {
    let mut spec = spec_with(serde_json::json!({ "value": 1 }));
    spec.caps = 0;
    spec.nodes[0].kind = "transform".into();
    validate(&spec, &registry()).unwrap();
}

#[test]
fn node_progress_carries_elapsed_and_child_cpu_through_the_journal_encoding() {
    let ev = RunEvent::NodeProgress {
        node: NodeId::from("p"),
        elapsed_ms: 1500,
        child_cpu_ms: 900,
    };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(
        json.contains("elapsed_ms") && json.contains("child_cpu_ms"),
        "{json}"
    );
    let back: RunEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ev);
}

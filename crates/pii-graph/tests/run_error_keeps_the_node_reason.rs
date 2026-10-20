//! Spec 224 R1 and R4: the run-level agent-call check keeps a node's own error, and it judges only a node whose own
//! outcome is success.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_abi::{NodeId, PortName, RunEvent, Terminal};
use pii_graph::NodeRegistry;
use pii_graph::agent_calls::call_verdict;

const REASON: &str = "criteria.unknown: 'cargo test' is not in the criteria table";

/// A one-node graph whose only node is a `coder`.
fn coder_spec() -> pii_graph::spec::GraphSpec {
    let src = "name = \"reason\"\nentry = \"code\"\n\n[[node]]\nid = \"code\"\nkind = \"transform\"\nvalue = \"x\"\n";
    let mut spec = pii_graph::load_toml_with_registry(src, &NodeRegistry::builtin()).unwrap();
    "coder".clone_into(&mut spec.nodes[0].kind);
    spec
}

fn journal(events: Vec<RunEvent>) -> Vec<(u64, RunEvent)> {
    events
        .into_iter()
        .enumerate()
        .map(|(i, e)| (i as u64, e))
        .collect()
}

/// A coder node that was refused and failed: started, then failed with its own reason, with no call events.
fn refused_node_events() -> Vec<RunEvent> {
    vec![
        RunEvent::NodeStarted {
            node: NodeId::from("code"),
            at_ms: 1,
        },
        RunEvent::NodeFailed {
            node: NodeId::from("code"),
            error: REASON.to_owned(),
            retryable: false,
            attempt: 1,
        },
    ]
}

/// A failed run keeps the failing node's reason as its terminal error, verbatim.
#[test]
fn run_error_is_the_node_reason_verbatim() {
    let terminal = Terminal::Failure {
        error: REASON.to_owned(),
    };
    let (kept, _) = call_verdict(&coder_spec(), &journal(refused_node_events()), terminal);
    match kept {
        Terminal::Failure { error } => assert_eq!(error, REASON),
        other => panic!("the node's reason must survive, got {other:?}"),
    }
}

/// A coder node that failed is not a silent success: the run-level check must not rewrite a success it did not earn.
#[test]
fn failed_coder_node_is_not_judged_silent() {
    let mut events = refused_node_events();
    events.push(RunEvent::NodeFinished {
        node: NodeId::from("other"),
        port: PortName::from("ok"),
        value: serde_json::json!({}),
        value_hash: String::new(),
        at_ms: 2,
        dur_ms: 1,
    });
    let terminal = Terminal::Success {
        output: serde_json::json!({}),
    };
    let (kept, _) = call_verdict(&coder_spec(), &journal(events), terminal);
    assert!(
        matches!(kept, Terminal::Success { .. }),
        "a failed node must not be re-labelled coder.no_call_events, got {kept:?}"
    );
}

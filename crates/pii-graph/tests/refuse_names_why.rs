//! A `refuse` node says WHY it refused when the verdict that reached it carries a reason.
//!
//! Measured 2026-11-05: `sdd`'s `contract` judge refuses a row whose exit criteria already pass ("expect fail:
//! criterion passed: <cmd> (rc 0)"), and that verdict reaches the `refused` node. The node ignored it and the
//! run failed as `run.refused{hook=coder, reason=the agent failed}` — for rows that never reached a coder. Eight
//! dispatches in one night were recorded as worker failures for that reason, and nothing in the message said so.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_abi::{NodeInput, Outcome};
use pii_core::{Budget, CapSet, Ctx};
use pii_graph::node::NodeRegistry;

fn run(cfg: &serde_json::Value, payload: serde_json::Value) -> String {
    let node = NodeRegistry::builtin()
        .factory("refuse")
        .unwrap()
        .build(cfg)
        .unwrap();
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    let input = NodeInput {
        input: serde_json::json!({}),
        payload,
        nodes: serde_json::json!({}),
    };
    match node.run(&ctx, &input).unwrap() {
        Outcome::Fail { error } => error,
        other => panic!("a refusal with no `returns` must fail the run, got {other:?}"),
    }
}

fn cfg() -> serde_json::Value {
    serde_json::json!({ "hook": "coder", "reason": "the agent failed or the criteria could not run" })
}

#[test]
fn a_refusal_carrying_a_reason_names_it() {
    let error = run(
        &cfg(),
        serde_json::json!({ "contract": { "verdict": "not-red",
            "reason": "expect fail: criterion passed: grep -q 'graph = \"docs\"' scripts/run-batchC-pii.sh (rc 0)" } }),
    );
    assert!(
        error.contains("expect fail: criterion passed: grep -q"),
        "the refusal must carry the judge's reason: {error}"
    );
    assert!(
        error.contains("hook=coder"),
        "the hook is still named: {error}"
    );
}

#[test]
fn an_unrunnable_criterion_is_named_too() {
    let error = run(
        &cfg(),
        serde_json::json!({ "verdict": "unrunnable", "reason": "judge: cannot run cargo test -q (rc 127)" }),
    );
    assert!(
        error.contains("cannot run cargo test -q (rc 127)"),
        "{error}"
    );
}

// CONTROL: a refusal that arrives with NO reason (a coder that really failed) keeps the old text, exactly.
#[test]
fn a_refusal_without_a_reason_keeps_the_configured_text() {
    let error = run(&cfg(), serde_json::json!({ "error": "agent exited 1" }));
    assert_eq!(
        error,
        "run.refused{hook=coder, reason=the agent failed or the criteria could not run}"
    );
}

// CONTROL: an empty payload — nothing reached the node — is the same as no reason.
#[test]
fn an_empty_payload_keeps_the_configured_text() {
    let error = run(&cfg(), serde_json::Value::Null);
    assert_eq!(
        error,
        "run.refused{hook=coder, reason=the agent failed or the criteria could not run}"
    );
}

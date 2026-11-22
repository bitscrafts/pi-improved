//! A `refuse` node says WHY it refused — and, when the verdict that reached it carries no reason, WHICH ARM did.
//!
//! Measured 2026-11-05: `sdd`'s `contract` judge refuses a row whose exit criteria already pass ("expect fail:
//! criterion passed: <cmd> (rc 0)"), and that verdict reaches the `refused` node. The node ignored it and the
//! run failed as `run.refused{hook=coder, reason=the agent failed}` — for rows that never reached a coder. Eight
//! dispatches in one night were recorded as worker failures for that reason, and nothing in the message said so.
//!
//! Measured 2026-11-22: one `refuse` node was fed by TWO ports of the `contract` judge (`fail` and `unrunnable`)
//! with the single reason "the agent failed or the criteria could not run". When the arriving verdict carried no
//! reason, the node printed only that constant, so the log could not say which arm had fired. A join's payload is
//! keyed by source, and its values carry the arm's own `verdict` word, so the node now names them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::{NodeInput, Outcome, RunEvent, Terminal};
use pii_core::{Budget, CapSet, Ctx, Observer, RecordingObserver, TraceCtx};
use pii_graph::fragment;
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_store::Store;

fn run_node(cfg: &serde_json::Value, payload: serde_json::Value) -> String {
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

/// The reason `graphs/sdd.toml`'s `refused_unrunnable` node declares: one sentence, one cause.
fn cfg_unrunnable() -> serde_json::Value {
    serde_json::json!({
        "hook": "coder",
        "reason": "the criteria could not run at all, so nothing was verified",
    })
}

#[test]
fn a_refusal_carrying_a_reason_names_it() {
    let error = run_node(
        &cfg(),
        serde_json::json!({ "contract": { "verdict": "not-red",
            "reason": "expect fail: criterion passed: cargo test -q (rc 0)" } }),
    );
    // Bit for bit what the node printed before arms were named: a renderable reason is still the answer, and it
    // takes precedence — no `(arms: …)` is appended when the verdict already says why.
    assert_eq!(
        error,
        "run.refused{hook=coder, reason=the agent failed or the criteria could not run: \
         expect fail: criterion passed: cargo test -q (rc 0)}"
    );
}

#[test]
fn an_unrunnable_criterion_is_named_too() {
    let error = run_node(
        &cfg(),
        serde_json::json!({ "verdict": "unrunnable", "reason": "judge: cannot run cargo test -q (rc 127)" }),
    );
    assert!(
        error.contains("cannot run cargo test -q (rc 127)"),
        "{error}"
    );
}

// The arm says which port it left on, so the sentence names the cause and who reported it.
#[test]
fn a_refusal_without_a_reason_names_the_arm_that_arrived() {
    let error = run_node(
        &cfg_unrunnable(),
        serde_json::json!({ "contract": { "verdict": "unrunnable", "criteria": 4 } }),
    );
    assert_eq!(
        error,
        "run.refused{hook=coder, reason=the criteria could not run at all, so nothing was verified \
         (arms: contract.unrunnable)}"
    );
}

// PROOF BY CONTRAST: the same node, the same arm, one verdict carrying a reason and one carrying none. The two
// messages must differ, and only the second has to name the arm — otherwise the reason is being ignored, or the
// arms section is being printed when a better answer was already there.
#[test]
fn an_arm_with_a_reason_and_an_arm_without_produce_different_messages() {
    let with_reason = run_node(
        &cfg_unrunnable(),
        serde_json::json!({ "contract": { "verdict": "unrunnable",
            "reason": "judge: cannot run cargo test -q (rc 127)" } }),
    );
    let without_reason = run_node(
        &cfg_unrunnable(),
        serde_json::json!({ "contract": { "verdict": "unrunnable" } }),
    );
    assert_ne!(
        with_reason, without_reason,
        "a verdict that says why must not read like one that does not"
    );
    assert!(
        without_reason.contains("(arms: contract.unrunnable)"),
        "the arm that carried no reason must be named: {without_reason}"
    );
    assert!(
        with_reason.contains("judge: cannot run cargo test -q (rc 127)"),
        "the reason still wins: {with_reason}"
    );
}

// CONTROL: a refusal that arrives with NO reason and no arms (a coder that really failed) keeps the old text.
#[test]
fn a_refusal_without_a_reason_keeps_the_configured_text() {
    let error = run_node(&cfg(), serde_json::json!({ "error": "agent exited 1" }));
    assert_eq!(
        error,
        "run.refused{hook=coder, reason=the agent failed or the criteria could not run}"
    );
}

// CONTROL: an empty payload — nothing reached the node — is the same as no reason, and prints no empty brackets.
#[test]
fn an_empty_payload_keeps_the_configured_text() {
    let error = run_node(&cfg(), serde_json::Value::Null);
    assert_eq!(
        error,
        "run.refused{hook=coder, reason=the agent failed or the criteria could not run}"
    );
}

// CONTROL: a node with ONE inbound edge receives the verdict itself, not a join keyed by source. Its keys are the
// judge's fields (`verdict`, `spec`, `base`), not arms, and naming them would be nonsense — it stays plain.
#[test]
fn a_single_verdict_payload_stays_plain() {
    let error = run_node(
        &cfg(),
        serde_json::json!({ "verdict": "failed", "spec": "01-x.md", "base": "abc", "criteria": 3 }),
    );
    assert_eq!(
        error,
        "run.refused{hook=coder, reason=the agent failed or the criteria could not run}"
    );
}

/// The live path: two exclusive arms of one route reach one `refuse` node. Whichever port fires decides which
/// transform's verdict arrives — and the payload the scheduler builds for the refusal is the join, keyed by source.
const JOIN_GRAPH: &str = r#"
name = "join"
entry = "pick"
exit = "boom"

[[node]]
id = "pick"
kind = "route"
on = "$.input.which"
rules = [ { eq = "unrunnable", port = "unrunnable" }, { port = "failed" } ]

[[node]]
id = "criteria_unrunnable"
kind = "transform"
value = { verdict = "unrunnable", criteria = 4 }

[[node]]
id = "criteria_failed"
kind = "transform"
value = { verdict = "not-red", criteria = 4 }

[[node]]
id = "boom"
kind = "refuse"
join = "any"
hook = "coder"
reason = "the criteria could not run at all, so nothing was verified"

[[edge]]
from = "pick"
port = "unrunnable"
to = "criteria_unrunnable"
kind = "data"

[[edge]]
from = "pick"
port = "failed"
to = "criteria_failed"
kind = "data"

[[edge]]
from = "criteria_unrunnable"
port = "ok"
to = "boom"
kind = "data"

[[edge]]
from = "criteria_failed"
port = "ok"
to = "boom"
kind = "data"
"#;

fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

/// Runs [`JOIN_GRAPH`] with `which` selecting the arm, and returns the refusal the failing node journalled.
fn run_join(which: &str) -> String {
    let dir = tempfile::Builder::new()
        .prefix("refuse-arms-")
        .tempdir_in(tmp_root())
        .expect("tempdir under _tmp");
    let woven = fragment::load_host_with_graphs(
        JOIN_GRAPH,
        &dir.path().join("fragments"),
        &[dir.path().to_path_buf()],
        &NodeRegistry::builtin(),
    )
    .expect("the join graph loads");
    let store_dir = tempfile::Builder::new()
        .prefix("store-")
        .tempdir_in(tmp_root())
        .expect("tempdir under _tmp");
    let store = Store::open(&store_dir.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let obs = Arc::new(RecordingObserver::new());
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::clone(&obs) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    let terminal = run::run(
        &store,
        &woven.spec,
        &NodeRegistry::builtin(),
        &ctx,
        &run_id,
        serde_json::json!({ "which": which }),
    )
    .expect("a node failure is data, not an Err");
    assert!(
        !matches!(terminal, Terminal::Success { .. }),
        "the refusal must fail the run: {terminal:?}"
    );
    store
        .read_journal(&run_id)
        .expect("journal")
        .into_iter()
        .find_map(|(_, ev)| match ev {
            RunEvent::NodeFailed { node, error, .. } if node.as_str() == "boom" => Some(error),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no NodeFailed for `boom` in the journal"))
}

// The live path, both arms: only the arm that fired is named, and the two refusals are not the same sentence.
#[test]
fn the_scheduler_built_join_names_the_arm_that_fired() {
    let unrunnable = run_join("unrunnable");
    let failed = run_join("failed");
    assert_eq!(
        unrunnable,
        "run.refused{hook=coder, reason=the criteria could not run at all, so nothing was verified \
         (arms: criteria_unrunnable.unrunnable)}"
    );
    assert_eq!(
        failed,
        "run.refused{hook=coder, reason=the criteria could not run at all, so nothing was verified \
         (arms: criteria_failed.not-red)}"
    );
    assert_ne!(unrunnable, failed);
}

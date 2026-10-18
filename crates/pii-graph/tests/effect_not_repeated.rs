//! Spec 186: an effect node records its key before the effect and confirms it
//! after; a replay never repeats a confirmed effect and refuses an
//! unconfirmed one with `effect.unknown`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{Probe, chain, ctx, finished, registry, scratch, seed_run};
use pii_abi::{NodeId, NodeInput, PiiError, PortName, RunEvent, Terminal};
use pii_graph::effect::{idempotency_key, input_digest};
use pii_graph::run;
use pii_store::Store;

fn effect_chain() -> pii_graph::GraphSpec {
    chain([("probe", "ok"), ("effect", "ok"), ("probe", "ok")])
}

fn input(payload: serde_json::Value) -> NodeInput {
    NodeInput {
        input: serde_json::Value::Null,
        payload,
        nodes: serde_json::Value::Null,
    }
}

#[test]
fn a_live_effect_journals_its_key_before_and_confirms_after() {
    let dir = scratch("effect-live-");
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let probe = Probe::default();
    let reg = registry(&probe);
    let id = store.next_run_id().unwrap();
    run::run(
        &store,
        &effect_chain(),
        &reg,
        &ctx(id.clone()),
        &id,
        serde_json::Value::Null,
    )
    .unwrap();
    let journal = store.read_journal(&id).unwrap();
    let pos = |pred: &dyn Fn(&RunEvent) -> bool| journal.iter().position(|(_, e)| pred(e));
    let started = pos(&|e| matches!(e, RunEvent::EffectStarted { .. })).expect("EffectStarted");
    let node_started =
        pos(&|e| matches!(e, RunEvent::NodeStarted { node, .. } if node.as_str() == "b")).unwrap();
    let confirmed =
        pos(&|e| matches!(e, RunEvent::EffectConfirmed { .. })).expect("EffectConfirmed");
    let finished_b =
        pos(&|e| matches!(e, RunEvent::NodeFinished { node, .. } if node.as_str() == "b")).unwrap();
    assert!(node_started < started && started < confirmed && confirmed < finished_b);
    // The pure nodes journal no effect events.
    let n = journal
        .iter()
        .filter(|(_, e)| matches!(e, RunEvent::EffectStarted { .. }))
        .count();
    assert_eq!(n, 1);
    // The key is derived from (run, node, input digest): a's output is b's payload.
    let RunEvent::EffectStarted { key, .. } = &journal[started].1 else {
        panic!("not EffectStarted");
    };
    let payload = serde_json::json!({"node": "a", "n": 1});
    let want = idempotency_key(&id, &NodeId::from("b"), &input_digest(&input(payload)));
    assert_eq!(key, &want);
}

#[test]
fn input_digest_changes_the_key() {
    let run = pii_abi::RunId::from("r1");
    let node = NodeId::from("b");
    let k1 = idempotency_key(
        &run,
        &node,
        &input_digest(&input(serde_json::json!({"x": 1}))),
    );
    let k2 = idempotency_key(
        &run,
        &node,
        &input_digest(&input(serde_json::json!({"x": 2}))),
    );
    let k1_again = idempotency_key(
        &run,
        &node,
        &input_digest(&input(serde_json::json!({"x": 1}))),
    );
    assert_ne!(k1, k2, "a different input is a NEW key");
    assert_eq!(k1, k1_again, "the same input is the same key");
    assert_ne!(
        k1,
        idempotency_key(
            &pii_abi::RunId::from("r2"),
            &node,
            &input_digest(&input(serde_json::json!({"x": 1})))
        ),
        "and a different run"
    );
}

/// The crash gap: the effect happened and was confirmed, `NodeFinished` never
/// landed. Resume completes the node from the journal; the effect does not run.
#[test]
fn effect_not_repeated() {
    let dir = scratch("effect-orphan-");
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let probe = Probe::default();
    let reg = registry(&probe);
    let spec = effect_chain();
    let recorded = serde_json::json!({"node": "b", "n": 1, "posted": true});
    let id = seed_run(
        &store,
        &spec,
        &[
            finished("a", serde_json::json!({"node": "a", "n": 1})),
            RunEvent::EffectStarted {
                node: NodeId::from("b"),
                key: "k".into(),
            },
            RunEvent::EffectConfirmed {
                node: NodeId::from("b"),
                key: "k".into(),
                port: PortName::from("ok"),
                value: recorded.clone(),
            },
        ],
    );
    let out = run::resume(&store, &reg, &ctx(id.clone()), &id, None).unwrap();
    assert!(matches!(out, Terminal::Success { .. }), "{out:?}");
    assert_eq!(probe.count("b"), 0, "the confirmed effect is not repeated");
    assert_eq!(probe.count("a"), 0);
    assert_eq!(probe.count("c"), 1, "the rest of the run continues");
    let journal = store.read_journal(&id).unwrap();
    assert!(
        journal.iter().any(|(_, e)| matches!(e,
            RunEvent::NodeFinished { node, value, .. } if node.as_str() == "b" && *value == recorded)),
        "the node is finished with the recorded output"
    );
    let first_seen = probe.seen.lock()[0].1.clone();
    assert_eq!(
        first_seen["b"], recorded,
        "downstream sees the reused output"
    );
}

/// Started, never confirmed: unknown whether it happened. Refuse; append nothing.
#[test]
fn unconfirmed_effect_refuses_resume() {
    let dir = scratch("effect-unknown-");
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let probe = Probe::default();
    let reg = registry(&probe);
    let spec = effect_chain();
    let id = seed_run(
        &store,
        &spec,
        &[
            finished("a", serde_json::json!({"node": "a", "n": 1})),
            RunEvent::EffectStarted {
                node: NodeId::from("b"),
                key: "k".into(),
            },
        ],
    );
    let before = store.read_journal(&id).unwrap();
    let err = run::resume(&store, &reg, &ctx(id.clone()), &id, None).unwrap_err();
    assert!(
        matches!(&err, PiiError::Invalid(m) if m.contains("effect.unknown")),
        "{err:?}"
    );
    assert_eq!(probe.count("b"), 0, "the effect is not run");
    assert_eq!(store.read_journal(&id).unwrap(), before, "nothing appended");

    // The operator's explicit decision: retry accepts the risk and runs it.
    let out = run::retry(&store, &reg, &ctx(id.clone()), &id, &NodeId::from("b")).unwrap();
    assert!(matches!(out, Terminal::Success { .. }), "{out:?}");
    assert_eq!(probe.count("b"), 1);
}

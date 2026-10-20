//! Spec 217: a coder call is observable as a pair of events. `coder.spawned` carries the identity (agent, model,
//! argv digest, pid) and `coder.exited` closes the same pid with the exit code, the duration and the verdict counts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{NodeInput, Outcome};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_observe::{Event, Observer};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// A fake `pi` that sleeps a little (so the duration is measurable) and exits with `code`.
fn fake_pi(bin: &std::path::Path, code: i32) {
    let pi = bin.join("pi");
    std::fs::write(&pi, format!("#!/bin/sh\nsleep 0.2\nexit {code}\n")).unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

fn root_ctx(bin: &std::path::Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&BTreeMap::new()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

fn field<'a>(e: &'a Event, key: &str) -> Option<&'a serde_json::Value> {
    e.fields.get(key)
}

#[test]
fn coder_call_is_observable() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    fake_pi(bin.path(), 7);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let log = Arc::new(Rec::default());
    let factory = CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log.clone());
    let cfg = serde_json::json!({
        "agent": "pi",
        "model": "m-one",
        "brief": "do it",
        "spec": "specs/217-x.md",
        "workdir": work.path().display().to_string(),
    });
    let node = pii_graph::node::NodeFactory::build(&factory, &cfg).unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    let outcome = node.run(&root_ctx(bin.path()), &input).unwrap();
    assert!(
        matches!(&outcome, Outcome::Next { port, .. } if port.as_str() == "err"),
        "a non-zero exit routes to err: {outcome:?}"
    );

    let events = log.0.lock().clone();
    let spawned = events
        .iter()
        .find(|e| e.kind == "coder.spawned")
        .expect("coder.spawned journalled");
    let exited = events
        .iter()
        .find(|e| e.kind == "coder.exited")
        .expect("coder.exited journalled");
    assert_spawn_identity(spawned);
    assert_exit_closes_spawn(spawned, exited);
}

/// The identity a `coder.spawned` carries (spec 217 R2): who ran, with what argv digest, and under which pid.
fn assert_spawn_identity(spawned: &Event) {
    for key in [
        "agent",
        "model",
        "image",
        "image_digest",
        "argv0",
        "argv_digest",
        "session",
    ] {
        assert!(field(spawned, key).is_some(), "coder.spawned carries {key}");
    }
    assert_eq!(
        field(spawned, "agent").and_then(serde_json::Value::as_str),
        Some("pi")
    );
    assert_eq!(
        field(spawned, "model").and_then(serde_json::Value::as_str),
        Some("m-one")
    );
    assert!(
        field(spawned, "argv_len")
            .and_then(serde_json::Value::as_i64)
            .is_some_and(|n| n > 0),
        "argv_len is recorded"
    );
}

/// The exit closes the same pid and carries the child's code, a measured duration and the verdict counts (R4).
fn assert_exit_closes_spawn(spawned: &Event, exited: &Event) {
    let pid = field(spawned, "pid").and_then(serde_json::Value::as_i64);
    assert!(pid.is_some_and(|p| p > 0), "spawned pid: {pid:?}");
    assert_eq!(
        field(exited, "pid").and_then(serde_json::Value::as_i64),
        pid
    );
    assert_eq!(
        field(exited, "rc").and_then(serde_json::Value::as_i64),
        Some(7)
    );
    assert!(
        field(exited, "dur_ms")
            .and_then(serde_json::Value::as_i64)
            .is_some_and(|d| d > 0),
        "dur_ms is measured: {:?}",
        field(exited, "dur_ms")
    );
    assert_eq!(
        field(exited, "stop_reason").and_then(serde_json::Value::as_str),
        Some("end_turn")
    );
    assert_eq!(
        field(exited, "criteria_run").and_then(serde_json::Value::as_i64),
        Some(0)
    );
    assert_eq!(
        field(exited, "criteria_failed").and_then(serde_json::Value::as_i64),
        Some(0)
    );
    assert_eq!(
        field(exited, "first_failure").and_then(serde_json::Value::as_str),
        Some("")
    );
    assert!(
        field(exited, "cost_usd").is_none(),
        "cost is optional and absent for pi"
    );
}

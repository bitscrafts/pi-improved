//! Spec 217 R3: a secret never reaches the journal. The argv is recorded as a digest, an argv0 and a length; the
//! environment is recorded by NAME only. A literal secret from the brief or the daemon environment appears in no event.

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

#[test]
fn coder_secret_never_journalled() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let pi = bin.path().join("pi");
    std::fs::write(&pi, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();

    let daemon_env = BTreeMap::from([(
        String::from("ANTHROPIC_API_KEY"),
        String::from("sk-env-VALUE"),
    )]);
    let log = Arc::new(Rec::default());
    let factory = CoderFactory::with_env(bin.path().to_path_buf(), daemon_env, log.clone());
    let cfg = serde_json::json!({
        "agent": "pi",
        "model": "m-one",
        "brief": "fix it --api-key sk-live-XYZ",
        "spec": "specs/217-x.md",
        "workdir": work.path().display().to_string(),
        "env_pass": ["ANTHROPIC_API_KEY"],
    });
    let node = pii_graph::node::NodeFactory::build(&factory, &cfg).unwrap();

    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin.path()));
    argv0.extend(criteria::programs(&BTreeMap::new()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    let outcome = node
        .run(&Ctx::root(caps, Budget::unlimited()), &input)
        .unwrap();
    assert!(matches!(outcome, Outcome::Next { .. }), "got {outcome:?}");

    let events = log.0.lock().clone();
    for e in &events {
        let dump = format!("{:?}", e.fields);
        assert!(
            !dump.contains("sk-live-XYZ"),
            "brief secret in {}: {dump}",
            e.kind
        );
        assert!(
            !dump.contains("sk-env-VALUE"),
            "env value in {}: {dump}",
            e.kind
        );
    }

    let spawned = events
        .iter()
        .find(|e| e.kind == "coder.spawned")
        .expect("coder.spawned journalled");
    let digest = spawned
        .fields
        .get("argv_digest")
        .and_then(serde_json::Value::as_str)
        .expect("argv_digest recorded");
    assert_eq!(digest.len(), 64, "a sha256 hex digest: {digest}");
    assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(spawned.fields.get("argv0").is_some(), "argv0 recorded");
    assert!(
        spawned.fields.get("argv_len").is_some(),
        "argv_len recorded"
    );
    assert!(
        spawned.fields.get("argv").is_none(),
        "the raw argv is never journalled"
    );

    let names = events
        .iter()
        .find(|e| e.kind == "coder.env.names")
        .expect("coder.env.names journalled");
    assert_eq!(
        names
            .fields
            .get("names")
            .and_then(serde_json::Value::as_str),
        Some("ANTHROPIC_API_KEY")
    );
}

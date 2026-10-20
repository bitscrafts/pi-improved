//! Spec 217: a refusal is an event before it is an error. A coder node whose image is absent journals
//! `worker.start.refused` with its reason and no pid, routes it to `err`, and never spawns or exits anything.

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

/// A fake `podman` whose `image inspect` fails, so the declared image is absent.
fn fake_podman_without_image(dir: &std::path::Path) -> std::path::PathBuf {
    let podman = dir.join("podman");
    std::fs::write(&podman, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&podman, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    podman
}

#[test]
fn refusal_precedes_the_spawn() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let podman = fake_podman_without_image(bin.path());
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let log = Arc::new(Rec::default());
    let factory = CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log.clone())
        .with_podman(podman.clone());
    let cfg = serde_json::json!({
        "agent": "pi",
        "model": "m-one",
        "brief": "do it",
        "spec": "specs/217-x.md",
        "workdir": work.path().display().to_string(),
        "container_image": "pii/absent:1",
    });
    let node = pii_graph::node::NodeFactory::build(&factory, &cfg).unwrap();

    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh"), podman.display().to_string()];
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
    assert!(
        matches!(&outcome, Outcome::Next { port, .. } if port.as_str() == "err"),
        "a refusal routes to err: {outcome:?}"
    );

    let events = log.0.lock().clone();
    let refused: Vec<&Event> = events
        .iter()
        .filter(|e| e.kind == "worker.start.refused")
        .collect();
    let [refusal] = refused.as_slice() else {
        panic!("exactly one refusal journalled: {events:?}");
    };
    assert!(
        refusal
            .fields
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|r| !r.is_empty()),
        "the refusal names its reason"
    );
    assert!(
        refusal.fields.get("pid").is_none(),
        "a refusal carries no pid"
    );
    assert!(
        events
            .iter()
            .all(|e| e.kind != "coder.spawned" && e.kind != "coder.exited"),
        "nothing was spawned or exited: {events:?}"
    );
}

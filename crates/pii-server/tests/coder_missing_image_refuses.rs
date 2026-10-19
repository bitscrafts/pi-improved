//! Spec 209: a `coder` node whose image is absent is a REFUSAL, not an install. The node journals
//! `coder.image.missing` and routes to the `err` port (the heal port) without starting a container; podman is
//! never asked to build or pull.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{NodeInput, Outcome};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::CoderFactory;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// A fake podman that knows no images: every `image inspect` fails, and any other verb is journalled too.
fn fake_podman_without_images(dir: &Path) -> std::path::PathBuf {
    let log = dir.join("podman.log");
    let script = format!("#!/bin/sh\necho \"$*\" >> '{}'\nexit 1\n", log.display());
    let podman = dir.join("podman");
    std::fs::write(&podman, script).unwrap();
    std::fs::set_permissions(&podman, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    podman
}

#[test]
fn missing_image_refuses_on_the_err_port() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let podman = fake_podman_without_images(bin.path());
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let log = Arc::new(Rec::default());
    let factory = CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log.clone())
        .with_podman(podman.clone());
    let node = pii_graph::node::NodeFactory::build(
        &factory,
        &serde_json::json!({
            "agent": "pi",
            "model": "m",
            "brief": "b",
            "spec": "specs/209-x.md",
            "workdir": work.path().display().to_string(),
            "container_image": "pii-coder:never-built",
        }),
    )
    .unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(vec![podman.display().to_string()]);
    let outcome = node
        .run(&Ctx::root(caps, Budget::unlimited()), &input)
        .unwrap();

    match outcome {
        Outcome::Next { port, .. } => assert_eq!(port.as_str(), "err"),
        other => panic!("expected the err port, got {other:?}"),
    }
    assert!(
        log.0
            .lock()
            .iter()
            .any(|e| e.kind == names::CODER_IMAGE_MISSING),
        "coder.image.missing journalled"
    );
    let invoked = std::fs::read_to_string(bin.path().join("podman.log")).unwrap_or_default();
    assert!(
        !invoked.lines().any(|l| l.starts_with("run ")),
        "no container started: {invoked}"
    );
    assert!(!log.0.lock().iter().any(|e| e.kind == names::CODER_SPAWNED));
}

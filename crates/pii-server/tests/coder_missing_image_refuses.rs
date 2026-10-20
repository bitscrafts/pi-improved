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
/// `stderr_line` is what the fake writes to stderr before exiting 1 (`None` = silent failure).
fn fake_podman_without_images(dir: &Path, stderr_line: Option<&str>) -> std::path::PathBuf {
    let log = dir.join("podman.log");
    let err = stderr_line
        .map(|l| format!("echo '{l}' >&2\n"))
        .unwrap_or_default();
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\n{err}exit 1\n",
        log.display()
    );
    let podman = dir.join("podman");
    std::fs::write(&podman, script).unwrap();
    std::fs::set_permissions(&podman, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    podman
}

/// Builds the coder node against `podman` and runs it, returning the outcome port and the events.
fn refuse_outcome(podman: &Path, work: &Path) -> (String, Vec<Event>) {
    let bin = work.parent().unwrap().join("bin");
    let log = Arc::new(Rec::default());
    let factory =
        CoderFactory::with_env(bin, BTreeMap::new(), log.clone()).with_podman(podman.to_path_buf());
    let node = pii_graph::node::NodeFactory::build(
        &factory,
        &serde_json::json!({
            "agent": "pi",
            "model": "m",
            "brief": "b",
            "spec": "specs/209-x.md",
            "workdir": work.display().to_string(),
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
    let port = match outcome {
        Outcome::Next { port, .. } => port.as_str().to_owned(),
        other => panic!("expected the err port, got {other:?}"),
    };
    (port, log.0.lock().clone())
}

fn image_missing<'a>(events: &'a [Event]) -> &'a Event {
    events
        .iter()
        .find(|e| e.kind == names::CODER_IMAGE_MISSING)
        .expect("coder.image.missing journalled")
}

/// Spec 228 R2 control (absence clause only without stderr): a silent inspect failure refuses with the
/// no-stderr clause, and the old invented cause is gone from the tree's behaviour entirely.
#[test]
fn missing_image_refuses_on_the_err_port() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let podman = fake_podman_without_images(bin.path(), None);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    std::fs::create_dir_all(work.path().parent().unwrap().join("bin")).unwrap();
    let (port, events) = refuse_outcome(&podman, work.path());
    assert_eq!(port, "err");
    let ev = image_missing(&events);
    assert_eq!(
        ev.fields
            .get("probe_stderr")
            .and_then(serde_json::Value::as_str),
        Some("image inspect failed with no stderr"),
        "{ev:?}"
    );
    let invoked = std::fs::read_to_string(bin.path().join("podman.log")).unwrap_or_default();
    assert!(
        !invoked.lines().any(|l| l.starts_with("run ")),
        "no container started: {invoked}"
    );
    assert!(!events.iter().any(|e| e.kind == names::CODER_SPAWNED));
}

/// Spec 228 R2 (the defect it closes: measured on lnx, runs r000000004552..4554): podman's OWN words
/// reach the journal verbatim. The fake fails exactly the way the sandboxed daemon's podman did, and the
/// event must carry that stderr — the journal may never replace the runtime's failure with an invented
/// "image not present; build it outside pii".
#[test]
fn the_refusal_quotes_podmans_stderr_verbatim() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let podman = fake_podman_without_images(
        bin.path(),
        Some(
            "Error: acquiring runtime init lock: open /run/user/1000/libpod/tmp/alive.lck: permission denied",
        ),
    );
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    std::fs::create_dir_all(work.path().parent().unwrap().join("bin")).unwrap();
    let (port, events) = refuse_outcome(&podman, work.path());
    assert_eq!(port, "err");
    let said = "Error: acquiring runtime init lock: open /run/user/1000/libpod/tmp/alive.lck: permission denied";
    let ev = image_missing(&events);
    assert_eq!(
        ev.fields
            .get("probe_stderr")
            .and_then(serde_json::Value::as_str),
        Some(said),
        "the journal carries podman's words: {ev:?}"
    );
    assert_eq!(
        ev.fields.get("reason").and_then(serde_json::Value::as_str),
        Some(said),
        "the refusal clause is podman's stderr itself: {ev:?}"
    );
    assert!(
        !events.iter().any(|e| {
            e.kind == names::CODER_IMAGE_MISSING
                && e.fields
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|r| r.contains("image not present"))
        }),
        "the invented cause is gone: {events:?}"
    );
}

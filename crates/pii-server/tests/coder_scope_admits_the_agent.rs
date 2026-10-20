//! Spec 223 R1: the run scope admits the agent binary a `coder` node names and, for an image run, podman. A node with
//! a `container_image` runs its agent through podman, so the scheduler's node scope must reach `coder.spawned`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{CapKind, Terminal};
use pii_core::{ArgvScope, Budget, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::{CoderFactory, run_programs};

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// A fake podman that knows the image `pii-coder:1` by digest, runs anything else as a no-op, and journals its argv.
fn fake_podman(dir: &Path) -> std::path::PathBuf {
    let log = dir.join("podman.log");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$1\" in\n  image) echo sha256:feedface; exit 0 ;;\n  run) touch ./ran.txt; exit 0 ;;\n  *) exit 2 ;;\nesac\n",
        log.display()
    );
    let podman = dir.join("podman");
    std::fs::write(&podman, script).unwrap();
    std::fs::set_permissions(&podman, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    podman
}

/// An image run through the real scheduler, under the run scope the daemon builds for its coder nodes.
#[test]
fn image_run_scope_admits_podman_and_the_agent() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let podman = fake_podman(bin.path());
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log.clone())
            .with_podman(podman.clone()),
    ));
    let src = format!(
        "name = \"coder-image\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/223-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"pii-coder:1\"\n",
        work.path().display()
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(run_programs(bin.path(), &podman, &BTreeMap::new()));
    let root = Ctx::root(caps, Budget::unlimited());
    let run_id = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root,
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();

    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );
    let events = log.0.lock().clone();
    let spawned = events
        .iter()
        .find(|e| e.kind == names::CODER_SPAWNED)
        .unwrap_or_else(|| {
            panic!(
                "no coder.spawned; events: {:?}",
                events.iter().map(|e| e.kind).collect::<Vec<_>>()
            )
        });
    let digest = format!("{:?}", spawned.fields.get("image_digest"));
    assert!(
        digest.contains("sha256:feedface"),
        "digest journalled: {digest}"
    );
}

//! Spec 209: a coder run never installs anything. Every podman invocation the node makes is an `image` probe or a
//! `run`; no package manager, no `cargo install`, no `podman build` appears in any argv the node produced.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::Terminal;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// Words that would mean the run installed or built something on the way to the agent.
const INSTALLERS: [&str; 7] = [
    "apt",
    "apk",
    "pip",
    "npm",
    "cargo install",
    "podman build",
    "buildah",
];

/// A fake podman that answers `image inspect` for one image and journals every invocation.
fn fake_podman(dir: &Path, image: &str) -> std::path::PathBuf {
    let log = dir.join("podman.log");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{log}'\ncase \"$1\" in\n  image) if [ \"$5\" = '{image}' ]; then echo sha256:feedface; exit 0; fi; exit 1 ;;\n  run) touch ./ran.txt; exit 0 ;;\n  *) exit 2 ;;\nesac\n",
        log = log.display(),
    );
    let podman = dir.join("podman");
    std::fs::write(&podman, script).unwrap();
    std::fs::set_permissions(&podman, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    podman
}

#[test]
fn a_coder_run_installs_nothing() {
    let image = "pii-coder:prebuilt";
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let podman = fake_podman(bin.path(), image);
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
        "name = \"coder-no-install\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/209-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"{image}\"\ncriteria = [\"test -f ran.txt\"]\n",
        work.path().display()
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh"), podman.display().to_string()];
    argv0.extend(agent_binaries(bin.path()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    let run_id = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &Ctx::root(caps, Budget::unlimited()),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );

    let invoked = std::fs::read_to_string(bin.path().join("podman.log")).unwrap();
    assert!(!invoked.is_empty(), "podman was invoked");
    for line in invoked.lines() {
        let verb = line.split_whitespace().next().unwrap_or_default();
        assert!(
            verb == "image" || verb == "run",
            "podman verb outside probe and run: {line}"
        );
        for bad in INSTALLERS {
            assert!(!line.contains(bad), "install word `{bad}` in argv: {line}");
        }
    }
    let spawned = log
        .0
        .lock()
        .iter()
        .find(|e| e.kind == names::CODER_SPAWNED)
        .cloned()
        .expect("coder.spawned journalled");
    let argv = spawned
        .fields
        .get("argv")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    for bad in INSTALLERS {
        assert!(
            !argv.contains(bad),
            "install word `{bad}` in spawned argv: {argv}"
        );
    }
}

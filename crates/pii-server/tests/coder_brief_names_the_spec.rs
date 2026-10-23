//! Spec 246: the agent's prompt names the spec it implements. `brief_with_spec` is the pure rule; the last test drives the
//! real `coder` node through the real scheduler and checks that the agent's received argv carries the spec path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{RunStatus, Terminal};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer};
use pii_server::node_kinds::nodes::coder::prompt::brief_with_spec;
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
fn appends_the_spec_path() {
    let out = brief_with_spec("Do it.", "specs/9-x.md");
    assert!(out.contains("`specs/9-x.md`"), "{out}");
}

#[test]
fn keeps_the_original_brief_first() {
    let out = brief_with_spec("Do it.", "specs/9-x.md");
    assert!(out.starts_with("Do it."), "{out}");
}

#[test]
fn names_the_spec_once_when_the_brief_already_does() {
    assert_eq!(
        brief_with_spec("Do specs/9-x.md now.", "specs/9-x.md"),
        "Do specs/9-x.md now."
    );
}

#[test]
fn an_empty_spec_path_changes_nothing() {
    assert_eq!(brief_with_spec("Do it.", "  "), "Do it.");
}

#[test]
fn a_brief_naming_another_spec_still_gets_this_one() {
    let out = brief_with_spec("See specs/8-y.md.", "specs/9-x.md");
    assert!(out.contains("`specs/9-x.md`"), "{out}");
}

/// Runs `src` through the real scheduler with `reg`, then returns the stored run status and the terminal.
fn run_graph(
    reg: &NodeRegistry,
    src: &str,
    bin: &Path,
) -> (RunStatus, Terminal, pii_store::Store, tempfile::TempDir) {
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let spec = pii_graph::load_toml_with_registry(src, reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&BTreeMap::from([(
        "green".to_owned(),
        vec!["/bin/true".to_owned()],
    )])));
    caps.proc_argv0 = ArgvScope::new(argv0);
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        reg,
        &Ctx::root(caps, Budget::unlimited()),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let header = store.get_run(&run_id).unwrap().expect("the run is stored");
    (header.status, terminal, store, store_dir)
}

/// The brief the agent receives is the brief plus the spec path: the fake agent records its argv in the workdir.
#[test]
fn the_node_hands_both_agents_the_extended_brief() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    // The fake agent records every argument it was given and changes the tree (spec 239: a no-change run is refused).
    std::fs::write(
        bin.path().join("pi"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > argv.txt\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(
        bin.path().join("pi"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log).with_criteria(
            BTreeMap::from([("green".to_owned(), vec!["/bin/true".to_owned()])]),
        ),
    ));
    let src = format!(
        "name = \"names-spec\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\ntimeout_ms = 60000\nbrief = \"Implement it.\"\nspec = \"specs/9-x.md\"\nworkdir = \"{}\"\ncriteria = [\"green\"]\n",
        work.path().display()
    );

    let (status, terminal, _store, _dir) = run_graph(&reg, &src, bin.path());

    assert!(matches!(terminal, Terminal::Success { .. }), "{terminal:?}");
    assert_eq!(status, RunStatus::Succeeded);
    let argv = std::fs::read_to_string(work.path().join("argv.txt")).unwrap();
    assert!(
        argv.contains("specs/9-x.md"),
        "the agent names the spec: {argv}"
    );
    assert!(
        argv.contains("Implement it."),
        "the original brief survives: {argv}"
    );
}

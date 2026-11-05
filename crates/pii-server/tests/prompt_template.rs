//! Spec 386: a `coder` node may carry a `prompt` template whose `{{path}}` placeholders are bound to the run's values. The
//! fake agent records the argv it was given, so the test reads what the model was actually told.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{RunStatus, Terminal};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// Runs `src` through the real scheduler with `input` as the run input, then returns the stored run status and the terminal.
fn run_graph(
    reg: &NodeRegistry,
    src: &str,
    bin: &Path,
    input: serde_json::Value,
) -> (RunStatus, Terminal, pii_store::Store, tempfile::TempDir) {
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "prompt-store-").unwrap();
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
        input,
        true,
    )
    .unwrap();
    let header = store.get_run(&run_id).unwrap().expect("the run is stored");
    (header.status, terminal, store, store_dir)
}

/// A fake `pi` agent that records its argv in the workdir and changes the tree (spec 239: a no-change run is refused).
fn fake_agent() -> tempfile::TempDir {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "prompt-bin-").unwrap();
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
    bin
}

/// A registry whose coder kind records its events in `log`.
fn registry(bin: &Path, log: Arc<Rec>) -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.to_path_buf(), BTreeMap::new(), log).with_criteria(
            BTreeMap::from([("green".to_owned(), vec!["/bin/true".to_owned()])]),
        ),
    ));
    reg
}

/// A one-node graph whose coder carries `prompt` and runs in `work`.
fn graph(prompt: &str, work: &Path) -> String {
    format!(
        "name = \"prompt-t\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\ntimeout_ms = 60000\nprompt = \"{prompt}\"\nspec = \"specs/9-x.md\"\nworkdir = \"{}\"\ncriteria = [\"green\"]\n",
        work.display()
    )
}

#[test]
fn a_placeholder_resolves() {
    let bin = fake_agent();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "prompt-work-").unwrap();
    let reg = registry(bin.path(), Arc::new(Rec::default()));
    let src = graph("x {{input.n}}", work.path());

    let (status, terminal, _store, _dir) =
        run_graph(&reg, &src, bin.path(), serde_json::json!({ "n": 3 }));

    assert!(matches!(terminal, Terminal::Success { .. }), "{terminal:?}");
    assert_eq!(status, RunStatus::Succeeded);
    let argv = std::fs::read_to_string(work.path().join("argv.txt")).unwrap();
    assert!(
        argv.contains("x 3"),
        "the model is told the resolved prompt: {argv}"
    );
}

#[test]
fn the_resolved_prompt_is_journalled() {
    let bin = fake_agent();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "prompt-work-").unwrap();
    let log = Arc::new(Rec::default());
    let reg = registry(bin.path(), Arc::clone(&log));
    let src = graph("x {{input.n}}", work.path());

    let _ = run_graph(&reg, &src, bin.path(), serde_json::json!({ "n": 3 }));

    let json = log
        .0
        .lock()
        .iter()
        .find(|e| e.kind == names::CODER_PROMPT)
        .map(|e| serde_json::to_string(e).unwrap())
        .expect("the prompt is journalled");
    assert!(
        json.contains("x 3"),
        "the journal carries the resolved text: {json}"
    );
}

#[test]
fn an_absent_placeholder_is_a_refusal_not_a_blank() {
    let bin = fake_agent();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "prompt-work-").unwrap();
    let reg = registry(bin.path(), Arc::new(Rec::default()));
    let src = graph("x {{input.missing}}", work.path());

    let _ = run_graph(&reg, &src, bin.path(), serde_json::json!({}));

    assert!(
        !work.path().join("argv.txt").exists(),
        "no agent is spawned for an unresolved prompt"
    );
}

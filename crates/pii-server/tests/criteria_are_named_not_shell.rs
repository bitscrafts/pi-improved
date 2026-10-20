//! Spec 215: a `coder` node's criteria are NAMED checks resolved to a fixed command table, never shell.
//! An unknown name is a refusal (`criteria.unknown`) before any agent is spawned; a shell string is not run.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{NodeInput, Outcome, Terminal};
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

/// A scratch bin directory holding a fake `pi` whose body is `body`.
fn fake_bin(body: &str) -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "named-bin-").unwrap();
    let pi = dir.path().join("pi");
    std::fs::write(&pi, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    dir
}

fn root_ctx(bin: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&table()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// The one test table: `green` exits zero, `red` exits non-zero. Nothing here is a shell.
fn table() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        ("green".to_owned(), vec!["/bin/true".to_owned()]),
        ("red".to_owned(), vec!["/bin/false".to_owned()]),
    ])
}

fn run_coder_graph(bin: &Path, node_fields: &str) -> (Terminal, Vec<Event>) {
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "named-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.to_path_buf(), BTreeMap::new(), log.clone())
            .with_criteria(table()),
    ));
    let src = format!(
        "name = \"named-test\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\n{node_fields}\n"
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root_ctx(bin),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let events = log.0.lock().clone();
    (terminal, events)
}

fn fields(workdir: &Path, criteria: &str) -> String {
    format!(
        "agent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/215-x.md\"\nworkdir = \"{}\"\ncriteria = [{criteria}]\n",
        workdir.display()
    )
}

/// A named criterion from the table runs, and a green one passes the node.
#[test]
fn named_criterion_from_the_table_passes() {
    let bin = fake_bin("exit 0");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "named-work-").unwrap();
    let (terminal, _) = run_coder_graph(bin.path(), &fields(work.path(), "\"green\""));
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );
}

/// A named criterion that fails still fails the node, and the failure names it.
#[test]
fn named_criterion_that_fails_names_itself() {
    let bin = fake_bin("exit 0");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "named-work-").unwrap();
    let (terminal, _) = run_coder_graph(bin.path(), &fields(work.path(), "\"red\""));
    match terminal {
        Terminal::Failure { error } => assert!(error.contains("red"), "{error}"),
        other => panic!("expected failure naming the criterion, got {other:?}"),
    }
}

/// A shell string is an unknown name: refused before the agent spawns, journalled, and never executed.
#[test]
fn shell_string_is_refused_and_never_executed() {
    let bin = fake_bin("touch \"$PWD/spawned.txt\"");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "named-work-").unwrap();
    let shell = "touch pwned.txt; /bin/true";
    let (terminal, events) =
        run_coder_graph(bin.path(), &fields(work.path(), &format!("\"{shell}\"")));

    let Terminal::Failure { error } = &terminal else {
        panic!("the refusal fails the node and the run (spec 224), got {terminal:?}");
    };
    assert!(error.contains("criteria.unknown"), "{error}");
    assert!(
        !work.path().join("pwned.txt").exists(),
        "the shell string was executed"
    );
    assert!(
        !work.path().join("spawned.txt").exists(),
        "the agent spawned before the refusal"
    );
    let refused = events
        .iter()
        .find(|e| e.kind == names::CRITERIA_UNKNOWN)
        .expect("criteria.unknown journalled");
    assert_eq!(
        refused
            .fields
            .get("criterion")
            .and_then(serde_json::Value::as_str),
        Some(shell)
    );
    assert!(!events.iter().any(|e| e.kind == names::CODER_SPAWNED));
}

/// The refusal fails the node (spec 224), not a value on the `err` port and not a scheduler retry.
#[test]
fn unknown_criterion_fails_the_node() {
    let bin = fake_bin("exit 0");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "named-work-").unwrap();
    let log = Arc::new(Rec::default());
    let factory = CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log)
        .with_criteria(table());
    let node = <CoderFactory as pii_graph::node::NodeFactory>::build(
        &factory,
        &serde_json::json!({
            "agent": "pi", "model": "m", "brief": "b", "spec": "specs/215-x.md",
            "workdir": work.path().display().to_string(), "criteria": ["touch pwned.txt"]
        }),
    )
    .unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    let ctx = root_ctx(bin.path());
    match node.run(&ctx, &input).unwrap() {
        Outcome::Fail { error } => assert!(error.contains("criteria.unknown"), "{error}"),
        other => panic!("expected the node to fail, got {other:?}"),
    }
    assert!(!work.path().join("pwned.txt").exists());
}

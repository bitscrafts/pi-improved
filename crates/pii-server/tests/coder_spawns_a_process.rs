//! Spec 207: a `coder` node spawns its agent directly; the criteria, not the agent's exit, decide the verdict;
//! missing credentials are a refusal on the `err` port, never an error the scheduler would retry.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{NodeInput, Outcome, Terminal};
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

/// A scratch bin directory holding a fake `pi` whose body is `body` (a POSIX shell snippet).
fn fake_bin(body: &str) -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let pi = dir.path().join("pi");
    std::fs::write(&pi, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    dir
}

/// A root context with SPAWN + PROC and the run scope's programs: `/bin/sh` and the agent binaries under `bin`.
fn root_ctx(bin: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// Runs a one-node `coder` graph through the real scheduler and returns the terminal and the events.
fn run_coder_graph(
    bin: &Path,
    node_fields: &str,
    env: BTreeMap<String, String>,
) -> (Terminal, Vec<Event>, PathBuf) {
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(CoderFactory::with_env(
        bin.to_path_buf(),
        env,
        log.clone(),
    )));
    let src = format!(
        "name = \"coder-test\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\n{node_fields}\n"
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
    (terminal, events, store_dir.path().to_path_buf())
}

/// Spawns the fake `pi` through the node: the child runs, the criterion passes, the node succeeds, and the
/// journal-visible `coder.spawned` event names the pid and an argv that begins with the agent binary.
#[test]
fn coder_spawns_pi_and_passes_on_green_criteria() {
    let bin = fake_bin("echo ran > \"$PWD/ran.txt\"");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let fields = format!(
        "agent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/207-x.md\"\nworkdir = \"{}\"\ncriteria = [\"test -f ran.txt\"]\n",
        work.path().display()
    );
    let (terminal, events, _) = run_coder_graph(bin.path(), &fields, BTreeMap::new());

    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );
    assert!(
        work.path().join("ran.txt").exists(),
        "the agent process ran"
    );
    let spawned = events
        .iter()
        .find(|e| e.kind == names::CODER_SPAWNED)
        .expect("coder.spawned journalled");
    let argv = spawned
        .fields
        .get("argv")
        .and_then(serde_json::Value::as_str)
        .expect("argv field");
    assert!(
        argv.starts_with(&bin.path().join("pi").display().to_string()),
        "argv begins with the agent binary: {argv}"
    );
    assert!(spawned.fields.get("pid").is_some(), "pid recorded");
}

/// Criteria decide: the agent exits zero, but a failing criterion makes the node FAILED, and the failure
/// names that criterion.
#[test]
fn failing_criterion_fails_the_node_and_names_it() {
    let bin = fake_bin("exit 0");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let fields = format!(
        "agent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/207-x.md\"\nworkdir = \"{}\"\ncriteria = [\"false-criterion-xyz\"]\n",
        work.path().display()
    );
    let (terminal, _, _) = run_coder_graph(bin.path(), &fields, BTreeMap::new());
    match terminal {
        Terminal::Failure { error } => assert!(error.contains("false-criterion-xyz"), "{error}"),
        other => panic!("expected failure naming the criterion, got {other:?}"),
    }
}

/// A `claude` node with no credentials in its `env_pass` is refused: a `worker.start.refused` event, routed to
/// the `err` port, with no child spawned.
#[test]
fn missing_credentials_is_a_refusal_on_the_err_port() {
    let bin = fake_bin("touch \"$PWD/spawned.txt\"");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let fields = format!(
        "agent = \"claude\"\nmodel = \"haiku-5-5\"\nbrief = \"b\"\nspec = \"specs/207-x.md\"\nworkdir = \"{}\"\ncriteria = [\"true\"]\n",
        work.path().display()
    );
    let (terminal, events, _) = run_coder_graph(bin.path(), &fields, BTreeMap::new());
    assert!(
        !matches!(terminal, Terminal::Suspended { .. }),
        "a refusal must not suspend"
    );
    let refused = events
        .iter()
        .find(|e| e.kind == names::WORKER_START_REFUSED)
        .expect("worker.start.refused journalled");
    assert_eq!(
        refused
            .fields
            .get("model")
            .and_then(serde_json::Value::as_str),
        Some("haiku-5-5")
    );
    assert!(
        !work.path().join("spawned.txt").exists(),
        "no pid, no spawn"
    );
    assert!(!events.iter().any(|e| e.kind == names::CODER_SPAWNED));
}

/// The node's own outcome for a refusal is the `err` port (not an `Err`): the factory built it, and its run
/// returns `Next{err}` directly, so the scheduler never retries it.
#[test]
fn refusal_outcome_is_the_err_port_not_an_error() {
    let log = Arc::new(Rec::default());
    let factory = CoderFactory::with_env(PathBuf::from("/nonexistent"), BTreeMap::new(), log);
    let node = pii_graph::node::NodeFactory::build(
        &factory,
        &serde_json::json!({
            "agent": "claude", "model": "m", "brief": "b", "spec": "s", "workdir": "/tmp",
        }),
    )
    .unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    match node
        .run(&root_ctx(Path::new("/nonexistent")), &input)
        .unwrap()
    {
        Outcome::Next { port, .. } => assert_eq!(port.as_str(), "err"),
        other => panic!("expected err port, got {other:?}"),
    }
}

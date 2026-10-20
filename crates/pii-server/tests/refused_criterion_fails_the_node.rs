//! Spec 224 R1: a criterion the table does not name fails its `coder` node, and the run's terminal error is that
//! refusal verbatim. It is never a success that a later check (`coder.no_call_events`) rewrites.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use pii_abi::{NodeInput, Outcome, Terminal};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;

#[derive(Default)]
struct Rec(parking_lot::Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// A scratch bin directory holding a fake `pi` whose body is `body`.
fn fake_bin(body: &str) -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "refused-bin-").unwrap();
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

/// The one test table: `green` exits zero. `cargo test` is deliberately absent.
fn table() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([("green".to_owned(), vec!["/bin/true".to_owned()])])
}

fn run_coder_graph(bin: &Path, node_fields: &str) -> Terminal {
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "refused-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.to_path_buf(), BTreeMap::new(), log).with_criteria(table()),
    ));
    let src = format!(
        "name = \"refused-test\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\n{node_fields}\n"
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root_ctx(bin),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap()
}

fn fields(workdir: &Path, criteria: &str) -> String {
    format!(
        "agent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/224-x.md\"\nworkdir = \"{}\"\ncriteria = [{criteria}]\n",
        workdir.display()
    )
}

/// The node itself returns `Fail` with the refusal, not a value on the `err` port.
#[test]
fn unknown_criterion_is_a_node_failure() {
    let bin = fake_bin("exit 0");
    let work =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "refused-work-").unwrap();
    let factory = CoderFactory::with_env(
        bin.path().to_path_buf(),
        BTreeMap::new(),
        Arc::new(Rec::default()),
    )
    .with_criteria(table());
    let node = <CoderFactory as pii_graph::node::NodeFactory>::build(
        &factory,
        &serde_json::json!({
            "agent": "pi", "model": "m", "brief": "b", "spec": "specs/224-x.md",
            "workdir": work.path().display().to_string(), "criteria": ["cargo test"]
        }),
    )
    .unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    match node.run(&root_ctx(bin.path()), &input).unwrap() {
        Outcome::Fail { error } => {
            assert!(error.contains("criteria.unknown"), "{error}");
            assert!(error.contains("cargo test"), "{error}");
        }
        other => panic!("expected the node to fail, got {other:?}"),
    }
}

/// The run's terminal error is the refusal itself, and never the run-level `coder.no_call_events`.
#[test]
fn refused_criterion_fails_the_run_with_its_own_reason() {
    let bin = fake_bin("exit 0");
    let work =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "refused-work-").unwrap();
    let terminal = run_coder_graph(bin.path(), &fields(work.path(), "\"cargo test\""));
    match terminal {
        Terminal::Failure { error } => {
            assert!(error.contains("criteria.unknown"), "{error}");
            assert!(!error.contains("coder.no_call_events"), "{error}");
        }
        other => panic!("expected the refusal as the failure, got {other:?}"),
    }
}

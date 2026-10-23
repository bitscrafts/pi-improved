//! Spec 239: a `coder` node whose agent changed nothing fails only when its criteria already passed before the agent
//! ran. A criterion that failed before and passes now means something changed, so a no-change run is not refused.
//! Controls: every other combination keeps today's verdict, and the node that writes what its criterion needs passes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::Terminal;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::CoderFactory;
use pii_server::node_kinds::nodes::coder::agent_binaries;
use pii_server::node_kinds::nodes::coder::no_change::no_change_refusal;
use pii_server::node_kinds::nodes::criteria;

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
    argv0.extend(criteria::programs(&criteria_table()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// Spec 215: the named criteria these tests use; each name is a fixed argv, never a shell string.
fn criteria_table() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        (
            "ran-file".to_owned(),
            vec![
                "/usr/bin/test".to_owned(),
                "-f".to_owned(),
                "ran.txt".to_owned(),
            ],
        ),
        ("green".to_owned(), vec!["/bin/true".to_owned()]),
    ])
}

/// Runs a one-node `coder` graph through the real scheduler and returns the terminal and the events.
fn run_coder_graph(
    bin: &Path,
    node_fields: &str,
    env: BTreeMap<String, String>,
) -> (Terminal, Vec<Event>, tempfile::TempDir) {
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.to_path_buf(), env, log.clone()).with_criteria(criteria_table()),
    ));
    let src = format!(
        "name = \"coder-test\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ntimeout_ms = 60000\ncaps = [\"spawn\", \"proc\"]\n{node_fields}\n"
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
    (terminal, events, store_dir)
}

#[test]
fn no_change_with_criteria_already_passing_is_refused() {
    let reason = no_change_refusal(Some(true), "no", true);
    assert!(reason.expect("refused").contains("no change"));
}

#[test]
fn a_changed_tree_is_not_refused() {
    assert_eq!(no_change_refusal(Some(true), "yes", true), None);
}

#[test]
fn criteria_that_failed_before_the_agent_are_not_refused() {
    assert_eq!(no_change_refusal(Some(false), "no", true), None);
}

#[test]
fn an_unknown_tree_is_not_refused() {
    assert_eq!(no_change_refusal(Some(true), "unknown", true), None);
}

#[test]
fn no_pre_run_means_no_refusal() {
    assert_eq!(no_change_refusal(None, "no", true), None);
}

#[test]
fn a_failed_verdict_is_left_alone() {
    assert_eq!(no_change_refusal(Some(true), "no", false), None);
}

/// The agent exits zero and writes nothing, and the criterion already passes: the node must FAIL, not certify work
/// that never happened.
#[test]
fn the_node_fails_when_the_agent_writes_nothing_and_the_criterion_already_passes() {
    let bin = fake_bin("exit 0");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let fields = format!(
        "agent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/239-x.md\"\nworkdir = \"{}\"\ncriteria = [\"green\"]\n",
        work.path().display()
    );
    let (terminal, _, _) = run_coder_graph(bin.path(), &fields, BTreeMap::new());
    match terminal {
        Terminal::Failure { error } => assert!(error.contains("no change"), "{error}"),
        other => panic!("expected failure naming the no-change refusal, got {other:?}"),
    }
}

/// The agent writes the file its criterion tests for: the criterion fails before and passes after, so the node
/// succeeds. The event says the criteria did not pass before the agent ran.
#[test]
fn the_node_passes_when_the_agent_writes_what_the_criterion_needs() {
    let bin = fake_bin("echo ran > \"$PWD/ran.txt\"");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let fields = format!(
        "agent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/239-x.md\"\nworkdir = \"{}\"\ncriteria = [\"ran-file\"]\n",
        work.path().display()
    );
    let (terminal, events, _) = run_coder_graph(bin.path(), &fields, BTreeMap::new());
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );
    let exited = events
        .iter()
        .find(|e| e.kind == names::CODER_EXITED)
        .expect("coder.exited journalled");
    assert_eq!(
        exited.fields.get("pre_criteria_passed"),
        Some(&serde_json::json!("no")),
        "the criterion failed before the agent ran"
    );
}

//! Spec 364: the queue implements a row by CALLING `graphs/spec.toml`, and `record` writes the callee's verdict.
//!
//! The tracked `graphs/queue.toml` runs through the loader and the scheduler with a fake `pi`; the judges are the real
//! spec-mode judge on a throwaway repo. Each row's own criterion is one that can never pass, so a queue that still ran
//! the row's criteria would park the row the callee passed.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test builds the git repo the judge reads its spec from; no production path runs through it
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use pii_abi::{RunEvent, Terminal};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx, NullObserver, TraceCtx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;
use pii_server::node_kinds::nodes::queue::{JudgeFactory, RecordFactory, SelectFactory};
use pii_store::queue::{ItemStatus, QueueStore, WorkItem};

const SPEC: &str = "specs/900-queued.md";
/// The row's own criterion: it can never pass, and the queue must never run it.
const ROW_CRITERION: &str = "row-criterion-the-queue-must-not-run";
/// The fake agent's work: the spec's criterion passes once it has run.
const WORKS: &str = "touch built.txt";
const CRASHES: &str = "exit 1";

struct Silent;

impl Observer for Silent {
    fn event(&self, _e: &Event) {}
}

fn graphs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A repo whose one commit holds `SPEC` with the one criterion `criterion`. Returns the repo and the commit id.
fn repo(criterion: &str) -> (tempfile::TempDir, String) {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "qcs-repo-").unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@example.invalid"]);
    git(root, &["config", "user.name", "t"]);
    std::fs::create_dir_all(root.join("specs")).unwrap();
    let blob = format!("# Spec 900\n\n## Exit Criteria\n\n- [ ] `{criterion}`\n");
    std::fs::write(root.join(SPEC), blob).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "spec"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    (dir, base)
}

/// A scratch bin directory whose fake `pi` runs `work`.
fn fake_bin(work: &str) -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "qcs-bin-").unwrap();
    let pi = dir.path().join("pi");
    std::fs::write(&pi, format!("#!/bin/sh\n{work}\n")).unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    dir
}

fn root_ctx(bin: &Path, run: &pii_abi::RunId) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC | CapKind::FS_READ | CapKind::FS_WRITE;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&BTreeMap::new()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    let trace = TraceCtx {
        run: run.clone(),
        span: 0,
    };
    Ctx::with_observer(caps, Budget::unlimited(), Arc::new(NullObserver), trace)
}

/// What one tick of the tracked queue graph left behind.
struct Ran {
    terminal: Terminal,
    journal: Vec<(u64, RunEvent)>,
    item: WorkItem,
}

impl Ran {
    /// Every node the run started, in order.
    fn nodes(&self) -> Vec<String> {
        self.journal
            .iter()
            .filter_map(|(_, ev)| match ev {
                RunEvent::NodeStarted { node, .. } => Some(node.to_string()),
                _ => None,
            })
            .collect()
    }

    fn started(&self, node: &str) -> bool {
        self.nodes().iter().any(|n| n == node)
    }

    fn called(&self, graph: &str) -> bool {
        self.journal.iter().any(|(_, ev)| match ev {
            RunEvent::NodeObserved { kind, fields, .. } => {
                kind == "graph.call" && fields["graph"] == graph
            }
            _ => false,
        })
    }
}

/// One tick over one row, whose spec's criterion is `criterion`, with the fake agent doing `work`.
fn tick(criterion: &str, work: &str) -> Ran {
    tick_row(criterion, work, false)
}

/// One tick over one row. When `held`, another run already holds the row: `select` reports it as running.
fn tick_row(criterion: &str, work: &str, held: bool) -> Ran {
    let (repo_dir, base) = repo(criterion);
    let bin = fake_bin(work);
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "qcs-store-").unwrap();
    let queue = Arc::new(QueueStore::open(&dir.path().join("queue.redb")).unwrap());
    let mut row = WorkItem::new("A1", &[]);
    SPEC.clone_into(&mut row.spec);
    row.criteria = vec![ROW_CRITERION.to_owned()];
    queue.put_item(&row).unwrap();
    if held {
        queue
            .set_status("A1", ItemStatus::Running, None, "other-run")
            .unwrap();
    }

    let mut reg = NodeRegistry::builtin();
    let env = BTreeMap::from([(String::from("OLLAMA_API_KEY"), String::from("placeholder"))]);
    let log: Arc<dyn Observer> = Arc::new(Silent);
    reg.register(Arc::new(SelectFactory {
        store: Arc::clone(&queue),
    }));
    reg.register(Arc::new(CoderFactory::with_env(
        bin.path().to_path_buf(),
        env,
        log,
    )));
    reg.register(Arc::new(JudgeFactory));
    reg.register(Arc::new(RecordFactory {
        store: Arc::clone(&queue),
        memory: None,
    }));
    let src = std::fs::read_to_string(graphs_dir().join("queue.toml")).unwrap();
    let frags = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "qcs-frags-").unwrap();
    let woven =
        pii_graph::fragment::load_host_with_graphs(&src, frags.path(), &[graphs_dir()], &reg)
            .unwrap();
    let store = pii_store::Store::open(&dir.path().join("store.redb")).unwrap();
    let run_id = store.next_run_id().unwrap();
    let input = serde_json::json!({
        "workdir": repo_dir.path().display().to_string(),
        "base": base,
    });
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &woven.spec,
        &reg,
        &root_ctx(bin.path(), &run_id),
        &run_id,
        input,
        true,
    )
    .unwrap();
    Ran {
        terminal,
        journal: store.read_journal(&run_id).unwrap(),
        item: queue.get_item("A1").unwrap().unwrap(),
    }
}

#[test]
fn a_row_is_implemented_by_the_spec_graph() {
    let r = tick("test -f built.txt", WORKS);
    assert!(
        r.called("spec"),
        "the trace shows the call: {:?}",
        r.nodes()
    );
    let nodes = r.nodes();
    let at = |n: &str| {
        nodes
            .iter()
            .position(|x| x == n)
            .unwrap_or_else(|| panic!("{n} never started: {nodes:?}"))
    };
    assert!(
        at("select") < at("implement__code")
            && at("implement__code") < at("implement__verify")
            && at("implement__verify") < at("implement__docs__docs")
            && at("implement__docs__docs") < at("implement__reverify")
            && at("implement__reverify") < at("implement__done")
            && at("implement__done") < at("record"),
        "the callee's whole path runs, and record follows its done: {nodes:?}"
    );
    let Terminal::Success { output } = &r.terminal else {
        panic!("a passing row succeeds: {:?}", r.terminal);
    };
    assert_eq!(output["status"], "done", "{output}");
    assert_eq!(r.item.status, ItemStatus::Done, "{:?}", r.item);
}

#[test]
fn CONTROL_the_queue_runs_no_judge_of_its_own() {
    let r = tick("test -f built.txt", WORKS);
    let nodes = r.nodes();
    // Only the host's own nodes and the call's namespace exist: anything else is a second implementation.
    let own: Vec<&String> = nodes
        .iter()
        .filter(|n| !n.starts_with("implement__"))
        .collect();
    assert_eq!(
        own,
        ["select", "record"],
        "the queue's own nodes: {nodes:?}"
    );
    let judges: Vec<&String> = nodes.iter().filter(|n| n.contains("verify")).collect();
    assert_eq!(
        judges,
        ["implement__verify", "implement__reverify"],
        "the only judges are the callee's: {nodes:?}"
    );
    // The row's criterion cannot pass, so a Done row proves it was not run and that the callee's verdict decided.
    assert_eq!(r.item.status, ItemStatus::Done, "{:?}", r.item);
    assert!(r.item.reason.is_none(), "{:?}", r.item);
}

#[test]
fn a_refusing_callee_fails_the_row() {
    // The judges fail: the spec's criterion is never met, whatever the agent does.
    let r = tick("test -f never-built.txt", WORKS);
    assert!(r.started("implement__unverified"), "{:?}", r.nodes());
    assert!(!r.started("implement__done"), "{:?}", r.nodes());
    assert!(r.started("record"), "record ran: {:?}", r.nodes());
    assert_eq!(r.item.status, ItemStatus::Parked, "{:?}", r.item);
    let reason = r.item.reason.clone().unwrap_or_default();
    assert!(reason.contains("unverified"), "names the port: {reason}");
    assert!(
        reason.contains("never-built.txt"),
        "names the criterion: {reason}"
    );
    assert!(
        !reason.contains(ROW_CRITERION),
        "the row's own criterion was not run: {reason}"
    );
    let Terminal::Success { output } = &r.terminal else {
        panic!(
            "the verdict is recorded, the run completes: {:?}",
            r.terminal
        );
    };
    assert_eq!(output["status"], "parked", "{output}");

    // The agent fails: the other refusal port, and the judges never run.
    let r = tick("test -f built.txt", CRASHES);
    assert!(r.started("implement__refused"), "{:?}", r.nodes());
    assert!(!r.started("implement__verify"), "{:?}", r.nodes());
    assert_eq!(r.item.status, ItemStatus::Parked, "{:?}", r.item);
    let reason = r.item.reason.unwrap_or_default();
    assert!(reason.contains("refused"), "names the port: {reason}");
}

#[test]
fn a_row_held_by_another_run_is_recorded_as_running() {
    // Spec 381 R2: `select.running` is a verdict. The run ends on `record`, which says so and touches nothing: the held
    // row belongs to the other run, and nothing is implemented here.
    let r = tick_row("test -f built.txt", WORKS, true);
    assert!(r.started("record"), "record ran: {:?}", r.nodes());
    assert!(!r.started("implement"), "{:?}", r.nodes());
    let Terminal::Success { output } = &r.terminal else {
        panic!("a held row is a verdict, not a failure: {:?}", r.terminal);
    };
    assert_eq!(output["status"], "running", "{output}");
    assert_eq!(r.item.status, ItemStatus::Running, "{:?}", r.item);
}

//! Spec 248: a run's session file belongs to the run. Two runs on one workdir journal two different session
//! files; an explicit `session =` stays a deliberate pin shared by every run that names it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx, NullObserver, TraceCtx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::agent_worker::{WorkerSpec, pi_session_path};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
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

/// A context for run `run` with SPAWN + PROC and the run scope's programs: `/bin/sh` and the agent binaries under `bin`.
fn root_ctx(bin: &Path, run: &pii_abi::RunId) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&criteria_table()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    let trace = TraceCtx {
        run: run.clone(),
        span: 0,
    };
    Ctx::with_observer(caps, Budget::unlimited(), Arc::new(NullObserver), trace)
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
        ("red".to_owned(), vec!["/bin/false".to_owned()]),
    ])
}

/// A stand-in pi that appends its pid to the session file it is handed.
const WRITER: &str = "while [ $# -gt 0 ]; do case \"$1\" in --session) S=\"$2\"; shift;; esac; shift; done; echo \"$$\" >> \"$S\"";

/// Runs the same one-node coder graph once per entry of `n` against ONE store and ONE workdir; returns the
/// `coder.spawned.session` of each run.
fn run_twice(work: &Path, extra: &str, runs: usize) -> Vec<String> {
    let bin = fake_bin(WRITER);
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let mut sessions = Vec::new();
    for _ in 0..runs {
        let log = Arc::new(Rec::default());
        let mut reg = NodeRegistry::builtin();
        reg.register(Arc::new(
            CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log.clone())
                .with_criteria(criteria_table()),
        ));
        let src = format!(
            "name = \"coder-test\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ntimeout_ms = 60000\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/248-x.md\"\nworkdir = \"{}\"\ncriteria = [\"green\"]\n{extra}\n",
            work.display()
        );
        let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
        let run_id = store.next_run_id().unwrap();
        // The verdict is not under test (the stand-in changes only `.pii-sessions`, which the digest skips).
        pii_graph::run::run_with_validate(
            &store,
            &spec,
            &reg,
            &root_ctx(bin.path(), &run_id),
            &run_id,
            serde_json::json!({}),
            true,
        )
        .unwrap();
        let events = log.0.lock().clone();
        let spawned = events
            .iter()
            .find(|e| e.kind == names::CODER_SPAWNED)
            .expect("coder.spawned journalled");
        let session = spawned
            .fields
            .get("session")
            .and_then(serde_json::Value::as_str)
            .expect("session field")
            .to_owned();
        sessions.push(session);
    }
    sessions
}

fn work_dir() -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap()
}

#[test]
fn two_runs_on_one_workdir_get_two_session_files() {
    let work = work_dir();
    let s = run_twice(work.path(), "", 2);
    assert_ne!(s[0], s[1], "two runs, two session paths");
    for p in &s {
        assert!(Path::new(p).exists(), "{p} exists");
        let text = std::fs::read_to_string(p).unwrap();
        assert!(text.starts_with("{\"type\":\"session\""), "pi header first");
        assert_eq!(
            text.lines().count(),
            2,
            "header plus exactly one pid line: {text}"
        );
    }
}

#[test]
fn the_journalled_session_is_the_file_on_disk() {
    let work = work_dir();
    let s = run_twice(work.path(), "", 1);
    let text = std::fs::read_to_string(&s[0]).unwrap();
    assert!(text.starts_with("{\"type\":\"session\""), "{text}");
}

#[test]
fn an_explicit_session_pins_the_path() {
    let work = work_dir();
    let pinned = work.path().join("pinned.jsonl");
    let extra = format!("session = \"{}\"", pinned.display());
    let s = run_twice(work.path(), &extra, 2);
    assert_eq!(s[0], s[1]);
    assert_eq!(s[0], pinned.display().to_string());
    let text = std::fs::read_to_string(&pinned).unwrap();
    assert_eq!(text.lines().count(), 3, "header plus BOTH pids: {text}");
}

fn cfg(run: Option<&str>) -> serde_json::Value {
    let mut v = serde_json::json!({
        "agent": "pi", "model": "m", "spec": "specs/248-x.md", "workdir": "/projects/x",
    });
    if let Some(r) = run {
        v["run"] = serde_json::json!(r);
    }
    v
}

#[test]
fn the_default_session_path_carries_the_run() {
    let spec = WorkerSpec::from_cfg(&cfg(Some("r000000000042"))).unwrap();
    assert_eq!(
        pi_session_path(&spec),
        "/projects/x/.pii-sessions/248-x-r000000000042.jsonl"
    );
    let spec = WorkerSpec::from_cfg(&cfg(None)).unwrap();
    assert_eq!(
        pi_session_path(&spec),
        "/projects/x/.pii-sessions/248-x.jsonl"
    );
}

#[test]
fn a_malformed_run_is_refused() {
    for bad in ["../../etc/x", ""] {
        let e = WorkerSpec::from_cfg(&cfg(Some(bad))).unwrap_err();
        assert!(format!("{e}").contains("run"), "names the field: {e}");
    }
}

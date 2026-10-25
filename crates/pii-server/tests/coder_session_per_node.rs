//! Spec 271: a pi coder's default session file belongs to its node as well as its run. Two coder nodes of one run
//! journal two different session files; an explicit `session =` stays a deliberate pin shared by every node that names it;
//! a loop resumes its own file; a claude coder is unaffected.

#![allow(
    non_snake_case,
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
use pii_server::node_kinds::nodes::coder::session_path::node_session_path;
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// A scratch bin directory holding a fake `pi` whose body is `body` (a POSIX shell snippet) and a fake `claude` that exits 0.
fn fake_bin(body: &str) -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "node-bin-").unwrap();
    for (name, script) in [
        ("pi", format!("#!/bin/sh\n{body}\n")),
        ("claude", String::from("#!/bin/sh\nexit 0\n")),
    ] {
        let p = dir.path().join(name);
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }
    dir
}

/// A context for run `run` with SPAWN + PROC and the run scope's programs: `/bin/sh` and the agent binaries under `bin`.
fn root_ctx(bin: &Path, run: &pii_abi::RunId) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
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

/// A stand-in pi that appends its pid to the session file it is handed.
const WRITER: &str = "while [ $# -gt 0 ]; do case \"$1\" in --session) S=\"$2\"; shift;; esac; shift; done; echo \"$$\" >> \"$S\"";

/// One node of kind `coder`: id `id`, agent `agent`, on the spec and workdir the graph names, plus `extra` config.
fn coder(id: &str, agent: &str, work: &Path, extra: &str) -> String {
    format!(
        "\n[[node]]\nid = \"{id}\"\nkind = \"coder\"\ntimeout_ms = 60000\ncaps = [\"spawn\", \"proc\"]\nagent = \"{agent}\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/271-x.md\"\nworkdir = \"{}\"\n{extra}\n",
        work.display()
    )
}

/// Runs the graph `body` (its nodes and edges, after the header) `runs` times against ONE store and ONE workdir, with
/// the entry `entry`. Returns, per run, the `session` field of each `coder.spawned` in journal order.
fn run_graph(bin: &Path, entry: &str, body: &str, runs: usize) -> Vec<Vec<String>> {
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "node-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let mut out = Vec::new();
    for _ in 0..runs {
        let log = Arc::new(Rec::default());
        let mut reg = NodeRegistry::builtin();
        reg.register(Arc::new(CoderFactory::with_env(
            bin.to_path_buf(),
            BTreeMap::new(),
            log.clone(),
        )));
        let src = format!(
            "name = \"node-test\"\nentry = \"{entry}\"\ncaps = [\"spawn\", \"proc\"]\n{body}"
        );
        let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
        let run_id = store.next_run_id().unwrap();
        // The verdict is not under test: the nodes carry no criteria, so a clean exit hands on by `ok`.
        pii_graph::run::run_with_validate(
            &store,
            &spec,
            &reg,
            &root_ctx(bin, &run_id),
            &run_id,
            serde_json::json!({}),
            true,
        )
        .unwrap();
        let events = log.0.lock().clone();
        let sessions = events
            .iter()
            .filter(|e| e.kind == names::CODER_SPAWNED)
            .map(|e| {
                e.fields
                    .get("session")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();
        out.push(sessions);
    }
    out
}

/// A session file is a pi header followed by `pids` pid lines.
fn assert_pi_file(path: &str, pids: usize) {
    assert!(Path::new(path).exists(), "{path} exists");
    let text = std::fs::read_to_string(path).unwrap();
    assert!(
        text.starts_with("{\"type\":\"session\""),
        "pi header first: {text}"
    );
    assert_eq!(
        text.lines().count(),
        1 + pids,
        "header plus {pids} pid lines: {text}"
    );
}

/// Two pi coder nodes, `code` then `docs`, one spec and one workdir; `extra` is the config both share.
fn two_nodes(work: &Path, extra_code: &str, extra_docs: &str) -> String {
    format!(
        "{}{}\n[[edge]]\nfrom = \"code\"\nport = \"ok\"\nto = \"docs\"\nkind = \"data\"\n",
        coder("code", "pi", work, extra_code),
        coder("docs", "pi", work, extra_docs),
    )
}

fn work_dir() -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "node-work-").unwrap()
}

#[test]
fn two_nodes_get_two_sessions() {
    let work = work_dir();
    let bin = fake_bin(WRITER);
    let s = run_graph(bin.path(), "code", &two_nodes(work.path(), "", ""), 1);
    let sessions = &s[0];
    assert_eq!(sessions.len(), 2, "both coder nodes spawned: {sessions:?}");
    assert_ne!(sessions[0], sessions[1], "two nodes, two session paths");
    for p in sessions {
        assert_pi_file(p, 1);
    }
}

#[test]
fn CONTROL_one_literal_session_is_shared() {
    let work = work_dir();
    let bin = fake_bin(WRITER);
    let pinned = work.path().join("pinned.jsonl");
    let extra = format!("session = \"{}\"", pinned.display());
    let s = run_graph(
        bin.path(),
        "code",
        &two_nodes(work.path(), &extra, &extra),
        1,
    );
    let sessions = &s[0];
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0], sessions[1], "the same literal path");
    assert_eq!(sessions[0], pinned.display().to_string());
    assert_pi_file(&sessions[0], 2);
}

#[test]
fn CONTROL_a_loop_resumes_its_own_file() {
    let work = work_dir();
    let bin = fake_bin(WRITER);
    // `code` re-enters through a transform back-edge, as a fix loop does; its budget bounds the re-entry.
    let body = format!(
        "{}\n[[node]]\nid = \"retry\"\nkind = \"transform\"\nexpr = \"$.payload\"\n\n[[edge]]\nfrom = \"code\"\nport = \"ok\"\nto = \"retry\"\nkind = \"data\"\n\n[[edge]]\nfrom = \"retry\"\nport = \"ok\"\nto = \"code\"\nkind = \"data\"\n",
        coder("code", "pi", work.path(), "")
            .replace("kind = \"coder\"", "kind = \"coder\"\nloop_budget = 2"),
    );
    let s = run_graph(bin.path(), "code", &body, 1);
    let sessions = &s[0];
    assert!(sessions.len() >= 2, "the loop ran twice: {sessions:?}");
    assert!(
        sessions.iter().all(|p| p == &sessions[0]),
        "the same path each time: {sessions:?}"
    );
}

#[test]
fn two_runs_get_two_sessions() {
    let work = work_dir();
    let bin = fake_bin(WRITER);
    let body = coder("code", "pi", work.path(), "");
    let s = run_graph(bin.path(), "code", &body, 2);
    assert_eq!(s[0].len(), 1);
    assert_eq!(s[1].len(), 1);
    assert_ne!(s[0][0], s[1][0], "two runs of one node, two paths");
}

#[test]
fn a_hostile_node_id_cannot_leave_the_workdir() {
    let work = work_dir();
    let bin = fake_bin(WRITER);
    let body = coder("a/../b", "pi", work.path(), "");
    let s = run_graph(bin.path(), "a/../b", &body, 1);
    let path = &s[0][0];
    let name = Path::new(path).file_name().unwrap().to_str().unwrap();
    assert!(name.ends_with("-a_.._b.jsonl"), "sanitised name: {name}");
    assert!(
        Path::new(path).starts_with(work.path().join(".pii-sessions")),
        "inside the workdir: {path}"
    );
    assert_pi_file(path, 1);
}

#[test]
fn CONTROL_claude_is_unchanged() {
    let work = work_dir();
    let bin = fake_bin(WRITER);
    let body = coder("code", "claude", work.path(), "");
    let s = run_graph(bin.path(), "code", &body, 1);
    for p in &s[0] {
        assert!(!p.contains("-code"), "a claude session names no node: {p}");
    }
    // The journalled `session` is read from the argv, and a claude argv has no `--session`, so the line above cannot fail
    // by itself. The file system can: only a pi dispatch writes a session file.
    assert!(
        !work.path().join(".pii-sessions").exists(),
        "a claude coder wrote a pi session directory"
    );
}

#[test]
fn CONTROL_with_node_session_leaves_every_other_spec_alone() {
    use pii_server::node_kinds::nodes::agent_worker::WorkerSpec;
    use pii_server::node_kinds::nodes::coder::session_path::with_node_session;
    let spec = |agent: &str, session: Option<&str>| {
        let mut cfg = serde_json::json!({
            "agent": agent, "model": "m", "spec": "specs/271-x.md", "workdir": "/projects/x",
            "run": "r000000000001", "node-id": "docs"
        });
        if let Some(s) = session {
            cfg["session"] = serde_json::json!(s);
        }
        (WorkerSpec::from_cfg(&cfg).unwrap(), cfg)
    };
    // The case it exists for: pi, no explicit session, a node id.
    let (pi, cfg) = spec("pi", None);
    assert_eq!(
        with_node_session(pi, &cfg).session.as_deref(),
        Some("/projects/x/.pii-sessions/271-x-r000000000001-docs.jsonl")
    );
    // Near misses that must take the normal path: claude, an explicit session, and no node id.
    let (claude, cfg) = spec("claude", None);
    assert_eq!(with_node_session(claude, &cfg).session, None, "claude");
    let (pinned, cfg) = spec("pi", Some("/p/pinned.jsonl"));
    assert_eq!(
        with_node_session(pinned, &cfg).session.as_deref(),
        Some("/p/pinned.jsonl"),
        "an explicit session pins"
    );
    let (anon, mut cfg) = spec("pi", None);
    cfg.as_object_mut().unwrap().remove("node-id");
    assert_eq!(with_node_session(anon, &cfg).session, None, "no node id");
}

#[test]
fn node_session_path_shapes() {
    let w = "/projects/x";
    assert_eq!(
        node_session_path(w, "specs/271-x.md", "r1", "code"),
        "/projects/x/.pii-sessions/271-x-r1-code.jsonl"
    );
    assert_eq!(
        node_session_path(w, "specs/271-x.md", "r1", ".."),
        "/projects/x/.pii-sessions/271-x-r1-node.jsonl"
    );
    assert_eq!(
        node_session_path(w, "specs/271-x.md", "r1", ""),
        "/projects/x/.pii-sessions/271-x-r1-node.jsonl"
    );
    assert_eq!(
        node_session_path(w, "specs/271-x.md", "r1", "é-x"),
        "/projects/x/.pii-sessions/271-x-r1-_-x.jsonl"
    );
}

#[test]
fn the_planned_path_follows_the_container_workdir() {
    let host = node_session_path("/projects/x", "specs/271-x.md", "r1", "code");
    let planned = node_session_path("/work", "specs/271-x.md", "r1", "code");
    assert!(host.starts_with("/projects/x/.pii-sessions/"), "{host}");
    assert!(planned.starts_with("/work/.pii-sessions/"), "{planned}");
    assert_ne!(host, planned);
}

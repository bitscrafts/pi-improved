//! Spec 272: `graphs/spec.toml` documents a verified spec through a CALLED graph, `graphs/docs.toml`, and re-verifies
//! after it. The graph runs through the library path (the loader, then the scheduler) with a fake `pi` and `claude`
//! standing for the agents, so nothing spends. The judge is the real spec-mode judge, run on a throwaway git repo.

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

use parking_lot::Mutex;
use pii_abi::{RunEvent, Terminal};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx, NullObserver, TraceCtx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;
use pii_server::node_kinds::nodes::queue::JudgeFactory;
use pii_store::Store;

const SPEC: &str = "specs/900-documented.md";
const IMPL: &str = "IMPL-BRIEF";
const DOCS: &str = "DOCS-BRIEF";
const DOCS_OK: &str = "echo docs > docs.md";
const DOCS_BREAK: &str = "rm -f built.txt; echo docs > docs.md";

/// The fake `pi`: records its argv, appends its pid to its `--session` file, then does its step. The docs step is the
/// call whose argv names `DOCS-BRIEF`; every other call (the code step) touches `built.txt`.
const PI: &str = r#"#!/bin/sh
ARGS="$*"
{ printf '%s\n' "$@"; echo '--end'; } >> "${0%/*}/calls.log"
while [ $# -gt 0 ]; do case "$1" in --session) S="$2"; shift;; esac; shift; done
echo "$$" >> "$S"
case "$ARGS" in
  *DOCS-BRIEF*) DOCS_BODY ;;
  *) touch built.txt ;;
esac
"#;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

fn graphs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

fn tracked_source() -> String {
    std::fs::read_to_string(graphs_dir().join("spec.toml")).unwrap()
}

/// A scratch bin directory with a fake `pi` whose docs step runs `docs`, and a fake `claude` that exits 0.
fn fake_bin(docs: &str) -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "docs-bin-").unwrap();
    let pi = PI.replace("DOCS_BODY", docs);
    for (name, script) in [("pi", pi), ("claude", String::from("#!/bin/sh\nexit 0\n"))] {
        let p = dir.path().join(name);
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }
    dir
}

/// A root context with SPAWN + PROC, `/bin/sh` (the judge's shell), and the agent binaries under `bin`.
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

// reason: test fixture only; it builds the repo the judge reads, and no production path runs through it
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A repo whose one commit holds `SPEC` with one criterion `criterion`. Returns the repo and the commit id.
fn repo(criterion: &str) -> (tempfile::TempDir, String) {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "docs-repo-").unwrap();
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

/// The run input: the five coder keys, the base the judge reads the spec at, and the docs brief unless `docs_brief` is
/// false.
fn input(work: &Path, base: &str, docs_brief: bool) -> serde_json::Value {
    let mut v = serde_json::json!({
        "agent": "pi",
        "model": "m-one",
        "provider": "ollama-cloud",
        "spec": SPEC,
        "workdir": work.display().to_string(),
        "brief": IMPL,
        "base": base,
        "docs_brief": DOCS,
    });
    if !docs_brief {
        v.as_object_mut().unwrap().remove("docs_brief");
    }
    v
}

/// What one run of the tracked graph left behind.
struct Ran {
    terminal: Terminal,
    journal: Vec<(u64, RunEvent)>,
    /// The `session` of each `coder.spawned`, in spawn order.
    sessions: Vec<String>,
    /// The text of each session file, read before the workdir is dropped, in spawn order.
    session_texts: Vec<String>,
    /// The argv of each agent call, one string per call, in call order.
    calls: Vec<String>,
}

impl Ran {
    /// The nodes of the journal that this spec's graph names, in the order they started.
    fn order(&self) -> Vec<String> {
        const KEEP: [&str; 6] = [
            "code",
            "verify",
            "docs__docs",
            "reverify",
            "done",
            "unverified",
        ];
        self.journal
            .iter()
            .filter_map(|(_, ev)| match ev {
                RunEvent::NodeStarted { node, .. } => Some(node.to_string()),
                _ => None,
            })
            .filter(|n| KEEP.contains(&n.as_str()))
            .collect()
    }

    fn started(&self, node: &str) -> bool {
        self.order().iter().any(|n| n == node)
    }

    /// How many `graph.call` observations name the sub-graph `graph`.
    fn calls_to(&self, graph: &str) -> usize {
        self.journal
            .iter()
            .filter(|(_, ev)| match ev {
                RunEvent::NodeObserved { kind, fields, .. } => {
                    kind == "graph.call" && fields["graph"] == graph
                }
                _ => false,
            })
            .count()
    }
}

/// Runs the tracked graph on a repo whose criterion is `criterion`, with the docs step's fake agent doing `docs`.
fn run(criterion: &str, docs: &str, docs_brief: bool) -> Ran {
    let (work, base) = repo(criterion);
    let bin = fake_bin(docs);
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    // The provider's key must be in the daemon's environment for a pi dispatch to start (agent_worker); a placeholder
    // stands for it here, and no agent reads it.
    let env = BTreeMap::from([(String::from("OLLAMA_API_KEY"), String::from("placeholder"))]);
    reg.register(Arc::new(CoderFactory::with_env(
        bin.path().to_path_buf(),
        env,
        log.clone(),
    )));
    reg.register(Arc::new(JudgeFactory));
    let frags = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "docs-frags-").unwrap();
    let woven = pii_graph::fragment::load_host_with_graphs(
        &tracked_source(),
        frags.path(),
        &[graphs_dir()],
        &reg,
    )
    .unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "docs-store-").unwrap();
    let store = Store::open(&store_dir.path().join("store.redb")).unwrap();
    let run_id = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &woven.spec,
        &reg,
        &root_ctx(bin.path(), &run_id),
        &run_id,
        input(work.path(), &base, docs_brief),
        true,
    )
    .unwrap();
    let journal = store.read_journal(&run_id).unwrap();
    let sessions: Vec<String> = log
        .0
        .lock()
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
    let session_texts = sessions
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap_or_default())
        .collect();
    let calls = std::fs::read_to_string(bin.path().join("calls.log"))
        .unwrap_or_default()
        .split("--end\n")
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect();
    Ran {
        terminal,
        journal,
        sessions,
        session_texts,
        calls,
    }
}

/// A session file's text is a pi header followed by `pids` pid lines.
fn assert_pi_file(text: &str, pids: usize) {
    assert!(
        text.starts_with("{\"type\":\"session\""),
        "pi header first: {text}"
    );
    assert_eq!(
        text.lines().count(),
        1 + pids,
        "header plus {pids} pids: {text}"
    );
}

#[test]
fn a_passing_run_documents_after_verify() {
    let r = run("test -f built.txt", DOCS_OK, true);
    assert!(
        matches!(r.terminal, Terminal::Success { .. }),
        "{:?}",
        r.terminal
    );
    assert_eq!(
        r.order(),
        // Spec 385: the implementer lives inside the methodology the driver picks, so this test's `order()`
        // observes the host's tail — the docs step, then the re-judge, then the end. The methodology's own
        // steps are asserted by spec_selects_a_methodology.
        ["docs__docs", "reverify", "done"],
        "the docs step runs between verify and reverify"
    );
    assert_eq!(r.calls_to("docs"), 1, "one graph.call for docs");
}

#[test]
fn the_docs_step_is_briefed_by_docs_brief() {
    let r = run("test -f built.txt", DOCS_OK, true);
    assert_eq!(r.calls.len(), 2, "two agent calls: {:?}", r.calls);
    let code = &r.calls[0];
    let docs = &r.calls[1];
    assert!(code.contains(IMPL) && !code.contains(DOCS), "code: {code}");
    assert!(docs.contains(DOCS) && !docs.contains(IMPL), "docs: {docs}");
}

#[test]
fn the_two_coders_have_two_sessions() {
    let r = run("test -f built.txt", DOCS_OK, true);
    assert_eq!(r.sessions.len(), 2, "both coders spawned: {:?}", r.sessions);
    assert_ne!(
        r.sessions[0], r.sessions[1],
        "two coders, two session paths"
    );
    for text in &r.session_texts {
        assert_pi_file(text, 1);
    }
}

#[test]
fn CONTROL_a_failed_verify_never_reaches_docs() {
    // Nothing creates `never.txt`, so the criterion fails at `verify`.
    let r = run("test -f never.txt", DOCS_OK, true);
    assert!(
        r.order().ends_with(&["unverified".to_owned()]),
        "{:?}",
        r.order()
    );
    assert!(
        !r.started("docs__docs"),
        "no docs node starts: {:?}",
        r.order()
    );
    assert!(!r.started("done"), "done never runs");
    assert!(
        r.calls.iter().all(|c| !c.contains(DOCS)),
        "no docs agent is spawned: {:?}",
        r.calls
    );
}

#[test]
fn reverify_catches_a_docs_step_that_breaks_the_criteria() {
    // The docs agent deletes the file the criterion checks: `reverify` must refuse the run.
    let r = run("test -f built.txt", DOCS_BREAK, true);
    assert_eq!(
        r.order(),
        ["docs__docs", "reverify", "unverified"],
        "the run ends at unverified, through reverify"
    );
    assert!(!r.started("done"), "done never runs");
    assert!(
        matches!(r.terminal, Terminal::Failure { .. }),
        "{:?}",
        r.terminal
    );
}

#[test]
fn CONTROL_a_missing_docs_brief_refuses_before_any_docs_agent() {
    let r = run("test -f built.txt", DOCS_OK, false);
    let Terminal::Failure { error } = &r.terminal else {
        panic!("refused: {:?}", r.terminal);
    };
    // The refusal names the sub-graph's declared key, which `docs_brief` maps into.
    assert!(error.contains("graph.call.need"), "{error}");
    assert!(error.contains("needs 'brief'"), "{error}");
    assert!(
        !r.started("docs__docs"),
        "no docs node starts: {:?}",
        r.order()
    );
    assert!(
        r.calls.iter().all(|c| !c.contains(DOCS)),
        "no docs agent is spawned: {:?}",
        r.calls
    );
}

#[test]
fn the_pin_is_the_real_digest_and_a_change_breaks_it() {
    let docs = std::fs::read(graphs_dir().join("docs.toml")).unwrap();
    let digest = pii_graph::fragment::sha256_hex(&docs);
    let spec = tracked_source();
    assert!(
        spec.contains(&format!("sha256 = \"{digest}\"")),
        "the call pins the digest of graphs/docs.toml"
    );
    // One byte changed in a copy of the graphs: the same pin must now refuse.
    let copy = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "docs-pin-").unwrap();
    let changed = String::from_utf8(docs)
        .unwrap()
        .replacen("1200000", "1200001", 1);
    // `spec` routes to the methodologies now, so the copy must hold every graph the pin chain can reach:
    // with only spec.toml and docs.toml the loader fails on `sdd` before it ever compares the docs pin,
    // and the test would pass for the wrong reason (or fail with the wrong message).
    for entry in std::fs::read_dir(graphs_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "toml") {
            let name = path.file_name().unwrap().to_owned();
            std::fs::copy(&path, copy.path().join(name)).unwrap();
        }
    }
    std::fs::write(copy.path().join("spec.toml"), &spec).unwrap();
    std::fs::write(copy.path().join("docs.toml"), changed).unwrap();
    let reg = NodeRegistry::builtin();
    let frags = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "docs-frags-").unwrap();
    let loaded = pii_graph::fragment::load_host_with_graphs(
        &spec,
        frags.path(),
        &[copy.path().to_path_buf()],
        &reg,
    );
    let Err(err) = loaded else {
        panic!("a changed sub-graph is refused");
    };
    assert!(err.to_string().contains("fragment.pin.mismatch"), "{err}");
}

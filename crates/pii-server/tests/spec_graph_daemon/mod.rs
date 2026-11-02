//! Spec 284: the harness the daemon-path tests share: fake agents, a throwaway git repo, the `Engine` builder.
//!
//! The workspace forbids `unsafe_code`, so a test cannot `set_var`. The outer test re-executes this test binary with
//! `PATH` starting at an empty bin directory; the child sees the marker, fills that directory and runs the body.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::missing_panics_doc,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the harness builds the git repo the judge reads its spec from, and re-executes the test binary; no production
// path runs through either
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{RunEvent, RunId, RunStatus, Terminal};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

pub const SPEC: &str = "specs/900-documented.md";
pub const IMPL: &str = "IMPL-BRIEF";
pub const DOCS: &str = "DOCS-BRIEF";
pub const DOCS_OK: &str = "echo docs > docs.md";
pub const DOCS_BREAK: &str = "rm -f built.txt; echo docs > docs.md";
/// Spec 285: the docs step hangs and records its pid in `pids` beside the fake `claude`. It then execs `sleep` under the
/// name it was spawned as (`exec -a "$0"`), so `/proc/<pid>/cmdline` starts with that path, as a real agent's does.
pub const DOCS_HANG: &str = "echo $$ >> \"${0%/*}/pids\"; exec -a \"$0\" /bin/sleep 100000";

/// The environment variable that marks the child: its value is the bin directory the outer test put first on `PATH`.
const MARK: &str = "PII_SPEC_GRAPH_DAEMON_BIN";

/// The fake `claude`: records its argv on one line of `argv.log` beside itself. The docs call's argv names `DOCS-BRIEF`,
/// and its step is `DOCS_BODY`; every other call (the code step) touches `built.txt`. It reports `total_cost_usd` 0.40.
const CLAUDE: &str = r#"#!/bin/bash
echo "$*" >> "${0%/*}/argv.log"
case "$*" in
  *DOCS-BRIEF*) DOCS_BODY ;;
  *) touch built.txt ;;
esac
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"num_turns":1,"total_cost_usd":0.40,"result":"done"}'
"#;

/// The fake `pi`: never selected, since the spec runs `agent = claude`, but the agent bin directory must hold both.
const PI: &str = "#!/bin/sh\nexit 0\n";

/// The tracked graph directory: `graphs/` at the workspace root.
pub fn tracked_graphs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

fn hello() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json")
}

/// Writes the fake agents into `bin`; the docs call's step is `docs`.
fn write_fake_agents(bin: &Path, docs: &str) {
    let claude = CLAUDE.replace("DOCS_BODY", docs);
    for (name, script) in [("pi", PI.to_owned()), ("claude", claude)] {
        let p = bin.join(name);
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }
}

/// Re-executes `test` in a child whose `PATH` starts with an empty bin directory, and returns `None`.
///
/// In the child (the marker is set) it fills that directory with the fake agents and returns it.
pub fn in_child(test: &str, docs: &str) -> Option<PathBuf> {
    if let Some(bin) = std::env::var_os(MARK) {
        write_fake_agents(Path::new(&bin), docs);
        return Some(PathBuf::from(bin));
    }
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "daemon-bin-").unwrap();
    let path = std::env::join_paths(std::iter::once(bin.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env("PATH", path)
        .env(MARK, bin.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child run of {test} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    None
}

/// One run's scratch world: the store and workspace under `dir`, the throwaway repo the judge reads, and the bin directory
/// the fake agents live in.
pub struct Case {
    pub dir: tempfile::TempDir,
    pub bin: PathBuf,
    pub work: PathBuf,
    pub base: String,
}

impl Case {
    /// A repo whose one commit holds `SPEC` with one criterion `criterion`, under the workspace; the fake agents
    /// already sit in `bin`.
    pub fn new(bin: &Path, criterion: &str) -> Self {
        let dir =
            pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "daemon-case-").unwrap();
        let work = dir.path().join("ws/repo");
        std::fs::create_dir_all(work.join("specs")).unwrap();
        git(&work, &["init", "-q"]);
        git(&work, &["config", "user.email", "t@example.invalid"]);
        git(&work, &["config", "user.name", "t"]);
        let blob = format!("# Spec 900\n\n## Exit Criteria\n\n- [ ] `{criterion}`\n");
        std::fs::write(work.join(SPEC), blob).unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "spec"]);
        let base = git(&work, &["rev-parse", "HEAD"]);
        Self {
            dir,
            bin: bin.to_path_buf(),
            work,
            base,
        }
    }

    /// The engine over `graphs`, with its store and workspace under this case's directory.
    pub fn engine(&self, graphs: &Path) -> Arc<Engine> {
        self.engine_on_store(graphs, self.dir.path().join("store.redb"))
    }

    /// The engine over `graphs` on the store at `store_path` (spec 285: a copy of the store opens as a restarted daemon).
    pub fn engine_on_store(&self, graphs: &Path, store_path: PathBuf) -> Arc<Engine> {
        Engine::new(EngineConfig {
            grant_ceiling: None,
            workspace: self.dir.path().join("ws"),
            store_path,
            model: format!("scripted:{}", hello().display()),
            sandbox: SandboxProfileName::Off,
            route_policy: BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
            graph_dir: graphs.to_path_buf(),
            mqtt_tool_url: None,
        })
        .expect("engine")
    }

    /// The run input for `spec` with `agent = claude` and a $3.00 ceiling; `docs_brief` is absent when false.
    pub fn input(&self, docs_brief: bool) -> serde_json::Value {
        let mut v = serde_json::json!({
            "agent": "claude",
            "model": "m-one",
            // The called graph needs a provider; claude names none, so the agent's own name stands for it.
            "provider": "claude",
            "spec": SPEC,
            "workdir": self.work.display().to_string(),
            "brief": IMPL,
            "base": self.base,
            "max_budget_usd": 3.00,
            "docs_brief": DOCS,
        });
        if !docs_brief {
            v.as_object_mut().unwrap().remove("docs_brief");
        }
        v
    }

    /// One string per agent call, in call order: the argv the fake `claude` recorded. A brief spans lines, so each call is
    /// split on the `--session-id` that opens its argv.
    pub fn agent_argv(&self) -> Vec<String> {
        std::fs::read_to_string(self.bin.join("argv.log"))
            .unwrap_or_default()
            .split("--session-id ")
            .skip(1)
            .map(|call| format!("--session-id {call}"))
            .collect()
    }
}

/// A copy of the tracked graphs under `dir`, with `spec.toml`'s `[limits]` ceiling replaced by `micro_usd` when given.
pub fn graphs_copy(dir: &Path, micro_usd: Option<u64>) -> PathBuf {
    let graphs = dir.join("graphs");
    std::fs::create_dir_all(&graphs).unwrap();
    let spec = std::fs::read_to_string(tracked_graphs().join("spec.toml")).unwrap();
    let spec = match micro_usd {
        Some(m) => spec.replacen(
            "max_micro_usd = 1000000",
            &format!("max_micro_usd = {m}"),
            1,
        ),
        None => spec,
    };
    std::fs::write(graphs.join("spec.toml"), spec).unwrap();
    std::fs::copy(tracked_graphs().join("docs.toml"), graphs.join("docs.toml")).unwrap();
    graphs
}

/// Polls the store until the run reaches a terminal status, then returns that status and its journal.
pub async fn settle(engine: &Engine, run: &RunId) -> (RunStatus, Vec<(u64, RunEvent)>) {
    for _ in 0..1000 {
        // The header is written by the spawned run thread, so "not yet" is a normal first poll.
        if let Some(header) = engine.store.get_run(run).unwrap()
            && matches!(header.status, RunStatus::Succeeded | RunStatus::Failed)
        {
            let journal = engine.store.read_journal(run).unwrap();
            return (header.status, journal);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run {run} did not settle");
}

/// The nodes the journal started, in order.
pub fn started(journal: &[(u64, RunEvent)]) -> Vec<String> {
    journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::NodeStarted { node, .. } => Some(node.to_string()),
            _ => None,
        })
        .collect()
}

/// The `(node, error)` of every node that failed, in journal order.
pub fn failures(journal: &[(u64, RunEvent)]) -> Vec<(String, String)> {
    journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::NodeFailed { node, error, .. } => Some((node.to_string(), error.clone())),
            _ => None,
        })
        .collect()
}

/// The `(node, fields)` of every observation of `kind`, in journal order.
pub fn observed(journal: &[(u64, RunEvent)], kind: &str) -> Vec<(String, serde_json::Value)> {
    journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::NodeObserved {
                node,
                kind: k,
                fields,
            } if k == kind => Some((node.to_string(), fields.clone())),
            _ => None,
        })
        .collect()
}

/// Spec 285: the host that calls `docs` twice in sequence; `SHA` is the digest of `docs.toml`.
pub const TWICE: &str = r#"name = "twice"
entry = "first"
caps = ["spawn", "proc"]
node = [
  { id = "first", kind = "call", graph = "docs", sha256 = "SHA", caps = ["spawn", "proc"], in = { brief = "$.input.docs_brief" } },
  { id = "second", kind = "call", graph = "docs", sha256 = "SHA", caps = ["spawn", "proc"], in = { brief = "$.input.docs_brief" } },
  { id = "done", kind = "transform", value = "done" },
]
edge = [
  { from = "first", port = "ok", to = "second", kind = "data" },
  { from = "second", port = "ok", to = "done", kind = "data" },
]
"#;

/// Spec 285: a host whose call loops back on itself: `again` returns to `docs` until its loop budget is spent.
pub const LOOPED: &str = r#"name = "looped"
entry = "docs"
caps = ["spawn", "proc"]
node = [
  { id = "docs", kind = "call", graph = "docs", sha256 = "SHA", caps = ["spawn", "proc"], in = { brief = "$.input.docs_brief" } },
  { id = "again", kind = "transform", value = "again", loop_budget = 2 },
]
edge = [
  { from = "docs", port = "ok", to = "again", kind = "data" },
  { from = "again", port = "ok", to = "docs", kind = "data" },
]
"#;

/// Spec 285: a host that calls `needy`; the call takes the host's input, so `x` reaches it when the host got `x`.
pub const NEEDY_HOST: &str = r#"name = "needy-host"
entry = "start"
node = [
  { id = "start", kind = "transform", value = "go" },
  { id = "need", kind = "call", graph = "needy", sha256 = "SHA", caps = [] },
  { id = "done", kind = "transform", expr = "$.payload" },
]
edge = [
  { from = "start", port = "ok", to = "need", kind = "data" },
  { from = "need", port = "ok", to = "done", kind = "data" },
]
"#;

/// Spec 285: the sub-graph that declares `needs = ["x"]`.
pub const NEEDY: &str = r#"name = "needy"
entry = "fmt"
exit = "fmt"
needs = ["x"]
node = [{ id = "fmt", kind = "transform", expr = "$.input.x" }]
"#;

/// Spec 285: the digest of the tracked `docs.toml`, which a host's `sha256` pin must name.
pub fn docs_digest() -> String {
    pii_graph::fragment::sha256_hex(&std::fs::read(tracked_graphs().join("docs.toml")).unwrap())
}

/// True for the coder a called graph runs: the call's id, `__`, then the coder's own id (`docs__docs`).
pub fn is_call_coder(node: &str) -> bool {
    node.contains("__") && node.ends_with("__docs")
}

/// Polls until a coder inside a call has started, and returns its node. The journal holds `coder.spawned` only once the
/// node returns (spec 217), so the start is the signal the journal gives for a live agent.
pub async fn call_coder_started(engine: &Engine, run: &RunId) -> String {
    for _ in 0..1000 {
        let journal = engine.store.read_journal(run).unwrap();
        if let Some(node) = started(&journal).into_iter().find(|n| is_call_coder(n)) {
            return node;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no coder inside a call started for {run}");
}

/// Polls until `n` fake agents have recorded their pids, and returns them.
pub async fn wait_pids(sleepers: &Sleepers, n: usize) -> Vec<u32> {
    for _ in 0..1000 {
        if sleepers.pids().len() >= n {
            return sleepers.pids();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{n} fake agents never recorded a pid");
}

/// True once `pid` is gone, within two seconds.
pub async fn wait_dead(pid: u32) -> bool {
    for _ in 0..200 {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

/// The journal's `graph.call` observations for `docs`.
pub fn docs_calls(journal: &[(u64, RunEvent)]) -> Vec<(String, serde_json::Value)> {
    observed(journal, "graph.call")
        .into_iter()
        .filter(|(_, f)| f["graph"] == "docs")
        .collect()
}

/// A run's failure text: its node failures and its terminal error, joined.
pub fn reason(journal: &[(u64, RunEvent)]) -> String {
    let mut text: Vec<String> = failures(journal).into_iter().map(|(_, e)| e).collect();
    for (_, ev) in journal {
        if let RunEvent::Finished {
            terminal: Terminal::Failure { error },
            ..
        } = ev
        {
            text.push(error.clone());
        }
    }
    text.join("\n")
}

/// The argv the fake agent recorded for the coder whose session is `session`.
pub fn argv_of(case: &Case, session: &str) -> String {
    case.agent_argv()
        .into_iter()
        .find(|a| a.starts_with(&format!("--session-id {session}")))
        .unwrap_or_default()
}

/// The store as a restarted daemon finds it: a copy of `case`'s store, opened by a new engine over `graphs`.
pub fn restarted(case: &Case, graphs: &Path) -> Arc<Engine> {
    let copy = case.dir.path().join("restart");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(case.dir.path().join("store.redb"), copy.join("store.redb")).unwrap();
    case.engine_on_store(graphs, copy.join("store.redb"))
}

/// `restarted`, but only once the copied store holds `want` `coder.spawned` events for `run`. Spec 289 makes a spawn durable WITHIN
/// two seconds (the barrier's `START_DURABLE_WITHIN`), not at the instant the agent starts, so a copy taken the moment a fake agent
/// records its pid can legitimately miss it (measured under 32 CPU burners: the agent was alive and sleeping, the restart found nothing
/// to reap). Retries for six seconds, then panics: a spawn that never becomes durable still fails the test.
pub async fn restarted_when_durable(
    case: &Case,
    graphs: &Path,
    run: &RunId,
    want: usize,
) -> Arc<Engine> {
    let mut seen = 0;
    for _ in 0..120 {
        let engine = restarted(case, graphs);
        seen = observed(&engine.store.read_journal(run).unwrap(), "coder.spawned").len();
        if seen >= want {
            return engine;
        }
        drop(engine);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the copied store held {seen} of {want} coder.spawned after six seconds");
}

/// The engine over the tracked graphs plus `twice.toml`, the host that calls `docs` twice.
pub fn twice_engine(case: &Case) -> Arc<Engine> {
    let graphs = graphs_copy(case.dir.path(), None);
    std::fs::write(
        graphs.join("twice.toml"),
        TWICE.replace("SHA", &docs_digest()),
    )
    .unwrap();
    case.engine(&graphs)
}

/// The engine over the tracked graphs plus `needy` and its host `needy-host`.
pub fn needy_engine(case: &Case) -> Arc<Engine> {
    let graphs = graphs_copy(case.dir.path(), None);
    let needy = pii_graph::fragment::sha256_hex(NEEDY.as_bytes());
    std::fs::write(graphs.join("needy.toml"), NEEDY).unwrap();
    std::fs::write(
        graphs.join("needy-host.toml"),
        NEEDY_HOST.replace("SHA", &needy),
    )
    .unwrap();
    case.engine(&graphs)
}

/// The sleeping stand-ins the fake agents recorded in `bin/pids`. Dropped, it kills every one still alive, so a failed
/// assertion leaks no sleeper.
pub struct Sleepers {
    bin: PathBuf,
}

impl Sleepers {
    pub fn new(bin: &Path) -> Self {
        Self {
            bin: bin.to_path_buf(),
        }
    }

    /// Every shell pid the fake agents have recorded so far, in order.
    pub fn pids(&self) -> Vec<u32> {
        std::fs::read_to_string(self.bin.join("pids"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect()
    }
}

impl Drop for Sleepers {
    fn drop(&mut self) {
        for pid in self.pids() {
            // The coder starts the agent in its own process group, so the group goes first, then the pid itself.
            let _ = Command::new("kill")
                .args(["-9", "--", &format!("-{pid}")])
                .output();
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
        }
    }
}

/// True while `pid` names a live process. Signal 0 sends nothing: it only asks.
pub fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|out| out.status.success())
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

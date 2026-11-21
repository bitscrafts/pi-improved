//! Spec 430 R10-R14: `graphs/worker-runtime.toml` decides what happens to the worker's container, exercised with no
//! container runtime: the graph's `tool = "run"` nodes take their argv as input, so a stub stands in for podman,
//! answers from a file and journals every call. The ONLY thing the harness rewrites is the program name, so the rules
//! under test are the shipped ones (`the_injected_copy_differs_only_in_the_program_name`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{NodeId, PortName, RunEvent, RunStatus, Terminal};
use pii_graph::fragment;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const GRAPH: &str = "worker-runtime";
/// The digest the container was created from, and a different one from the image's.
const OLD: &str = "sha256:0ldc0nta1ner";
const NEW: &str = "sha256:new1mage";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn graph_src(name: &str) -> String {
    std::fs::read_to_string(repo().join(format!("graphs/{name}.toml")))
        .unwrap_or_else(|e| panic!("graphs/{name}.toml: {e}"))
}

/// The `[[node]]` tables of a graph source.
fn nodes(src: &str) -> Vec<toml::Table> {
    let doc: toml::Table = toml::from_str(src).expect("valid toml");
    let all = doc.get("node").and_then(toml::Value::as_array);
    all.map_or_else(Vec::new, |a| {
        a.iter()
            .filter_map(toml::Value::as_table)
            .cloned()
            .collect()
    })
}

fn scratch(prefix: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).expect("tempdir")
}

/// The injected runtime: a stub program that answers from files and journals every call.
struct Probe {
    dir: tempfile::TempDir,
    program: PathBuf,
}

impl Probe {
    fn new() -> Self {
        let dir = scratch("worker-runtime-probe-");
        // `$1` is the subcommand, `$2` the file the call answers from (`unreachable` = a dead runtime), `$3` the
        // journal of calls. Every call is appended BEFORE it answers, so a call that fails is still a call.
        let script = "#!/bin/sh\n\
             printf '%s\\n' \"$1\" >> \"$3\"\n\
             case \"$1\" in\n\
             \x20 ps|image)\n\
             \x20   if [ \"$2\" = unreachable ]; then echo 'cannot connect to the runtime' >&2; exit 125; fi\n\
             \x20   cat \"$2\" ;;\n\
             \x20 start|create|rm) exit 0 ;;\n\
             \x20 *) echo \"stub runtime: unknown subcommand '$1'\" >&2; exit 2 ;;\n\
             esac\n";
        let program = dir.path().join("podman");
        std::fs::write(&program, script).expect("stub written");
        std::fs::set_permissions(
            &program,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("stub is executable");
        Self { dir, program }
    }

    /// A file holding the canned answer `stdout` for one call.
    fn answer(&self, name: &str, stdout: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, stdout).expect("answer written");
        path
    }

    fn log(&self) -> PathBuf {
        self.dir.path().join("calls.log")
    }

    /// The subcommands the runtime received, in order.
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

struct Harness {
    dir: tempfile::TempDir,
    ws: PathBuf,
    probe: Probe,
    engine: Arc<Engine>,
}

impl Harness {
    /// An engine whose graph directory holds the shipped graph with its program replaced by the stub.
    fn new() -> Self {
        let dir = scratch("worker-runtime-");
        let probe = Probe::new();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(&ws).expect("workspace");
        let graphs = dir.path().join("graphs");
        std::fs::create_dir_all(&graphs).expect("graph dir");
        let injected = graph_src(GRAPH).replace(
            "programs = [\"podman\"]",
            &format!("programs = [\"{}\"]", probe.program.display()),
        );
        assert!(
            injected != graph_src(GRAPH),
            "the injection changes the graph"
        );
        assert!(
            !injected.contains("programs = [\"podman\"]"),
            "every runtime node is pointed at the stub"
        );
        std::fs::write(graphs.join(format!("{GRAPH}.toml")), injected).expect("graph written");
        let engine = Engine::new(EngineConfig {
            grant_ceiling: None,
            workspace: ws.clone(),
            store_path: dir.path().join("store.redb"),
            // No node in this graph calls a model; the scripted fixture is what `Engine::new` validates at boot.
            model: format!(
                "scripted:{}",
                repo().join("fixtures/phase2/hello.json").display()
            ),
            sandbox: SandboxProfileName::Off,
            route_policy: std::collections::BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
            models: pii_server::daemon::empty_model_roles(),
            graph_dir: graphs,
            mqtt_tool_url: None,
        })
        .expect("engine");
        Self {
            dir,
            ws: std::fs::canonicalize(&ws).expect("canonical workspace"),
            probe,
            engine,
        }
    }

    /// The graph input for a canned pair of answers: what `find` prints, and what `digest` prints.
    fn input(&self, found: &str, digest: &str) -> serde_json::Value {
        let find = if found == "unreachable" {
            PathBuf::from("unreachable")
        } else {
            self.probe.answer("find.txt", found)
        };
        let now = self.probe.answer("digest.txt", digest);
        let log = self.probe.log();
        let argv = |sub: &str, from: &Path| {
            serde_json::json!([
                self.probe.program.display().to_string(),
                sub,
                from.display().to_string(),
                log.display().to_string()
            ])
        };
        serde_json::json!({
            "instance": "worker",
            "image": "pii-worker:test",
            "host_root": "/projects",
            "repo_subpath": "pi-improved",
            "probe": ["true"],
            "cwd": self.ws.display().to_string(),
            "find_argv": argv("ps", &find),
            "digest_argv": argv("image", &now),
            "create_argv": argv("create", Path::new("-")),
            "start_argv": argv("start", Path::new("-")),
            "recreate_argv": argv("rm", Path::new("-")),
        })
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }
}

/// Runs the graph with `input` and settles it: `(run header, journal)` — the pair the github-issue harness's
/// `run_graph` returns, read from the store instead of over the loopback socket.
async fn run_graph(
    e: &Arc<Engine>,
    input: serde_json::Value,
) -> (serde_json::Value, Vec<(u64, RunEvent)>) {
    let run = e.start_run(GRAPH, &input).expect("the graph starts");
    for _ in 0..600 {
        if let Some(header) = e.store.get_run(&run).expect("header")
            && matches!(header.status, RunStatus::Succeeded | RunStatus::Failed)
        {
            let journal = e.store.read_journal(&run).expect("journal");
            let run = serde_json::to_value(&header).expect("header serialises");
            return (run, journal);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("run {run} did not settle");
}

fn started(journal: &[(u64, RunEvent)], id: &str) -> bool {
    journal.iter().any(|(_, ev)| match ev {
        RunEvent::NodeStarted { node, .. } => *node == NodeId::from(id),
        _ => false,
    })
}

fn finished_on(journal: &[(u64, RunEvent)], id: &str) -> Option<PortName> {
    journal.iter().find_map(|(_, ev)| match ev {
        RunEvent::NodeFinished { node, port, .. } if *node == NodeId::from(id) => {
            Some(port.clone())
        }
        _ => None,
    })
}

fn terminal(journal: &[(u64, RunEvent)]) -> Terminal {
    journal
        .iter()
        .find_map(|(_, ev)| match ev {
            RunEvent::Finished { terminal, .. } => Some(terminal.clone()),
            _ => None,
        })
        .expect("the run reached a terminal")
}

/// The terminal output of a successful run.
fn output(journal: &[(u64, RunEvent)]) -> serde_json::Value {
    match terminal(journal) {
        Terminal::Success { output } => output,
        other => panic!("expected a success, got {other:?}"),
    }
}

/// The graph as shipped loads through the server's own registry: it is a graph a lane can run, not a draft.
#[test]
fn the_shipped_graphs_load_and_encode_the_four_decisions() {
    let reg = pii_server::lint_registry(Path::new("."), Path::new("."));
    for name in ["podman", GRAPH] {
        let src = graph_src(name);
        fragment::load_host_with_graphs(
            &src,
            &repo().join("graphs/fragments"),
            std::slice::from_ref(&repo().join("graphs")),
            &reg,
        )
        .unwrap_or_else(|e| panic!("graphs/{name}.toml must load: {e}"));
    }

    let src = graph_src(GRAPH);
    let all = nodes(&src);
    let decide = all
        .iter()
        .find(|n| n.get("id").and_then(toml::Value::as_str) == Some("decide"))
        .expect("a `decide` node");
    let ports: Vec<&str> = decide
        .get("rules")
        .and_then(toml::Value::as_array)
        .expect("`decide` takes `rules`")
        .iter()
        .filter_map(|r| r.get("port").and_then(toml::Value::as_str))
        .collect();
    assert_eq!(
        ports,
        ["create", "reuse", "start", "recreate"],
        "first match wins, in the order the decision is written"
    );

    // Spec 400 R1: a `run` node's program is a literal name in the file, never run input.
    for node in &all {
        if node.get("tool").and_then(toml::Value::as_str) == Some("run") {
            let programs: Vec<&str> = node
                .get("programs")
                .and_then(toml::Value::as_array)
                .expect("a `run` node declares `programs`")
                .iter()
                .filter_map(toml::Value::as_str)
                .collect();
            assert_eq!(programs, ["podman"], "node {node:?} names the runtime");
        }
    }
}

/// Nothing found: the container is created.
#[tokio::test]
async fn decide_creates_when_absent() {
    let h = Harness::new();
    let (run, journal) = run_graph(&h.engine, h.input("", NEW)).await;

    assert_eq!(run["status"], "Succeeded", "{run:?}\n{journal:?}");
    assert_eq!(finished_on(&journal, "create"), Some(PortName::from("ok")));
    assert_eq!(output(&journal), serde_json::json!("ready:created"));
    assert_eq!(
        h.probe.calls(),
        ["ps", "image", "create"],
        "the runtime is asked what exists, what the image is, then told to create"
    );
    assert!(!started(&journal, "start"), "nothing was stopped");
    assert!(!started(&journal, "recreate"), "nothing was replaced");
}

/// Running and the same image: the container is reused, and nothing is created or started.
#[tokio::test]
async fn decide_reuses_a_running_container() {
    let h = Harness::new();
    let (run, journal) = run_graph(&h.engine, h.input(&format!("running|{NEW}\n"), NEW)).await;

    assert_eq!(run["status"], "Succeeded", "{run:?}\n{journal:?}");
    assert_eq!(
        finished_on(&journal, "end_reused"),
        Some(PortName::from("ok"))
    );
    assert_eq!(output(&journal), serde_json::json!("ready:reused"));
    assert_eq!(
        h.probe.calls(),
        ["ps", "image"],
        "asking is not acting: reuse makes no call that changes the container"
    );
}

/// Stopped with the same image: the container is started.
#[tokio::test]
async fn decide_starts_a_stopped_container() {
    let h = Harness::new();
    let (run, journal) = run_graph(&h.engine, h.input(&format!("exited|{NEW}\n"), NEW)).await;

    assert_eq!(run["status"], "Succeeded", "{run:?}\n{journal:?}");
    assert_eq!(finished_on(&journal, "start"), Some(PortName::from("ok")));
    assert_eq!(output(&journal), serde_json::json!("ready:started"));
    assert_eq!(h.probe.calls(), ["ps", "image", "start"]);
    assert!(!started(&journal, "create"), "nothing was absent");
    assert!(!started(&journal, "recreate"), "the image is current");
}

/// The image moved on: the container is recreated.
#[tokio::test]
async fn decide_recreates_on_digest_change() {
    let h = Harness::new();
    let (run, journal) = run_graph(&h.engine, h.input(&format!("running|{OLD}\n"), NEW)).await;

    assert_eq!(run["status"], "Succeeded", "{run:?}\n{journal:?}");
    assert_eq!(
        finished_on(&journal, "recreate"),
        Some(PortName::from("ok"))
    );
    assert_eq!(output(&journal), serde_json::json!("ready:recreated"));
    assert_eq!(
        h.probe.calls(),
        ["ps", "image", "rm"],
        "a stale container is replaced, not started"
    );
    assert!(!started(&journal, "start"), "a stale image is not reused");
}

/// CONTROL: the same digest must NOT recreate. Without this, a graph that recreates on every run passes the
/// test above, and the container is rebuilt under every row.
#[tokio::test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
async fn CONTROL_decide_same_digest_never_recreates() {
    let h = Harness::new();
    let (run, journal) = run_graph(&h.engine, h.input(&format!("running|{NEW}\n"), NEW)).await;

    assert_eq!(run["status"], "Succeeded", "{run:?}\n{journal:?}");
    assert!(
        !started(&journal, "recreate"),
        "an equal digest is never a reason to recreate: {:?}",
        h.probe.calls()
    );
    assert!(
        !h.probe.calls().contains(&String::from("rm")),
        "no container was removed: {:?}",
        h.probe.calls()
    );
    assert_eq!(output(&journal), serde_json::json!("ready:reused"));
}

/// R12: a second run is ready again, and creates, starts and recreates nothing.
#[tokio::test]
async fn a_second_run_is_ready_and_creates_nothing() {
    let h = Harness::new();
    let input = h.input(&format!("running|{NEW}\n"), NEW);

    let (first, first_journal) = run_graph(&h.engine, input.clone()).await;
    let after_first = h.probe.calls().len();
    let (second, second_journal) = run_graph(&h.engine, input).await;

    assert_eq!(first["status"], "Succeeded", "{first:?}");
    assert_eq!(second["status"], "Succeeded", "{second:?}");
    assert_eq!(output(&first_journal), output(&second_journal));
    assert_eq!(
        h.probe.calls()[after_first..],
        ["ps", "image"],
        "the second run asks and stops asking"
    );
    assert!(
        !started(&second_journal, "create")
            && !started(&second_journal, "start")
            && !started(&second_journal, "recreate"),
        "a ready container is left alone: {second_journal:?}"
    );
}

/// No runtime: the run reports the named reason and no container is created.
#[tokio::test]
async fn an_unreachable_runtime_creates_nothing() {
    let h = Harness::new();
    let (run, journal) = run_graph(&h.engine, h.input("unreachable", NEW)).await;

    assert_eq!(run["status"], "Failed", "{run:?}\n{journal:?}");
    let error = match terminal(&journal) {
        Terminal::Failure { error } => error,
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert!(
        error.contains("no container runtime"),
        "the reason is named, not inferred: {error}"
    );
    assert_eq!(h.probe.calls(), ["ps"], "the runtime was asked once");
    assert!(!started(&journal, "create"), "nothing was created");
}

/// The harness rewrites the program name and nothing else: the rules under test are the shipped ones.
#[test]
fn the_injected_copy_differs_only_in_the_program_name() {
    let h = Harness::new();
    let shipped = graph_src(GRAPH);
    let injected = std::fs::read_to_string(h.root().join(format!("graphs/{GRAPH}.toml")))
        .expect("the injected graph");
    let back = injected.replace(
        &format!("programs = [\"{}\"]", h.probe.program.display()),
        "programs = [\"podman\"]",
    );
    assert_eq!(back, shipped, "the decision is exercised as it ships");
}

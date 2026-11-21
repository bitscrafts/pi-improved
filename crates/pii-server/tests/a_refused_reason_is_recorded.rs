//! Spec 428: a node that ends on a branch leading to a refusal writes ONE `graph.node.failed` event at `warn`, naming the
//! run, the node and the node's own error text. A healthy run writes none. The run stays `succeeded` (a modelled refusal
//! is not a run failure), and the reason is in the run's journal, which `pii run explain` reads.
//!
//! The run is a real daemon run (as in `github_issue_graph`): the planner is a scripted provider whose fixture is the
//! reply it returns, and an empty fixture makes its call fail after the model is asked.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_observe::{Level, OwnedEvent, Recorder};
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const FAILED: &str = "graph.node.failed";
const FAILING: &str = "failing-plan";
const HEALTHY: &str = "healthy-pair";
/// The node's own error text, as its failed call journals it (`NodeFinished` on the `err` port).
const REASON: &str = "scripted provider exhausted";

/// A planner that fails: no scripted turns, so its one model call has nothing to answer with.
fn planner_fixture(dir: &Path) -> PathBuf {
    let path = dir.join("planner-fixture.json");
    std::fs::write(&path, serde_json::json!({ "turns": [] }).to_string()).unwrap();
    path
}

/// `plan` fails and its `err` edge leads to `refused`: the shape of `graphs/github-issue.toml`, reduced to two nodes.
const FAILING_SRC: &str = r#"
name = "failing-plan"
entry = "plan"
exit = "refused"
caps = ["fs_read", "fs_write", "net", "proc"]

[[node]]
id = "plan"
kind = "agent"
caps = ["fs_read"]
tools = ["fs_read"]
model = "@plan"
max_turns = 1
prompt = "Draft one spec for the finding."

[[node]]
id = "refused"
kind = "transform"
value = { text = "", outcome = "refused" }

[[edge]]
from = "plan"
port = "err"
to = "refused"
kind = "data"
"#;

/// Two nodes that both succeed: the control, which must write no failure event.
const HEALTHY_SRC: &str = r#"
name = "healthy-pair"
entry = "first"
exit = "second"

[[node]]
id = "first"
kind = "transform"
value = { text = "a" }

[[node]]
id = "second"
kind = "transform"
value = { text = "b" }

[[edge]]
from = "first"
port = "ok"
to = "second"
kind = "data"
"#;

struct Harness {
    d: Arc<Daemon>,
    log: Arc<Recorder>,
    _dir: tempfile::TempDir,
}

fn scratch(prefix: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).expect("tempdir")
}

/// A daemon over a fresh directory, with the log attached BEFORE the daemon is built (as `pii daemon` does) and the
/// given graph installed under its own name.
fn daemon_with(prefix: &str, graph: &str, src: &str) -> Harness {
    let dir = scratch(prefix);
    let root = dir.path();
    let ws = root.join("ws");
    std::fs::create_dir_all(ws.join("specs")).unwrap();
    std::fs::create_dir_all(root.join("graphs")).unwrap();
    std::fs::write(root.join(format!("graphs/{graph}.toml")), src).unwrap();
    let planner = planner_fixture(root);
    let spec = format!("scripted:{}", planner.display());
    let models = [(
        String::from("plan"),
        pii_server::daemon::ModelRole {
            model: spec.clone(),
            provider: spec.clone(),
        },
    )]
    .into();
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: ws,
        store_path: root.join("store.redb"),
        model: spec,
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models,
        graph_dir: root.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine");
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let cfg_path = root.join("pii.toml");
    std::fs::write(&cfg_path, "").unwrap();
    let mut cfg = Config::load(&cfg_path).expect("config");
    cfg.core.durable_dir = Some(root.join("durable").display().to_string());
    let d = Daemon::build(
        engine,
        &cfg,
        root,
        cfg_path,
        Arc::new(pii_core::SystemClock),
    )
    .expect("daemon");
    Harness { d, log, _dir: dir }
}

async fn call(h: &Harness, m: Method, params: serde_json::Value) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(&h.d.engine));
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-428".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _) = pii_proto::client::Client::connect(r, w, info)
        .await
        .expect("connect");
    let out = client.call(m, params).await;
    client.shutdown();
    out.unwrap_or_else(|e| serde_json::json!({"error": format!("{e:?}")}))
}

/// Starts `graph` and waits for it to end: `(run id, run header, trace)`.
async fn run_to_end(h: &Harness, graph: &str) -> (String, serde_json::Value, serde_json::Value) {
    let started = call(
        h,
        Method::RunStart,
        serde_json::json!({"graph": graph, "input": {"text": "the finding"}}),
    )
    .await;
    let rid = started["run_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the run starts: {started}"))
        .to_owned();
    for _ in 0..300 {
        let head = call(h, Method::RunGet, serde_json::json!({"run_id": rid})).await;
        if matches!(head["status"].as_str(), Some("succeeded" | "failed")) {
            let trace = call(h, Method::RunTrace, serde_json::json!({"run_id": rid})).await;
            return (rid, head, trace);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("run {rid} never terminated");
}

fn failure_events(log: &Recorder) -> Vec<OwnedEvent> {
    log.events_named(FAILED)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_node_names_itself_in_the_log() {
    let h = daemon_with("refused-log-", FAILING, FAILING_SRC);
    let (rid, head, _) = run_to_end(&h, FAILING).await;
    assert_eq!(
        head["status"], "succeeded",
        "a refusal is modelled, not failed: {head}"
    );

    let events = failure_events(&h.log);
    assert_eq!(events.len(), 1, "one failure event: {:?}", h.log.events());
    let ev = &events[0];
    assert_eq!(ev.level, Level::Warn, "{ev:?}");
    let text = serde_json::to_string(ev).unwrap();
    assert!(text.contains(&rid), "the event names the run: {text}");
    assert!(text.contains("plan"), "the event names the node: {text}");
    assert!(
        text.contains(REASON),
        "the event carries the node's own error text: {text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[allow(non_snake_case)] // reason: the name is the spec's contract (a CONTROL test)
async fn CONTROL_a_healthy_run_writes_no_failure_event() {
    let h = daemon_with("refused-healthy-", HEALTHY, HEALTHY_SRC);
    let (_, head, _) = run_to_end(&h, HEALTHY).await;
    assert_eq!(head["status"], "succeeded", "{head}");
    assert!(
        failure_events(&h.log).is_empty(),
        "a healthy run logs no failure: {:?}",
        h.log.events()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn explain_prints_the_last_failed_node() {
    // `pii run explain` prints a node's failure from the journal it reads through `run.trace` (spec 220): a `NodeFailed`
    // for the node, or its failure observation. The renderer lives in pii-cli; this asserts the journal it prints from,
    // which must name `plan` and carry its reason.
    let h = daemon_with("refused-explain-", FAILING, FAILING_SRC);
    let (_, _, trace) = run_to_end(&h, FAILING).await;
    let journal = trace["journal"].as_array().cloned().unwrap_or_default();
    assert!(
        journal.iter().any(|e| names_plan_failure(&e["event"])),
        "the journal names the failed node and its reason: {journal:?}"
    );
}

/// Whether one journal event is `plan`'s failure with the reason: a `NodeFailed`, or a `graph.node.failed` observation.
fn names_plan_failure(ev: &serde_json::Value) -> bool {
    let failed = ev.get("NodeFailed").is_some_and(|f| {
        f["node"] == "plan" && f["error"].as_str().is_some_and(|e| e.contains(REASON))
    });
    let observed = ev.get("NodeObserved").is_some_and(|o| {
        o["node"] == "plan" && o["kind"] == FAILED && o["fields"].to_string().contains(REASON)
    });
    failed || observed
}

//! Spec 403: `review-one` and `review` are a generic reviewer built from existing node kinds.
//!
//!   * a reviewer passes only on `VERDICT: PASS` as the first line, and every other answer is a failure (R4);
//!   * a rubric is a file under `rubrics/`, and a name that leaves that directory is refused before anything is read (R2);
//!   * the reviewer is read-only by construction (R3);
//!   * the fan-out is four pinned calls: a skipped slot is neither a pass nor a failure, and an absent `rubric_1` is refused (R5).
//!
//! The scripted provider answers in call order, so the fan-out tests give every reviewer the same answer.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use pii_abi::{NodeId, NodeInput, PiiError, PortName, RunEvent, RunStatus};
use pii_core::{Budget, CapSet, Ctx};
use pii_graph::node::NodeFactory as _;
use pii_model::{ChatRequest, ChatResponse, ModelProvider};
use pii_server::node_kinds::agent::AgentFactory;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const DOC: &str = "specs/probe-403.md";
const WORKDIR: &str = "/projects/probe-workdir-403";
const SYSTEM: &str = "You are a read-only reviewer. Judge only the document named below against the rubric below. Answer in the format the rubric requires.";
const RUBRIC_BODY: &str = "Check the document against the requirements.\n";

mod fan_out;
mod recording;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A temp dir under the repo's `_tmp/`, because the sandbox does not let tests write to `/tmp`.
fn scratch() -> tempfile::TempDir {
    let root = repo().join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("review-403-")
        .tempdir_in(root)
        .unwrap()
}

fn graph(name: &str) -> String {
    std::fs::read_to_string(repo().join(format!("graphs/{name}.toml")))
        .unwrap_or_else(|e| panic!("graphs/{name}.toml: {e}"))
}

fn table(src: &str) -> toml::Table {
    toml::from_str(src).expect("valid toml")
}

fn nodes_of_kind<'a>(doc: &'a toml::Table, kind: &str) -> Vec<&'a toml::Table> {
    doc.get("node")
        .and_then(toml::Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(toml::Value::as_table)
                .filter(|n| n.get("kind").and_then(toml::Value::as_str) == Some(kind))
                .collect()
        })
        .unwrap_or_default()
}

fn strings(node: &toml::Table, key: &str) -> Vec<String> {
    node.get(key)
        .and_then(toml::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Spec 403 R3: the reviewer may hold only `fs_read`, in its tools and its caps, and takes at most 12 turns.
/// Returns every violation; an empty list means the reviewer is read-only.
fn read_only_violations(doc: &toml::Table) -> Vec<String> {
    let agents = nodes_of_kind(doc, "agent");
    let mut out = Vec::new();
    if agents.len() != 1 {
        out.push(format!("expected one reviewer, found {}", agents.len()));
    }
    for agent in agents {
        for key in ["tools", "caps"] {
            for item in strings(agent, key) {
                if item != "fs_read" {
                    out.push(format!("`{key}` holds `{item}`"));
                }
            }
        }
        let turns = agent
            .get("max_turns")
            .and_then(toml::Value::as_integer)
            .unwrap_or(i64::MAX);
        if turns > 12 {
            out.push(format!("max_turns is {turns}"));
        }
    }
    out
}

/// A temp workdir holding `rubrics/<name>.md` for each pair and a document at `DOC`. Returns it with its canonical path.
fn workdir(rubrics: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = scratch();
    std::fs::create_dir_all(dir.path().join("rubrics")).unwrap();
    for (name, body) in rubrics {
        std::fs::write(dir.path().join(format!("rubrics/{name}.md")), body).unwrap();
    }
    std::fs::create_dir_all(dir.path().join("specs")).unwrap();
    std::fs::write(dir.path().join(DOC), "# Spec: probe\n").unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    (dir, canonical)
}

/// The inputs every review run takes, with `workdir` filled in.
fn base(workdir: &Path) -> serde_json::Value {
    serde_json::json!({
        "doc": DOC,
        "workdir": workdir.display().to_string(),
        "model": "scripted",
        "provider": "scripted",
    })
}

/// The scripted answers, one turn each. Four copies cover every reviewer of a fan-out.
fn passes() -> Vec<&'static str> {
    vec!["VERDICT: PASS"; 4]
}

/// What a run settled as: its status and its journal.
struct Settled {
    status: RunStatus,
    journal: Vec<(u64, RunEvent)>,
}

impl Settled {
    /// A review passes only when the run succeeds and its exit `done` took port `ok` (R4, R5).
    fn passed(&self) -> bool {
        matches!(self.status, RunStatus::Succeeded)
            && self.finished_on("done") == Some(PortName::from("ok"))
    }

    /// The values nodes produced and every failure, without the run input: refusal text is searched here.
    fn text(&self) -> String {
        self.journal
            .iter()
            .filter_map(|(_, ev)| match ev {
                RunEvent::NodeFinished { value, .. } => Some(value.to_string()),
                RunEvent::NodeFailed { error, .. } => Some(error.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The text of every failure: a refusal lives here. A passing slot's echoed inputs are values, not failures.
    fn failures(&self) -> String {
        self.journal
            .iter()
            .filter_map(|(_, ev)| match ev {
                RunEvent::NodeFailed { error, .. } => Some(error.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The port a node finished on, if it finished.
    fn finished_on(&self, id: &str) -> Option<PortName> {
        self.journal.iter().find_map(|(_, ev)| match ev {
            RunEvent::NodeFinished { node, port, .. } if *node == NodeId::from(id) => {
                Some(port.clone())
            }
            _ => None,
        })
    }

    fn started(&self, id: &str) -> bool {
        self.journal.iter().any(|(_, ev)| match ev {
            RunEvent::NodeStarted { node, .. } => *node == NodeId::from(id),
            _ => false,
        })
    }
}

/// Runs `graph_name` through the engine with `answers` as the scripted reviewer turns, and settles it.
async fn run(graph_name: &str, answers: &[&str], input: &serde_json::Value) -> Settled {
    let tmp = scratch();
    let turns: Vec<serde_json::Value> = answers
        .iter()
        .map(|text| {
            serde_json::json!({"text": text, "tool_calls": [], "tokens_in": 1, "tokens_out": 1})
        })
        .collect();
    let fixture = tmp.path().join("fixture.json");
    std::fs::write(&fixture, serde_json::json!({ "turns": turns }).to_string()).unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: ws.clone(),
        store_path: tmp.path().join("store.redb"),
        model: format!("scripted:{}", fixture.display()),
        sandbox: SandboxProfileName::Off,
        route_policy: BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: repo().join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine");
    let run = engine
        .start_run(graph_name, input)
        .unwrap_or_else(|e| panic!("`{graph_name}` starts with its inputs: {e}"));
    for _ in 0..1000 {
        if let Some(header) = engine.store.get_run(&run).unwrap()
            && matches!(header.status, RunStatus::Succeeded | RunStatus::Failed)
        {
            let journal = engine.store.read_journal(&run).unwrap();
            return Settled {
                status: header.status,
                journal,
            };
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run {run} did not settle");
}

/// Runs `review-one` on `rubric` with one reviewer answer, in a workdir holding `critic` and the rubric.
async fn review_one(rubric: &str, answers: &[&str]) -> Settled {
    let (_dir, wd) = workdir(&[("critic", RUBRIC_BODY)]);
    let mut input = base(&wd);
    input["rubric"] = serde_json::json!(rubric);
    run("review-one", answers, &input).await
}

// ---- R4: verdict parsing ----

#[tokio::test]
async fn a_reviewer_that_answers_pass_passes() {
    let settled = review_one("critic", &["VERDICT: PASS"]).await;
    assert!(settled.passed(), "{:?}", settled.text());
}

#[tokio::test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
async fn CONTROL_pass_with_findings_after_it_still_passes() {
    let settled = review_one("critic", &["VERDICT: PASS\n- Nit: wording"]).await;
    assert!(settled.passed(), "{:?}", settled.text());
}

#[tokio::test]
async fn prose_that_contains_pass_is_a_failure() {
    let settled = review_one("critic", &["this does not PASS"]).await;
    assert!(!settled.passed(), "{:?}", settled.text());
}

#[tokio::test]
async fn a_verdict_that_only_starts_like_pass_is_a_failure() {
    let settled = review_one("critic", &["VERDICT: PASSING"]).await;
    assert!(!settled.passed(), "{:?}", settled.text());
}

#[tokio::test]
async fn a_fail_verdict_refuses_with_its_findings() {
    let settled = review_one(
        "critic",
        &["VERDICT: FAIL\n- Blocker R3: the file list is short"],
    )
    .await;
    assert!(!settled.passed(), "{:?}", settled.text());
    assert!(
        settled.text().contains("Blocker R3"),
        "the refusal carries the findings: {:?}",
        settled.text()
    );
}

#[tokio::test]
async fn an_empty_answer_is_a_failure() {
    let settled = review_one("critic", &[""]).await;
    assert!(!settled.passed(), "{:?}", settled.text());
}

/// No scripted turn at all: the provider errors, and the reviewer did not run to an answer.
#[tokio::test]
async fn a_reviewer_that_errors_is_a_failure() {
    let settled = review_one("critic", &[]).await;
    assert!(!settled.passed(), "{:?}", settled.text());
}

// ---- R2: rubric names ----

#[tokio::test]
async fn a_missing_rubric_is_a_failure_naming_it() {
    let settled = review_one("nope", &passes()).await;
    assert!(!settled.passed(), "{:?}", settled.text());
    assert!(settled.text().contains("nope"), "{:?}", settled.text());
}

#[tokio::test]
async fn a_rubric_name_that_leaves_the_directory_is_refused() {
    for name in ["../x", "a/b", ".hidden", ""] {
        let (_dir, wd) = workdir(&[("critic", RUBRIC_BODY)]);
        // A reviewer-shaped file beside `rubrics/`: if the name reached it, it would pass.
        std::fs::write(wd.join("x.md"), "VERDICT: PASS\n").unwrap();
        let mut input = base(&wd);
        input["rubric"] = serde_json::json!(name);
        let settled = run("review-one", &passes(), &input).await;
        assert!(
            !settled.passed(),
            "`{name}` is refused: {:?}",
            settled.text()
        );
        assert!(
            !settled.started("rubric"),
            "`{name}` is refused before anything is read"
        );
        assert!(
            !settled.started("reviewer"),
            "`{name}` never reaches a reviewer"
        );
    }
}

#[tokio::test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
async fn CONTROL_a_plain_rubric_name_is_accepted() {
    let settled = review_one("critic", &["VERDICT: PASS"]).await;
    assert!(settled.started("rubric"), "the rubric file is read");
    assert!(settled.passed(), "{:?}", settled.text());
}

// ---- R3: the reviewer's prompt and its read-only shape ----

#[test]
fn every_reviewer_is_read_only() {
    let violations = read_only_violations(&table(&graph("review-one")));
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
fn CONTROL_the_read_only_check_catches_a_reviewer_with_bash() {
    let mut doc = table(&graph("review-one"));
    let nodes = doc
        .get_mut("node")
        .and_then(toml::Value::as_array_mut)
        .expect("the graph has nodes");
    for node in nodes.iter_mut() {
        let Some(t) = node.as_table_mut() else {
            continue;
        };
        if t.get("kind").and_then(toml::Value::as_str) == Some("agent") {
            let tools = t
                .entry("tools")
                .or_insert_with(|| toml::Value::Array(Vec::new()));
            tools
                .as_array_mut()
                .unwrap()
                .push(toml::Value::String(String::from("bash")));
        }
    }
    assert!(
        !read_only_violations(&doc).is_empty(),
        "a reviewer with bash is reported"
    );
}

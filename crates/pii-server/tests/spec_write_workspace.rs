//! Spec 414: a spec drafted by a graph is written in the daemon's workspace, whatever directory the daemon runs from.
//!
//! Each test runs a one-node graph (`ledger` op `spec.write`) through the engine. The daemon's workspace is `<tmp>/ws`;
//! the process's own directory is the crate, which is never the workspace. The graph's `workdir` is what varies.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{NodeId, PortName, RunEvent, RunStatus};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const FINDING: &str = "The widget renders blank when the list is empty";

/// A spec the planner might draft: the body `spec.write` puts in `specs/`.
const DRAFT: &str =
    "# Spec: the widget renders when empty\n\n## Files\n\n- `crates/pii-server/src/widget.rs`\n";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A one-node graph: `write` is the `spec.write` ledger op, with `workdir` set by `workdir_line` (empty = no key).
fn draft_graph(workdir_line: &str) -> String {
    format!(
        r#"name = "draft-spec"
entry = "write"
exit = "write"
caps = ["fs_write", "ledger"]

[[node]]
id = "write"
kind = "ledger"
op = "spec.write"
caps = ["fs_write", "ledger"]
finding = "$.input.finding"
{workdir_line}
text = "$.input.text"
"#
    )
}

/// What one run of the draft graph left behind.
struct Draft {
    status: RunStatus,
    journal: Vec<(u64, RunEvent)>,
    /// The daemon's workspace, canonical: `<tmp>/ws`.
    ws: PathBuf,
    /// The scratch root that holds the workspace; nothing a run writes outside `ws` lands here.
    tmp: tempfile::TempDir,
}

/// Runs the draft graph with `workdir_line` in a daemon whose workspace is `<tmp>/ws`. `setup` prepares the workspace
/// (for example a symlink out of it) before the run starts.
async fn run_draft(workdir_line: &str, setup: fn(&Path)) -> Draft {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    setup(&ws);
    let graphs = tmp.path().join("graphs");
    std::fs::create_dir_all(&graphs).unwrap();
    std::fs::write(graphs.join("draft-spec.toml"), draft_graph(workdir_line)).unwrap();

    let engine: Arc<Engine> = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: ws.clone(),
        store_path: tmp.path().join("store.redb"),
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
    let ws = std::fs::canonicalize(&ws).unwrap();
    let input = serde_json::json!({
        "finding": FINDING,
        "text": DRAFT,
        "workdir": ws,
    });
    let run = engine
        .start_run("draft-spec", &input)
        .expect("draft-spec starts with its inputs");
    for _ in 0..500 {
        if let Some(header) = engine.store.get_run(&run).unwrap()
            && matches!(header.status, RunStatus::Succeeded | RunStatus::Failed)
        {
            let journal = engine.store.read_journal(&run).unwrap();
            return Draft {
                status: header.status,
                journal,
                ws,
                tmp,
            };
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run {run} did not settle");
}

fn no_setup(_ws: &Path) {}

/// The port and value a node finished with, if it finished.
fn finished(journal: &[(u64, RunEvent)], id: &str) -> Option<(PortName, serde_json::Value)> {
    journal.iter().find_map(|(_, ev)| match ev {
        RunEvent::NodeFinished {
            node, port, value, ..
        } if *node == NodeId::from(id) => Some((port.clone(), value.clone())),
        _ => None,
    })
}

/// The `specs/` entries in `dir`, sorted; empty when there is no `specs/`.
fn spec_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("specs")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// R1: a relative `workdir` is the daemon's workspace, whatever directory the process runs from.
#[tokio::test]
async fn a_relative_workdir_is_the_workspace() {
    let draft = run_draft(r#"workdir = ".""#, no_setup).await;
    let (port, value) = finished(&draft.journal, "write")
        .unwrap_or_else(|| panic!("write never finished: {:?}", draft.journal));
    assert_eq!(port, PortName::from("ok"), "{value}");
    assert!(
        matches!(draft.status, RunStatus::Succeeded),
        "{:?}",
        draft.status
    );
    let spec = value["spec"].as_str().unwrap();
    assert!(spec.starts_with("specs/1-"), "{spec}");
    assert_eq!(
        spec_files(&draft.ws).len(),
        1,
        "{:?}",
        spec_files(&draft.ws)
    );
    assert_eq!(
        std::fs::read_to_string(draft.ws.join(spec)).unwrap(),
        DRAFT,
        "the spec lands in the workspace, unedited"
    );
}

/// R2: an absolute `workdir` is used as given, as it is today.
#[tokio::test]
async fn an_absolute_workdir_is_used_as_given() {
    let draft = run_draft(r#"workdir = "$.input.workdir""#, no_setup).await;
    let (port, value) = finished(&draft.journal, "write")
        .unwrap_or_else(|| panic!("write never finished: {:?}", draft.journal));
    assert_eq!(port, PortName::from("ok"), "{value}");
    assert!(
        matches!(draft.status, RunStatus::Succeeded),
        "{:?}",
        draft.status
    );
    let spec = value["spec"].as_str().unwrap();
    assert!(spec.starts_with("specs/1-"), "{spec}");
    assert!(draft.ws.join(spec).is_file(), "{spec} under the workspace");
    assert_eq!(
        spec_files(&draft.ws).len(),
        1,
        "{:?}",
        spec_files(&draft.ws)
    );
}

/// R2: an absolute `workdir` OUTSIDE the workspace is used as given, not refused as leaving it: the run's `fs_write`
/// scope decides, so the write is denied there and nothing is created. (An absolute path INSIDE the workspace cannot
/// tell "used as given" from "joined onto the workspace", because `Path::join` returns an absolute argument unchanged.)
#[tokio::test]
async fn an_absolute_workdir_outside_the_workspace_is_left_to_the_scope_not_refused_as_leaving_it() {
    let outside = tempfile::tempdir().unwrap();
    let line = format!(r#"workdir = "{}""#, outside.path().display());
    let draft = run_draft(&line, no_setup).await;
    let seen = format!("{:?}", draft.journal);
    assert!(
        !seen.contains("leaves the workspace"),
        "an absolute path is not this node's refusal: {seen}"
    );
    assert!(
        matches!(draft.status, RunStatus::Failed),
        "the scope denies the write: {:?}",
        draft.status
    );
    assert!(
        spec_files(outside.path()).is_empty(),
        "nothing is written outside the workspace: {:?}",
        spec_files(outside.path())
    );
}

/// R3: a relative `workdir` that leaves the workspace is refused, naming the path, and nothing is written.
#[tokio::test]
#[allow(non_snake_case)] // reason: the name is the spec's contract (a CONTROL test)
async fn CONTROL_a_relative_workdir_that_leaves_the_workspace_is_refused() {
    let draft = run_draft(r#"workdir = "..""#, no_setup).await;
    let (port, value) = finished(&draft.journal, "write")
        .unwrap_or_else(|| panic!("write never finished: {:?}", draft.journal));
    assert_eq!(port, PortName::from("err"), "{value}");
    let reason = value["reason"].as_str().unwrap_or_default();
    assert!(reason.contains(".."), "the reason names the path: {reason}");
    assert!(
        !draft.tmp.path().join("specs").exists(),
        "nothing written outside the workspace"
    );
}

/// R3: a symlink out of the workspace is refused, and nothing is written through it.
#[tokio::test]
#[allow(non_snake_case)] // reason: the name is the spec's contract (a CONTROL test)
async fn CONTROL_a_symlink_out_of_the_workspace_is_refused() {
    fn link_out(ws: &Path) {
        let outside = ws.parent().unwrap().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("out")).unwrap();
    }
    let draft = run_draft(r#"workdir = "out""#, link_out).await;
    let (port, value) = finished(&draft.journal, "write")
        .unwrap_or_else(|| panic!("write never finished: {:?}", draft.journal));
    assert_eq!(port, PortName::from("err"), "{value}");
    let reason = value["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("out"),
        "the reason names the path: {reason}"
    );
    let outside = draft.tmp.path().join("outside");
    assert!(
        spec_files(&outside).is_empty(),
        "nothing written under the link's target: {:?}",
        spec_files(&outside)
    );
}

/// R4: a missing `workdir` still refuses with the existing reason.
#[tokio::test]
async fn a_missing_workdir_still_refuses_with_the_existing_reason() {
    let draft = run_draft("", no_setup).await;
    let (port, value) = finished(&draft.journal, "write")
        .unwrap_or_else(|| panic!("write never finished: {:?}", draft.journal));
    assert_eq!(port, PortName::from("err"), "{value}");
    assert_eq!(value["reason"], "plan needs a workdir", "{value}");
    assert_eq!(spec_files(&draft.ws), Vec::<String>::new());
}

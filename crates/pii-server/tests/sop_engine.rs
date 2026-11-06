//! Spec 113: the worked example SOP runs end to end on the real engine
//! registry with the scripted model — suspends at the approval, resumes after
//! `approve`, and a failing `check` ends the run naming the step.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::{RunEvent, RunId, RunStatus, SuspendReason, Terminal};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx, PathScope};
use pii_observe::{Recorder, names};
use pii_server::daemon::approvals;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

struct Rig {
    engine: Arc<Engine>,
    log: Arc<Recorder>,
    ws: PathBuf,
    _tmp: tempfile::TempDir,
}

fn rig(prefix: &str) -> Rig {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("Cargo.toml"), "[package]\nversion = \"0.1.0\"\n").unwrap();
    let model = root().join("fixtures/sop/cut-release-model.json");
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: ws.clone(),
        store_path: tmp.path().join("store.redb"),
        model: format!("scripted:{}", model.display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: tmp.path().join("graphs"),
        mqtt_tool_url: None,
    })
    .unwrap();
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    Rig {
        engine,
        log,
        ws,
        _tmp: tmp,
    }
}

fn run_ctx(ws: &Path) -> Ctx {
    let caps = CapSet {
        kinds: CapKind::FS_READ | CapKind::FS_WRITE | CapKind::PROC,
        fs_read: PathScope::new(vec![ws.to_path_buf()]),
        fs_write: PathScope::new(vec![ws.join("_tmp")]),
        proc_argv0: ArgvScope::new(vec![String::from("/bin/sh")]),
        ..CapSet::default()
    };
    Ctx::root(caps, Budget::unlimited())
}

/// The example SOP, compiled, optionally with the check command replaced.
fn compiled(ws: &Path, check_cmd: Option<&str>) -> String {
    let mut src = std::fs::read_to_string(root().join("examples/cut-release.sop.toml")).unwrap();
    if let Some(cmd) = check_cmd {
        src = src.replace("test -f Cargo.toml && grep -q '^version' Cargo.toml", cmd);
    }
    // The example's notify path is relative to the process cwd; the resumed
    // run is workspace-scoped, so point it at the workspace explicitly.
    let abs = ws.join("_tmp/sop-cut-release-notify.log");
    src = src.replace(
        "_tmp/sop-cut-release-notify.log",
        &abs.display().to_string(),
    );
    pii_graph::sop::compile(&pii_graph::sop::load(&src).unwrap())
}

fn start(rig: &Rig, graph_toml: &str) -> (RunId, Terminal) {
    let spec = pii_graph::load_toml_with_registry(graph_toml, &rig.engine.reg).unwrap();
    let warnings = pii_graph::validate(&spec, &rig.engine.reg).unwrap();
    assert!(
        warnings.len() <= 1,
        "unexpected lint warnings: {warnings:?}"
    );
    let run_id = rig.engine.store.next_run_id().unwrap();
    let term = pii_graph::run::run(
        &rig.engine.store,
        &spec,
        &rig.engine.reg,
        &run_ctx(&rig.ws),
        &run_id,
        serde_json::json!({}),
    )
    .unwrap();
    (run_id, term)
}

fn events(rig: &Rig, kind: &str) -> Vec<(String, String)> {
    let field = |e: &pii_observe::OwnedEvent, k: &str| {
        e.fields
            .get(k)
            .map(|v| v.to_string().trim_matches('"').to_owned())
            .unwrap_or_default()
    };
    rig.log
        .events_named(kind)
        .iter()
        .map(|e| (field(e, "sop"), field(e, "step")))
        .collect()
}

async fn wait_status(rig: &Rig, run: &RunId, want: RunStatus) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let st = rig.engine.store.get_run(run).unwrap().map(|h| h.status);
        if st == Some(want) {
            return;
        }
        let failures: Vec<_> = rig
            .engine
            .store
            .read_journal(run)
            .unwrap()
            .into_iter()
            .filter(|(_, e)| matches!(e, RunEvent::NodeFailed { .. }))
            .collect();
        assert!(
            Instant::now() < end && !(st == Some(RunStatus::Failed) && want != RunStatus::Failed),
            "run at {st:?}, wanted {want:?}: {failures:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn suspend_of(rig: &Rig, run: &RunId) -> (String, String, String, String) {
    rig.engine
        .store
        .read_journal(run)
        .unwrap()
        .into_iter()
        .rev()
        .find_map(|(_, e)| match e {
            RunEvent::Suspended {
                node,
                reason: SuspendReason::Approval { policy, prompt },
                key,
            } => Some((node.to_string(), policy, prompt, key.0)),
            _ => None,
        })
        .expect("approval suspend in the journal")
}

fn step(s: &str) -> (String, String) {
    (String::from("cut-release"), s.to_owned())
}

/// R6/R4/R5: suspend at the approval, then resume to completion after allow.
#[tokio::test(flavor = "multi_thread")]
async fn example_suspends_at_the_approval_then_resumes_to_completion() {
    let rig = rig("sop-e2e-");
    let (run, term) = start(&rig, &compiled(&rig.ws, None));
    let Terminal::Suspended { key } = term else {
        panic!("expected a suspend at the approval, got {term:?}");
    };
    // Steps before the approval ran, in order; nothing after it did.
    assert_eq!(
        events(&rig, names::SOP_STEP_STARTED),
        vec![
            step("prepare"),
            step("manifest-present"),
            step("approve-tag")
        ]
    );
    assert_eq!(
        events(&rig, names::SOP_STEP_FINISHED),
        vec![step("prepare"), step("manifest-present")]
    );
    assert_eq!(
        events(&rig, names::SOP_STEP_AWAITING_APPROVAL),
        vec![step("approve-tag")]
    );
    // The awaiting event names the very approval id the gate suspended on.
    let id = key.0.strip_prefix("approval:").unwrap().to_owned();
    let awaiting = &rig.log.events_named(names::SOP_STEP_AWAITING_APPROVAL)[0];
    assert_eq!(
        awaiting.fields.get("approval").map(ToString::to_string),
        Some(format!("\"{id}\""))
    );

    // The approval a human sees says WHAT is being approved (R4).
    let (node, policy, prompt, jkey) = suspend_of(&rig, &run);
    assert_eq!(jkey, key.0);
    approvals::request(&rig.engine, &id, run.as_str(), &node, &policy, &prompt).unwrap();
    let pending = rig.engine.approvals.pending_approvals().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].node, "approve-tag");
    assert!(pending[0].prompt.contains("cut-release") && pending[0].prompt.contains("approve-tag"));
    // Idempotent across a lag rescan (spec 88): a second request changes nothing.
    approvals::request(&rig.engine, &id, run.as_str(), &node, &policy, &prompt).unwrap();
    assert_eq!(rig.engine.approvals.pending_approvals().unwrap().len(), 1);

    assert!(
        approvals::resolve(&rig.engine, &id, true, None)
            .unwrap()
            .resumed
    );
    wait_status(&rig, &run, RunStatus::Succeeded).await;
    assert_eq!(
        events(&rig, names::SOP_STEP_FINISHED),
        vec![
            step("prepare"),
            step("manifest-present"),
            step("approve-tag"),
            step("publish"),
            step("announce")
        ]
    );
    assert_eq!(
        events(&rig, names::SOP_STEP_FAILED),
        [] as [(String, String); 0]
    );
    let notified = std::fs::read_to_string(rig.ws.join("_tmp/sop-cut-release-notify.log")).unwrap();
    assert_eq!(notified, "release cut and published\n");
}

/// R3: a failing `check` ends the run naming the step; nothing after it runs.
#[test]
fn a_failing_check_ends_the_run_naming_the_step() {
    let rig = rig("sop-fail-");
    let (run, term) = start(&rig, &compiled(&rig.ws, Some("exit 3")));
    let Terminal::Failure { error } = term else {
        panic!("expected a named failure, got {term:?}");
    };
    assert!(
        error.contains("cut-release")
            && error.contains("manifest-present")
            && error.contains("exit code 3"),
        "{error}"
    );
    assert_eq!(
        events(&rig, names::SOP_STEP_FAILED),
        vec![step("manifest-present")]
    );
    assert!(!events(&rig, names::SOP_STEP_STARTED).contains(&step("approve-tag")));
    assert_eq!(
        rig.engine.approvals.pending_approvals().unwrap(),
        [] as [pii_store::approval::Approval; 0]
    );
    let status = rig.engine.store.get_run(&run).unwrap().unwrap().status;
    assert_eq!(status, RunStatus::Failed);
}

/// A human `deny` is a named failure too, not a silent stop.
#[tokio::test(flavor = "multi_thread")]
async fn a_denied_approval_fails_the_run_naming_the_step() {
    let rig = rig("sop-deny-");
    let (run, term) = start(&rig, &compiled(&rig.ws, None));
    let Terminal::Suspended { key } = term else {
        panic!("expected a suspend, got {term:?}");
    };
    let id = key.0.strip_prefix("approval:").unwrap().to_owned();
    let (node, policy, prompt, _) = suspend_of(&rig, &run);
    approvals::request(&rig.engine, &id, run.as_str(), &node, &policy, &prompt).unwrap();
    approvals::resolve(&rig.engine, &id, false, None).unwrap();
    wait_status(&rig, &run, RunStatus::Failed).await;
    assert_eq!(
        events(&rig, names::SOP_STEP_FAILED),
        vec![step("approve-tag")]
    );
    assert!(!events(&rig, names::SOP_STEP_STARTED).contains(&step("publish")));
}

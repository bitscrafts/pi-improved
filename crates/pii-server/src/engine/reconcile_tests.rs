//! Spec 154: an accepted run is on disk before its first node runs, and a boot
//! reconcile of a run that died mid-node is loud, terminal and never resumed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{
    Effects, NodeInput, Outcome, PiiError, PortName, RunEvent, RunHeader, RunId, RunStatus,
    Terminal,
};
use pii_core::{CapKind, Ctx, ObsEvent, Observer, RecordingObserver};
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_store::Store;

fn engine(prefix: &str) -> (tempfile::TempDir, Arc<crate::Engine>) {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).unwrap();
    let engine = crate::Engine::new(crate::EngineConfig {
        grant_ceiling: None,
        workspace: dir.path().to_path_buf(),
        store_path: dir.path().join("store.redb"),
        model: format!(
            "scripted:{}/../../fixtures/phase2/hello.json",
            env!("CARGO_MANIFEST_DIR")
        ),
        sandbox: pii_tools::sandbox::SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.path().join("graphs"),
        mqtt_tool_url: None,
    })
    .unwrap();
    (dir, engine)
}

/// What the store held for the run at the moment the first node executed.
type Seen = Arc<Mutex<Option<(RunStatus, usize)>>>;

struct ProbeFactory {
    store: Arc<Store>,
    seen: Seen,
}

impl NodeFactory for ProbeFactory {
    fn kind(&self) -> &'static str {
        "probe"
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }
    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(Probe {
            store: Arc::clone(&self.store),
            seen: Arc::clone(&self.seen),
        }))
    }
}

struct Probe {
    store: Arc<Store>,
    seen: Seen,
}

impl GraphNode for Probe {
    fn run(&self, ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        let run = ctx.trace().run;
        let header = self.store.get_run(&run)?;
        let events = self.store.read_journal(&run)?;
        *self.seen.lock() = header.map(|h| (h.status, events.len()));
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!(1),
        })
    }
}

fn seed_running(engine: &crate::Engine, id: &str) -> RunHeader {
    let mut header = RunHeader {
        run: RunId::from(id),
        graph: String::from("default"),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    };
    let started = RunEvent::Started {
        graph: String::from("default"),
        input_hash: String::from("h"),
    };
    let node = RunEvent::NodeStarted {
        node: pii_abi::NodeId::from("slow"),
        at_ms: 0,
    };
    engine
        .store
        .append_events(&mut header, &[&started, &node])
        .unwrap();
    header
}

#[test]
fn an_accepted_run_is_readable_before_its_first_node() {
    let (_dir, engine) = engine("accepted-");
    let seen: Seen = Arc::new(Mutex::new(None));
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(ProbeFactory {
        store: Arc::clone(&engine.store),
        seen: Arc::clone(&seen),
    }));
    let spec = pii_graph::toml_load::load_toml(
        "name = \"t\"\nentry = \"a\"\n[[node]]\nid = \"a\"\nkind = \"probe\"\n",
    )
    .unwrap();
    let run = RunId::from("r154");
    let ctx = Ctx::with_observer(
        pii_core::CapSet::default(),
        pii_core::Budget::unlimited(),
        Arc::new(pii_core::NullObserver),
        pii_core::TraceCtx {
            run: run.clone(),
            span: 0,
        },
    );
    pii_graph::run::run(
        &engine.store,
        &spec,
        &reg,
        &ctx,
        &run,
        serde_json::json!({}),
    )
    .unwrap();
    let (status, events) = seen.lock().expect("the probe node ran");
    assert_eq!(status, RunStatus::Running, "header is on disk, Running");
    assert!(
        events >= 1,
        "Started is on disk before the first node: {events}"
    );
}

#[test]
fn a_reconcile_emits_one_event() {
    let (_dir, engine) = engine("one-event-");
    let rec = Arc::new(RecordingObserver::new());
    engine.attach_observer(rec.clone() as Arc<dyn Observer>);
    seed_running(&engine, "r1");
    engine.reconcile_boot();
    let evs: Vec<_> = rec
        .events()
        .into_iter()
        .filter(|e| matches!(e, ObsEvent::RunReconciled { .. }))
        .collect();
    assert!(
        matches!(evs.as_slice(), [ObsEvent::RunReconciled { run, reason }]
            if run == "r1" && reason == "interrupted by daemon restart"),
        "exactly one reconcile event naming the run: {evs:?}"
    );
    let header = engine.store.get_run(&RunId::from("r1")).unwrap().unwrap();
    assert_eq!(header.status, RunStatus::Failed);
}

#[test]
fn a_write_error_is_not_swallowed() {
    let (_dir, engine) = engine("write-error-");
    let rec = Arc::new(RecordingObserver::new());
    engine.attach_observer(rec.clone() as Arc<dyn Observer>);
    seed_running(&engine, "r1");
    engine.store.set_fail_append_events_for_test(1);
    engine.reconcile_boot();
    let evs = rec.events();
    assert!(
        evs.iter()
            .any(|e| matches!(e, ObsEvent::PersistError { context, .. }
            if context.contains("reconcile") && context.contains("r1"))),
        "the refused write is surfaced: {evs:?}"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, ObsEvent::RunReconciled { .. })),
        "a run that was not reconciled is not reported as reconciled"
    );
}

#[test]
fn nothing_is_auto_resumed() {
    let (_dir, engine) = engine("no-resume-");
    let before = seed_running(&engine, "r1").seq;
    engine.reconcile_boot();
    let run = RunId::from("r1");
    let header = engine.store.get_run(&run).unwrap().unwrap();
    assert_eq!(header.status, RunStatus::Failed, "terminal");
    let journal = engine.store.read_journal(&run).unwrap();
    let after: Vec<_> = journal.iter().filter(|(s, _)| *s >= before).collect();
    assert!(
        matches!(after.as_slice(), [(_, RunEvent::Finished {
            terminal: Terminal::Failure { error }, ..
        })] if error == "interrupted by daemon restart"),
        "only the terminal event was added; no node ran: {after:?}"
    );
}

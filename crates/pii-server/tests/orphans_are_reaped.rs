//! Spec 242: a restart finds the agents a dead daemon left running, kills them, and says so on the run.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{NodeId, PiiError, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_core::{ArgvScope, CapKind, CapSet, ChildProc, Ctx};
use pii_server::engine::orphans::{ProcFs, ProcProbe, reap_orphans};
use pii_server::node_kinds::nodes::coder_runtime::{
    ContainerRuntime, Launch, Listed, Request, Spawn,
};
use pii_server::{Engine, EngineConfig};
use pii_store::Store;

const REASON: &str = "interrupted by daemon restart";
const REAPED: &str = "interrupted by daemon restart; agent reaped";

/// A runtime that holds a fixed list of containers and records every removal and every list call.
#[derive(Default)]
struct FakeRuntime {
    containers: Vec<Listed>,
    fail_remove: Option<String>,
    removed: RefCell<Vec<String>>,
    lists: RefCell<usize>,
}

impl ContainerRuntime for FakeRuntime {
    fn name(&self) -> &'static str {
        "cli"
    }

    fn run(
        &self,
        _ctx: &Ctx,
        _req: &Request<'_>,
        _timeout: Option<Duration>,
        _spawned: &mut dyn FnMut(&Spawn),
    ) -> Result<Launch, PiiError> {
        Err(PiiError::Internal(String::from("the fake does not run")))
    }

    fn list_all(&self, _ctx: &Ctx) -> Result<Vec<Listed>, PiiError> {
        *self.lists.borrow_mut() += 1;
        Ok(self.containers.clone())
    }

    fn force_remove(&self, _ctx: &Ctx, id: &str) -> Result<bool, PiiError> {
        if self.fail_remove.as_deref() == Some(id) {
            return Err(PiiError::Internal(format!("podman rm {id}: device busy")));
        }
        self.removed.borrow_mut().push(id.to_owned());
        Ok(true)
    }
}

/// A process probe over a fixed pid → cmdline map. Kills are recorded, never sent.
#[derive(Default)]
struct FakeProc {
    cmdlines: Vec<(u32, String)>,
    killed: RefCell<Vec<u32>>,
}

impl ProcProbe for FakeProc {
    fn cmdline(&self, pid: u32) -> Option<String> {
        self.cmdlines
            .iter()
            .find(|(p, _)| *p == pid)
            .map(|(_, c)| c.clone())
    }

    fn kill_group(&self, pid: u32) -> Result<(), PiiError> {
        self.killed.borrow_mut().push(pid);
        Ok(())
    }
}

fn container(id: &str, run: &str) -> Listed {
    Listed {
        id: id.to_owned(),
        labels: vec![
            (String::from("io.pii"), String::from("1")),
            (String::from("pii.run"), run.to_owned()),
        ],
    }
}

fn ctx() -> Ctx {
    let caps = CapSet {
        kinds: CapKind::PROC,
        ..CapSet::default()
    };
    Ctx::with_observer(
        caps,
        pii_core::Budget::unlimited(),
        Arc::new(pii_core::NullObserver),
        pii_core::TraceCtx {
            run: RunId::from("boot"),
            span: 0,
        },
    )
}

/// A context allowed to spawn `sleep` and nothing else: the one real process the test starts.
fn spawn_ctx() -> Ctx {
    let caps = CapSet {
        kinds: CapKind::PROC | CapKind::SPAWN,
        proc_argv0: ArgvScope::new(vec![String::from("sleep")]),
        ..CapSet::default()
    };
    Ctx::with_observer(
        caps,
        pii_core::Budget::unlimited(),
        Arc::new(pii_core::NullObserver),
        pii_core::TraceCtx {
            run: RunId::from("sleeper"),
            span: 0,
        },
    )
}

fn engine(prefix: &str) -> (tempfile::TempDir, Arc<Engine>) {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).unwrap();
    let engine = Engine::new(EngineConfig {
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
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.path().join("graphs"),
    })
    .unwrap();
    (dir, engine)
}

/// A run in `status` whose journal holds `events`, written through the store the engine reads at boot.
fn seed(store: &Store, run: &str, status: RunStatus, events: &[RunEvent]) {
    let mut header = RunHeader {
        run: RunId::from(run),
        graph: String::from("default"),
        status,
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
    let mut all: Vec<&RunEvent> = vec![&started];
    all.extend(events.iter());
    store.append_events(&mut header, &all).unwrap();
}

fn spawned(runtime: &str, pid: u32, argv0: &str, container: &str) -> RunEvent {
    observed(
        "coder.spawned",
        serde_json::json!({
            "runtime": runtime,
            "pid": pid,
            "argv0": argv0,
            "container": container,
            "session": "",
        }),
    )
}

fn exited(pid: u32) -> RunEvent {
    observed(
        "coder.exited",
        serde_json::json!({ "pid": pid, "rc": 0, "dur_ms": 1, "stop_reason": "exit" }),
    )
}

fn observed(kind: &str, fields: serde_json::Value) -> RunEvent {
    RunEvent::NodeObserved {
        node: NodeId::from("coder"),
        kind: kind.to_owned(),
        fields,
    }
}

/// The `coder.orphan_reaped` events journalled on `run`, as their fields.
fn reaped_events(store: &Store, run: &str) -> Vec<serde_json::Value> {
    store
        .read_journal(&RunId::from(run))
        .unwrap()
        .into_iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::NodeObserved { kind, fields, .. } if kind == "coder.orphan_reaped" => {
                Some(fields)
            }
            _ => None,
        })
        .collect()
}

/// The error the run's terminal `Finished` event carries, if it has one.
fn failure_text(store: &Store, run: &str) -> Option<String> {
    store
        .read_journal(&RunId::from(run))
        .unwrap()
        .into_iter()
        .rev()
        .find_map(|(_, ev)| match ev {
            RunEvent::Finished {
                terminal: Terminal::Failure { error },
                ..
            } => Some(error),
            _ => None,
        })
}

fn status_of(store: &Store, run: &str) -> RunStatus {
    store.get_run(&RunId::from(run)).unwrap().unwrap().status
}

#[test]
fn a_running_runs_container_is_removed() {
    let (_dir, engine) = engine("orphan-container-");
    seed(
        &engine.store,
        "r1",
        RunStatus::Running,
        &[spawned("cli", 0, "podman", "c1")],
    );
    let runtime = FakeRuntime {
        containers: vec![container("c1", "r1")],
        ..FakeRuntime::default()
    };
    let proc = FakeProc::default();

    engine.recover_boot(&ctx(), &runtime, &proc);

    assert_eq!(*runtime.removed.borrow(), vec![String::from("c1")]);
    let events = reaped_events(&engine.store, "r1");
    assert_eq!(events.len(), 1);
    assert_eq!(events.first().unwrap()["kind"], "container");
    assert_eq!(events.first().unwrap()["id"], "c1");
    assert_eq!(events.first().unwrap()["result"], "killed");
}

#[test]
fn a_container_of_another_run_is_left() {
    // CONTROL: r1 is Running and has its own container, so the runtime is searched; c9 belongs to no Running run.
    let (_dir, engine) = engine("orphan-other-run-");
    seed(
        &engine.store,
        "r1",
        RunStatus::Running,
        &[spawned("cli", 0, "podman", "c1")],
    );
    let runtime = FakeRuntime {
        containers: vec![container("c1", "r1"), container("c9", "r9")],
        ..FakeRuntime::default()
    };

    engine.recover_boot(&ctx(), &runtime, &FakeProc::default());

    assert_eq!(*runtime.removed.borrow(), vec![String::from("c1")]);
}

#[test]
fn a_container_without_the_pii_label_is_left() {
    // CONTROL: carries the run's label but not io.pii=1, so it is not ours to remove.
    let (_dir, engine) = engine("orphan-no-label-");
    seed(
        &engine.store,
        "r1",
        RunStatus::Running,
        &[spawned("cli", 0, "podman", "c1")],
    );
    let stranger = Listed {
        id: String::from("c2"),
        labels: vec![(String::from("pii.run"), String::from("r1"))],
    };
    let runtime = FakeRuntime {
        containers: vec![container("c1", "r1"), stranger],
        ..FakeRuntime::default()
    };

    engine.recover_boot(&ctx(), &runtime, &FakeProc::default());

    assert_eq!(*runtime.removed.borrow(), vec![String::from("c1")]);
}

#[test]
fn a_live_host_agent_is_killed() {
    let (_dir, engine) = engine("orphan-host-");
    seed(
        &engine.store,
        "r1",
        RunStatus::Running,
        &[spawned("host", 4242, "claude", "")],
    );
    let proc = FakeProc {
        cmdlines: vec![(4242, String::from("claude --print"))],
        ..FakeProc::default()
    };

    engine.recover_boot(&ctx(), &FakeRuntime::default(), &proc);

    assert_eq!(*proc.killed.borrow(), vec![4242]);
    let events = reaped_events(&engine.store, "r1");
    assert_eq!(events.len(), 1);
    assert_eq!(events.first().unwrap()["kind"], "process");
    assert_eq!(events.first().unwrap()["id"], "4242");
    assert_eq!(events.first().unwrap()["result"], "killed");
}

#[test]
fn a_recycled_pid_is_not_killed() {
    // CONTROL: pid 4242 now belongs to sshd, so the recorded agent is gone and nothing is signalled.
    let (_dir, engine) = engine("orphan-recycled-");
    seed(
        &engine.store,
        "r1",
        RunStatus::Running,
        &[spawned("host", 4242, "claude", "")],
    );
    let proc = FakeProc {
        cmdlines: vec![(4242, String::from("sshd -D"))],
        ..FakeProc::default()
    };

    engine.recover_boot(&ctx(), &FakeRuntime::default(), &proc);

    assert!(proc.killed.borrow().is_empty());
    let events = reaped_events(&engine.store, "r1");
    assert_eq!(events.len(), 1);
    assert_eq!(events.first().unwrap()["result"], "already_gone");
}

#[test]
fn a_run_with_coder_exited_is_not_searched() {
    // CONTROL: the agent exited and the node journalled it, so there is nothing to reap.
    let (_dir, engine) = engine("orphan-exited-");
    seed(
        &engine.store,
        "r1",
        RunStatus::Running,
        &[spawned("cli", 0, "podman", "c1"), exited(0)],
    );
    let runtime = FakeRuntime {
        containers: vec![container("c1", "r1")],
        ..FakeRuntime::default()
    };

    engine.recover_boot(&ctx(), &runtime, &FakeProc::default());

    assert_eq!(*runtime.lists.borrow(), 0);
    assert!(runtime.removed.borrow().is_empty());
    assert!(reaped_events(&engine.store, "r1").is_empty());
}

#[test]
fn a_run_that_never_reached_a_coder_is_free() {
    // CONTROL: no coder.* event at all, so the runtime is never asked.
    let (_dir, engine) = engine("orphan-free-");
    seed(&engine.store, "r1", RunStatus::Running, &[]);
    let runtime = FakeRuntime::default();

    engine.recover_boot(&ctx(), &runtime, &FakeProc::default());

    assert_eq!(*runtime.lists.borrow(), 0);
    assert_eq!(status_of(&engine.store, "r1"), RunStatus::Failed);
}

#[test]
fn a_failed_reap_does_not_stop_boot() {
    let (_dir, engine) = engine("orphan-failed-");
    seed(
        &engine.store,
        "r1",
        RunStatus::Running,
        &[spawned("cli", 0, "podman", "c1")],
    );
    seed(
        &engine.store,
        "r2",
        RunStatus::Running,
        &[spawned("cli", 0, "podman", "c2")],
    );
    let runtime = FakeRuntime {
        containers: vec![container("c1", "r1"), container("c2", "r2")],
        fail_remove: Some(String::from("c1")),
        ..FakeRuntime::default()
    };

    engine.recover_boot(&ctx(), &runtime, &FakeProc::default());

    let failed = reaped_events(&engine.store, "r1");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed.first().unwrap()["result"], "failed");
    assert_eq!(*runtime.removed.borrow(), vec![String::from("c2")]);
    assert_eq!(status_of(&engine.store, "r1"), RunStatus::Failed);
    assert_eq!(status_of(&engine.store, "r2"), RunStatus::Failed);
    assert_eq!(failure_text(&engine.store, "r2").as_deref(), Some(REAPED));
}

#[test]
fn the_failure_text_says_the_agent_was_reaped() {
    let (_dir, engine) = engine("orphan-text-");
    seed(
        &engine.store,
        "r1",
        RunStatus::Running,
        &[spawned("cli", 0, "podman", "c1")],
    );
    let runtime = FakeRuntime {
        containers: vec![container("c1", "r1")],
        ..FakeRuntime::default()
    };

    engine.recover_boot(&ctx(), &runtime, &FakeProc::default());

    assert_eq!(failure_text(&engine.store, "r1").as_deref(), Some(REAPED));
}

#[test]
fn the_failure_text_is_unchanged_without_an_agent() {
    // CONTROL: nothing was killed, so the text is the one spec 16 has always written.
    let (_dir, engine) = engine("orphan-plain-");
    seed(&engine.store, "r1", RunStatus::Running, &[]);

    engine.recover_boot(&ctx(), &FakeRuntime::default(), &FakeProc::default());

    assert_eq!(failure_text(&engine.store, "r1").as_deref(), Some(REASON));
}

#[test]
fn a_second_boot_reaps_nothing() {
    let (_dir, engine) = engine("orphan-twice-");
    seed(
        &engine.store,
        "r1",
        RunStatus::Running,
        &[spawned("cli", 0, "podman", "c1")],
    );
    let runtime = FakeRuntime {
        containers: vec![container("c1", "r1")],
        ..FakeRuntime::default()
    };
    engine.recover_boot(&ctx(), &runtime, &FakeProc::default());
    let before = reaped_events(&engine.store, "r1").len();
    let removed_before = runtime.removed.borrow().len();

    engine.recover_boot(&ctx(), &runtime, &FakeProc::default());

    assert_eq!(runtime.removed.borrow().len(), removed_before);
    assert_eq!(reaped_events(&engine.store, "r1").len(), before);
}

/// A real `sleep` in its own process group, started through the capability and killed by the test if an assertion
/// fails before it is reaped.
struct Sleeper {
    child: Option<ChildProc>,
}

impl Sleeper {
    fn start(ctx: &Ctx) -> Self {
        let argv = [String::from("sleep"), String::from("100000")];
        let child = ctx
            .proc()
            .unwrap()
            .spawn_child_exact(&argv, Path::new("/"), &[])
            .unwrap();
        Self { child: Some(child) }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().pid()
    }

    /// Waits for the process to end, up to `within`. True when it did.
    fn ended_within(&mut self, within: Duration) -> bool {
        let child = self.child.as_mut().unwrap();
        matches!(child.wait_timeout(within), Ok(Some(_)))
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
    }
}

#[test]
fn a_real_orphan_process_is_killed() {
    let (_dir, engine) = engine("orphan-real-");
    let mut agent = Sleeper::start(&spawn_ctx());
    let mut bystander = Sleeper::start(&spawn_ctx());
    seed(
        &engine.store,
        "r1",
        RunStatus::Running,
        &[spawned("host", agent.pid(), "sleep", "")],
    );

    reap_orphans(&engine.store, &ctx(), &FakeRuntime::default(), &ProcFs);

    assert!(
        agent.ended_within(Duration::from_secs(2)),
        "the recorded agent survived its reap"
    );
    assert!(
        !bystander.ended_within(Duration::from_millis(200)),
        "a process nobody recorded was killed"
    );
}

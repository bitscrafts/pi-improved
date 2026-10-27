//! Spec 292: the reaper is proven on a spawn a live run journalled, not one a test built by hand.
//!
//! The write side is `coder.rs::journal_spawned`, reached by a run's own coder through `Engine::start_run`. The read side
//! is `engine/orphans.rs::reap_orphans`, which boot's recovery calls. Every spawn these tests reap is the one the run
//! wrote to its store: nothing here constructs a `coder.spawned` event. The seeded suite, `orphans_are_reaped.rs`, cannot
//! see a break in the write (`_docs/05-operations/LIVE-SPAWN-REAP.md`).

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod spec_graph_daemon;

use std::cell::RefCell;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{PiiError, RunEvent, RunHeader, RunId, RunStatus};
use pii_core::{Budget, CapKind, CapSet, Ctx, NullObserver, TraceCtx};
use pii_server::Engine;
use pii_server::engine::orphans::{ProcFs, ProcProbe, reap_orphans};
use pii_server::node_kinds::nodes::coder_runtime::{
    ContainerRuntime, Launch, Listed, Request, Spawn,
};
use spec_graph_daemon::{
    Case, DOCS_OK, Sleepers, alive, observed, reason, restarted, settle, tracked_graphs, wait_dead,
};

const CRITERION: &str = "test -f built.txt";

/// A container runtime with no containers: these runs spawn host processes only.
struct NoContainers;

impl ContainerRuntime for NoContainers {
    fn name(&self) -> &'static str {
        "none"
    }

    fn run(
        &self,
        _ctx: &Ctx,
        _req: &Request<'_>,
        _timeout: Option<Duration>,
        _spawned: &mut dyn FnMut(&Spawn),
    ) -> Result<Launch, PiiError> {
        Err(PiiError::Internal(String::from("no containers run here")))
    }

    fn list_all(&self, _ctx: &Ctx) -> Result<Vec<Listed>, PiiError> {
        Ok(Vec::new())
    }

    fn force_remove(&self, _ctx: &Ctx, _id: &str) -> Result<bool, PiiError> {
        Ok(false)
    }
}

/// The real process table, with every kill recorded before it is sent.
#[derive(Default)]
struct Recording {
    kills: RefCell<Vec<u32>>,
}

impl ProcProbe for Recording {
    fn cmdline(&self, pid: u32) -> Option<String> {
        ProcFs.cmdline(pid)
    }

    fn kill_group(&self, pid: u32) -> Result<(), PiiError> {
        self.kills.borrow_mut().push(pid);
        ProcFs.kill_group(pid)
    }
}

fn ctx() -> Ctx {
    Ctx::with_observer(
        CapSet {
            kinds: CapKind::PROC,
            ..CapSet::default()
        },
        Budget::unlimited(),
        Arc::new(NullObserver),
        TraceCtx {
            run: RunId::from("boot"),
            span: 0,
        },
    )
}

/// Replaces the fake `claude` with one that records its pid and then sleeps under the name it was spawned as, so its
/// `/proc/<pid>/cmdline` starts with that name, as a real agent's does. The pid goes to `bin/pids` for [`Sleepers`].
fn hang(bin: &Path) {
    let script = format!(
        "#!/bin/bash\necho $$ >> '{}/pids'\nexec -a \"$0\" /bin/sleep 100000\n",
        bin.display()
    );
    std::fs::write(bin.join("claude"), script).unwrap();
}

/// Runs a test body on a tokio runtime: the engine's run and the test's waits share it.
fn on_runtime(body: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(body);
}

/// Polls the journal until `ready` holds, at most ten seconds, 50 ms apart. A missed deadline panics with the journal.
async fn wait_for(
    engine: &Engine,
    run: &RunId,
    what: &str,
    ready: impl Fn(&[(u64, RunEvent)]) -> bool,
) -> Vec<(u64, RunEvent)> {
    for _ in 0..200 {
        let journal = engine.store.read_journal(run).unwrap();
        if ready(&journal) {
            return journal;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let journal = engine.store.read_journal(run).unwrap();
    panic!("{what} never reached for {run}; journal: {journal:?}");
}

/// The pid a `coder.spawned` field names, which must be a positive integer.
fn pid_of(fields: &serde_json::Value) -> u32 {
    fields["pid"]
        .as_i64()
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid > 0)
        .unwrap_or_else(|| panic!("no positive pid in {fields}"))
}

/// The pids the run's coders spawned, in journal order.
fn spawned_pids(journal: &[(u64, RunEvent)]) -> Vec<u32> {
    observed(journal, "coder.spawned")
        .iter()
        .map(|(_, fields)| pid_of(fields))
        .collect()
}

/// A store of its own that holds `run` as a live run does: the run's journal copied from `source` under a `Running`
/// header. With `drop_first_exit`, the first `coder.exited` is left out: the near miss the reaper must catch.
fn reopen_running(
    case: &Case,
    source: &Engine,
    run: &RunId,
    name: &str,
    drop_first_exit: bool,
) -> Arc<Engine> {
    let dir = case.dir.path().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let dest = case.engine_on_store(&tracked_graphs(), dir.join("store.redb"));
    let mut header: RunHeader = source
        .store
        .get_run(run)
        .unwrap()
        .expect("the run has a header");
    header.status = RunStatus::Running;
    header.seq = 0;
    let mut exit_seen = false;
    let events: Vec<RunEvent> = source
        .store
        .read_journal(run)
        .unwrap()
        .into_iter()
        .map(|(_, ev)| ev)
        .filter(|ev| {
            let is_exit = format!("{ev:?}").contains("\"coder.exited\"");
            let drop = is_exit && drop_first_exit && !exit_seen;
            exit_seen |= is_exit;
            !drop
        })
        .collect();
    let refs: Vec<&RunEvent> = events.iter().collect();
    dest.store.append_events(&mut header, &refs).unwrap();
    dest
}

#[test]
fn a_live_coder_spawn_is_reaped_from_the_journal_the_run_wrote() {
    let Some(bin) = spec_graph_daemon::in_child(
        "a_live_coder_spawn_is_reaped_from_the_journal_the_run_wrote",
        DOCS_OK,
    ) else {
        return;
    };
    on_runtime(journal_reaped(bin));
}

async fn journal_reaped(bin: PathBuf) {
    let case = Case::new(&bin, CRITERION);
    hang(&case.bin);
    let sleepers = Sleepers::new(&case.bin);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");

    // (a) The spawn the run journalled: a host process, with the session and argv digest only the live writer sets.
    let journal = wait_for(&engine, &run, "coder.spawned", |j| {
        !observed(j, "coder.spawned").is_empty()
    })
    .await;
    let (_, fields) = observed(&journal, "coder.spawned").remove(0);
    assert_eq!(fields["runtime"], "host", "{fields}");
    let pid = pid_of(&fields);
    assert!(
        fields["session"].as_str().is_some_and(|s| !s.is_empty()),
        "{fields}"
    );
    assert!(
        fields["argv_digest"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "{fields}"
    );
    assert!(alive(pid), "the journalled agent {pid} is not running");
    assert!(
        !sleepers.pids().is_empty(),
        "the hanging agent never recorded its pid"
    );

    // (b) The reap, through the boot path. The copy is the store a restarted daemon finds: the run is still `Running`
    // and its agent alive. Boot's recovery calls `reap_orphans`, kills the journalled pid, and journals the reap on the
    // run; it then closes the run.
    let restart = restarted(&case, &tracked_graphs());
    let recording = Recording::default();
    restart.recover_boot(&ctx(), &NoContainers, &recording);
    assert_eq!(
        *recording.kills.borrow(),
        vec![pid],
        "the journalled pid is the one signalled"
    );
    assert!(
        wait_dead(pid).await,
        "the reaped agent {pid} is still alive"
    );

    let (status, journal) = settle(&restart, &run).await;
    assert_ne!(status, RunStatus::Running, "{journal:?}");
    assert!(
        observed(&journal, "coder.orphan_reaped")
            .iter()
            .any(
                |(_, f)| f["id"].as_str().and_then(|id| id.parse::<u32>().ok()) == Some(pid)
                    && f["kind"] == "process"
                    && f["result"] == "killed"
            ),
        "{journal:?}"
    );
    assert!(
        reason(&journal).contains("interrupted by daemon restart; agent reaped"),
        "{}",
        reason(&journal)
    );
}

#[test]
fn a_live_run_that_exited_is_not_searched() {
    let Some(bin) = spec_graph_daemon::in_child("a_live_run_that_exited_is_not_searched", DOCS_OK)
    else {
        return;
    };
    on_runtime(exited_is_not_searched(bin));
}

async fn exited_is_not_searched(bin: PathBuf) {
    let case = Case::new(&bin, CRITERION);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );
    assert!(
        !observed(&journal, "coder.exited").is_empty(),
        "{journal:?}"
    );

    // (c) The run as it stands: finished, so nothing is searched.
    let recording = Recording::default();
    let reaped = reap_orphans(&engine.store, &ctx(), &NoContainers, &recording);
    assert!(reaped.is_empty(), "{reaped:?}");
    assert!(recording.kills.borrow().is_empty());

    // The same journal held `Running`, its exits kept: every spawn is accounted for, so nothing is signalled.
    let kept = reopen_running(&case, &engine, &run, "kept", false);
    let recording = Recording::default();
    let reaped = reap_orphans(&kept.store, &ctx(), &NoContainers, &recording);
    assert!(reaped.is_empty(), "{reaped:?}");
    assert!(recording.kills.borrow().is_empty());
}

#[test]
fn a_live_run_is_reaped_once_its_exit_is_gone() {
    let Some(bin) =
        spec_graph_daemon::in_child("a_live_run_is_reaped_once_its_exit_is_gone", DOCS_OK)
    else {
        return;
    };
    on_runtime(near_miss_is_reaped(bin));
}

async fn near_miss_is_reaped(bin: PathBuf) {
    let case = Case::new(&bin, CRITERION);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );
    let pids = spawned_pids(&journal);
    assert_eq!(pids.len(), 2, "both coders spawned: {journal:?}");

    // The near miss: the first coder's `coder.exited` is gone from a copy of the run's own events. The run is now
    // searched and its first spawn is named; the second spawn's exit still accounts for its pid.
    let copy = reopen_running(&case, &engine, &run, "near", true);
    let recording = Recording::default();
    let reaped = reap_orphans(&copy.store, &ctx(), &NoContainers, &recording);
    assert_eq!(reaped.len(), 1, "{reaped:?}");
    assert_eq!(reaped[0].run, run.as_str(), "{reaped:?}");
    assert_eq!(reaped[0].kind, "process", "{reaped:?}");
    assert_eq!(reaped[0].id, pids[0].to_string(), "{reaped:?}");
    assert!(
        !alive(pids[0]),
        "the first coder's process is gone, so its pid is already gone"
    );
}

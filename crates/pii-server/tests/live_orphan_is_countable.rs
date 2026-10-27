//! Spec 297: the orphan count is read from a run a daemon wrote.
//!
//! `orphan_is_countable.rs` certifies `store_totals` on a spawn the test built by hand: its pid is one the test chose and its
//! origin is one the test passed back. Here the spawn is the one a run's own coder journalled through `Engine::start_run`,
//! and the origin is the one the run's header carries. The write side is `coder.rs::journal_spawned`; the read side is
//! `agent_calls::store_totals`, which feeds the health surface (`_docs/05-operations/LIVE-ORPHAN-COUNT.md`).

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

use pii_abi::{PiiError, RunEvent, RunId, RunStatus};
use pii_core::{Budget, CapKind, CapSet, Ctx, NullObserver, TraceCtx};
use pii_graph::agent_calls::{Totals, store_totals};
use pii_server::Engine;
use pii_server::engine::orphans::{ProcFs, ProcProbe};
use pii_server::node_kinds::nodes::coder_runtime::{
    ContainerRuntime, Launch, Listed, Request, Spawn,
};
use spec_graph_daemon::{
    Case, DOCS_OK, Sleepers, alive, observed, restarted_when_durable, settle, tracked_graphs,
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
/// `/proc/<pid>/cmdline` starts with that name. The pid goes to `bin/pids` for [`Sleepers`].
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

/// Starts the spec graph on `case`'s hanging agent and waits for the spawn the run journals with its pid. The pid is one
/// the run wrote, never one the test chose.
async fn start_hanging(case: &Case, engine: &Arc<Engine>) -> (RunId, u32) {
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    let journal = wait_for(engine, &run, "a coder.spawned with a pid", |j| {
        observed(j, "coder.spawned")
            .iter()
            .any(|(_, f)| f["pid"].as_i64().is_some_and(|pid| pid > 0))
    })
    .await;
    let pid = observed(&journal, "coder.spawned")
        .iter()
        .filter_map(|(_, f)| f["pid"].as_i64())
        .find(|pid| *pid > 0)
        .expect("a positive pid");
    let pid = u32::try_from(pid).expect("a pid fits in u32");
    assert!(alive(pid), "the journalled agent {pid} is not running");
    (run, pid)
}

/// The counter over `engine`'s store, with `origin` as the current process's start.
fn totals(engine: &Engine, origin: u64) -> Totals {
    store_totals(&engine.store, origin).unwrap()
}

/// The origin the run's own header carries: the value the live path stamped on it.
fn origin_of(engine: &Engine, run: &RunId) -> u64 {
    engine
        .store
        .get_run(run)
        .unwrap()
        .expect("the run has a header")
        .process_start_ms
}

#[test]
fn a_live_spawn_is_in_flight_while_its_run_runs() {
    let Some(bin) =
        spec_graph_daemon::in_child("a_live_spawn_is_in_flight_while_its_run_runs", DOCS_OK)
    else {
        return;
    };
    on_runtime(in_flight_while_running(bin));
}

async fn in_flight_while_running(bin: PathBuf) {
    let case = Case::new(&bin, CRITERION);
    hang(&case.bin);
    let _sleepers = Sleepers::new(&case.bin);
    let engine = case.engine(&tracked_graphs());
    let (run, _pid) = start_hanging(&case, &engine).await;

    // (a) A running run's spawn is in flight, not an orphan.
    let live = totals(&engine, origin_of(&engine, &run));
    assert_eq!(
        live.orphans_total, 0,
        "a running run's spawn is not an orphan"
    );
    assert!(
        live.agents_in_flight >= 1,
        "the running spawn is in flight: {live:?}"
    );
}

#[test]
fn a_live_spawn_is_an_orphan_once_the_run_is_terminal() {
    let Some(bin) = spec_graph_daemon::in_child(
        "a_live_spawn_is_an_orphan_once_the_run_is_terminal",
        DOCS_OK,
    ) else {
        return;
    };
    on_runtime(orphan_once_terminal(bin));
}

async fn orphan_once_terminal(bin: PathBuf) {
    let case = Case::new(&bin, CRITERION);
    hang(&case.bin);
    let _sleepers = Sleepers::new(&case.bin);
    let engine = case.engine(&tracked_graphs());
    let idle = totals(&engine, 0);
    let (run, pid) = start_hanging(&case, &engine).await;
    let live_origin = origin_of(&engine, &run);
    let live = totals(&engine, live_origin);
    let risen = live.agents_in_flight - idle.agents_in_flight;
    assert!(risen >= 1, "the spawn rose in flight: {live:?}");

    // (b) The same run once a restarted daemon has closed it: the reap is journalled, the spawn is an orphan.
    let restart = restarted_when_durable(&case, &tracked_graphs(), &run, 1).await;
    let recording = Recording::default();
    restart.recover_boot(&ctx(), &NoContainers, &recording);
    assert_eq!(
        *recording.kills.borrow(),
        vec![pid],
        "the journalled pid is the one signalled"
    );
    let (status, journal) = settle(&restart, &run).await;
    assert!(
        !matches!(status, RunStatus::Running),
        "the run is terminal: {status:?} {journal:?}"
    );

    // (c) The origin is the header's, read from the store the counter reads: the live path's stamp, as restarted.
    let origin = origin_of(&restart, &run);
    let after = totals(&restart, origin);
    assert_eq!(
        after.orphans_total, 1,
        "the terminal run's spawn is one orphan: {after:?} {journal:?}"
    );
    assert_eq!(
        after.orphans_since_start, 1,
        "the orphan belongs to the process that started the run: {after:?}"
    );
    assert_eq!(
        live.agents_in_flight - after.agents_in_flight,
        risen,
        "in flight dropped by what it rose: live {live:?}, after {after:?}"
    );
}

#[test]
fn a_live_run_that_exited_is_not_counted() {
    let Some(bin) = spec_graph_daemon::in_child("a_live_run_that_exited_is_not_counted", DOCS_OK)
    else {
        return;
    };
    on_runtime(exited_beside_reaped(bin));
}

async fn exited_beside_reaped(bin: PathBuf) {
    let case = Case::new(&bin, CRITERION);
    let engine = case.engine(&tracked_graphs());

    // The control: a run whose coder exits normally, journalling `coder.exited`.
    let exited = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    let (status, journal) = settle(&engine, &exited).await;
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );
    assert!(
        !observed(&journal, "coder.exited").is_empty(),
        "{journal:?}"
    );

    // The reaped run: its agent hangs, and a restarted daemon closes it.
    hang(&case.bin);
    let _sleepers = Sleepers::new(&case.bin);
    let (reaped, _pid) = start_hanging(&case, &engine).await;
    let restart = restarted_when_durable(&case, &tracked_graphs(), &reaped, 1).await;
    restart.recover_boot(&ctx(), &NoContainers, &Recording::default());
    let (status, journal) = settle(&restart, &reaped).await;
    assert!(
        !matches!(status, RunStatus::Running),
        "{status:?}: {journal:?}"
    );

    // Over both runs: every spawn either run journalled is a dispatch, and only the reaped run's spawn is an orphan. The
    // spawns are counted from the journals the runs wrote (a docs-brief run journals its docs coder and its code coder).
    let origin = origin_of(&restart, &reaped);
    let after = totals(&restart, origin);
    let spawns = [&exited, &reaped]
        .iter()
        .map(|run| observed(&restart.store.read_journal(run).unwrap(), "coder.spawned").len())
        .sum::<usize>();
    assert_eq!(
        after.dispatches_total,
        u64::try_from(spawns).unwrap(),
        "{after:?}"
    );
    assert_eq!(after.orphans_total, 1, "{after:?}");
}

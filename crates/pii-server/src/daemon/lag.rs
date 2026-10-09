//! Reactive-loop lag recovery (spec 79 / 043 D6).
//!
//! The daemon's reactive loop subscribes to the bus; when the receiver lags,
//! broadcast drops events — including CONTROL events such as `RunSuspended`,
//! which arms the timer that resumes a suspended run. Dropping one used to
//! mean a suspended run never resumed, silently.
//!
//! This module holds the two helpers that make recovery possible: the stderr
//! log line (the crate has no tracing dependency) and the rescan that re-applies
//! the latest suspension of every `Suspended` run. Kept out of `daemon/mod.rs`
//! so that file stays inside the workspace's effective-line gate.

use std::sync::Arc;

use pii_abi::{RunEvent, RunId, RunStatus};

use crate::Engine;
use crate::daemon::scheduler::Scheduler;
use crate::daemon::status::DaemonContext;
use crate::daemon::{on_finished, on_suspended};
use crate::engine::bus::map_terminal;

/// Daemon stderr log line (no tracing dep wired; stderr is the module's channel).
pub(super) fn log_warn(msg: &str) {
    #[allow(clippy::print_stderr)]
    {
        eprintln!("pii: {msg}");
    }
}

/// Lag recovery (spec 79): re-reacts to the latest suspension of every
/// `Suspended` run. Idempotent — the wheel is keyed by (fire time, run), so
/// re-arming an already-armed timer is a no-op, and an approval that already
/// has a row (pending or decided) is neither re-requested nor overwritten.
pub(super) fn recover_suspended(engine: &Arc<Engine>, scheduler: &Arc<Scheduler>) {
    let runs = match engine.store.runs_by_status(RunStatus::Suspended) {
        Ok(runs) => runs,
        Err(e) => {
            log_warn(&format!("lag recovery: cannot list suspended runs: {e}"));
            return;
        }
    };
    for run in runs {
        let Ok(journal) = engine.store.read_journal(&run) else {
            continue;
        };
        let key = journal.iter().rev().find_map(|(_, e)| match e {
            RunEvent::Suspended { key, .. } => Some(key.0.clone()),
            _ => None,
        });
        if let Some(key) = key {
            on_suspended(engine, scheduler, run.as_str(), &key);
        }
    }
}

/// Lag recovery for lost `RunFinished` events: any run still tracked in
/// `cron_runs` / `channel_runs` whose journal already holds a terminal
/// outcome had its event dropped, so `on_finished` is re-driven for it. The
/// maps are drained by `on_finished`, which makes this idempotent.
///
/// Not covered: consumers of `RunFinished` that keep no daemon-side
/// bookkeeping (bus subscribers such as CLI/TUI streams, which get `Lagged`
/// themselves) — only cron re-arm and channel replies are recovered.
pub(super) fn recover_finished(engine: &Arc<Engine>, ctx: &Arc<DaemonContext>) {
    let mut tracked: Vec<String> = ctx.cron_runs.lock().keys().cloned().collect();
    tracked.extend(ctx.channel_runs.lock().keys().cloned());
    tracked.sort();
    tracked.dedup();
    for run_id in tracked {
        let Ok(journal) = engine.store.read_journal(&RunId::from(run_id.as_str())) else {
            continue;
        };
        let terminal = journal.iter().rev().find_map(|(_, e)| match e {
            RunEvent::Finished { terminal, .. } => Some(map_terminal(terminal)),
            _ => None,
        });
        if let Some(terminal) = terminal {
            on_finished(engine, ctx, &run_id, &terminal);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use pii_abi::{
        NodeId, ResumeKey, RunEvent, RunHeader, RunId, RunStatus, SuspendReason, Terminal,
    };
    use pii_core::FixedClock;
    use pii_proto::event::Event;
    use pii_store::timer::TimerWheel;

    use super::{recover_finished, recover_suspended};
    use crate::daemon::scheduler::Scheduler;
    use crate::daemon::status::CronEntry;
    use crate::daemon::tests::test_ctx;
    use crate::{Engine, EngineConfig};

    fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
        let fixture = dir.join("fixture.json");
        std::fs::write(&fixture, r#"{"turns":[]}"#).unwrap();
        Engine::new(EngineConfig {
            workspace: dir.to_path_buf(),
            store_path: dir.join("store.redb"),
            model: format!("scripted:{}", fixture.display()),
            sandbox: pii_tools::sandbox::SandboxProfileName::Off,
            graph_dir: dir.to_path_buf(),
            route_policy: std::collections::BTreeMap::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
        })
        .unwrap()
    }

    fn scratch(prefix: &str) -> tempfile::TempDir {
        let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"));
        std::fs::create_dir_all(root).unwrap();
        tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(root)
            .unwrap()
    }

    fn header(run: &str, status: RunStatus) -> RunHeader {
        RunHeader {
            run: RunId::from(run),
            graph: String::from("default"),
            status,
            seq: 0,
            dur_ms: 0,
        }
    }

    /// Spec 88 R3a: a lag rescan must neither erase a recorded decision nor
    /// re-broadcast `ApprovalRequested` while the run is still `Suspended`.
    #[tokio::test]
    async fn lag_recovery_keeps_decided_approval_and_does_not_rebroadcast() {
        let tmp = scratch("lag-approval-");
        let dir = tmp.path().to_path_buf();
        let engine = engine_in(&dir);
        let mut h = header("r-appr", RunStatus::Suspended);
        engine
            .store
            .append_event(
                &mut h,
                &RunEvent::Suspended {
                    node: NodeId::from("gate"),
                    reason: SuspendReason::Approval {
                        policy: String::from("p"),
                        prompt: String::from("ok?"),
                    },
                    key: ResumeKey(String::from("approval:appr-1")),
                },
            )
            .unwrap();
        crate::daemon::approvals::request(&engine, "appr-1", "r-appr", "gate", "p", "ok?").unwrap();
        engine
            .approvals
            .resolve_approval("appr-1", true, Some("fine"), 42)
            .unwrap();
        let before = engine.approvals.get_approval("appr-1").unwrap().unwrap();

        let mut rx = engine.bus.subscribe();
        let wheel = Arc::new(TimerWheel::open(&dir.join("timer.redb")).unwrap());
        let scheduler = Scheduler::new(Arc::clone(&engine), wheel, Arc::new(FixedClock::new()));
        recover_suspended(&engine, &scheduler);

        let mut requested = 0;
        while let Ok(se) = rx.try_recv() {
            if matches!(se.event, Event::ApprovalRequested { .. }) {
                requested += 1;
            }
        }
        assert_eq!(requested, 0, "lag recovery re-broadcast ApprovalRequested");
        let after = engine.approvals.get_approval("appr-1").unwrap().unwrap();
        assert_eq!(after, before, "decided approval row must be untouched");
        assert_eq!(after.decision, Some(true));
    }

    /// Spec 88 R3b: a cron run whose `RunFinished` was dropped under a lag
    /// must still start the next firing.
    #[tokio::test]
    async fn lagged_run_finished_still_starts_next_cron_firing() {
        let tmp = scratch("lag-cron-");
        let dir = tmp.path().to_path_buf();
        std::fs::write(
            dir.join("tick.toml"),
            "name = \"tick\"\nentry = \"step\"\ncaps = []\n\n[[node]]\nid = \"step\"\nkind = \"transform\"\nvalue = \"ok\"\n",
        )
        .unwrap();
        let engine = engine_in(&dir);
        let mut h = header("r-cron-old", RunStatus::Succeeded);
        engine
            .store
            .append_event(
                &mut h,
                &RunEvent::Finished {
                    terminal: Terminal::Success {
                        output: serde_json::json!("ok"),
                    },
                    output_hash: String::new(),
                },
            )
            .unwrap();
        let ctx = test_ctx(&dir);
        ctx.cron.lock().insert(
            String::from("job"),
            CronEntry {
                name: String::from("job"),
                graph: String::from("tick"),
                schedule: String::from("* * * * *"),
                input: serde_json::json!({}),
            },
        );
        ctx.cron_runs
            .lock()
            .insert(String::from("r-cron-old"), String::from("job"));

        recover_finished(&engine, &ctx);

        let runs = ctx.cron_runs.lock().clone();
        assert!(
            !runs.contains_key("r-cron-old"),
            "finished run still tracked"
        );
        assert_eq!(runs.len(), 1, "next firing must have started: {runs:?}");
        assert!(runs.values().all(|n| n == "job"));
    }

    /// Spec 92 R3: the REAL `reactive_loop` must call `recover_finished` on a
    /// `Lagged` receive (the test above calls the helper directly, so it
    /// cannot pin the wiring). The bus is flooded past capacity before the
    /// loop's first read, so tokio itself reports the lag.
    #[tokio::test]
    async fn reactive_loop_lag_rearms_cron_chain_for_lost_run_finished() {
        let tmp = scratch("lag-loop-cron-");
        let dir = tmp.path().to_path_buf();
        std::fs::write(
            dir.join("tick.toml"),
            "name = \"tick\"\nentry = \"step\"\ncaps = []\n\n[[node]]\nid = \"step\"\nkind = \"transform\"\nvalue = \"ok\"\n",
        )
        .unwrap();
        let engine = engine_in(&dir);
        let mut h = header("r-cron-old", RunStatus::Succeeded);
        engine
            .store
            .append_event(
                &mut h,
                &RunEvent::Finished {
                    terminal: Terminal::Success {
                        output: serde_json::json!("ok"),
                    },
                    output_hash: String::new(),
                },
            )
            .unwrap();
        let ctx = test_ctx(&dir);
        ctx.cron.lock().insert(
            String::from("job"),
            CronEntry {
                name: String::from("job"),
                graph: String::from("tick"),
                schedule: String::from("* * * * *"),
                input: serde_json::json!({}),
            },
        );
        ctx.cron_runs
            .lock()
            .insert(String::from("r-cron-old"), String::from("job"));
        let wheel = Arc::new(TimerWheel::open(&dir.join("timer.redb")).unwrap());
        let scheduler = Scheduler::new(
            Arc::clone(&engine),
            Arc::clone(&wheel),
            Arc::new(FixedClock::new()),
        );

        let rx = engine.bus.subscribe();
        for i in 0..5_000u64 {
            engine.bus.broadcast(Event::Lagged {
                sub_id: String::from("burst"),
                dropped: i,
            });
        }
        let handle = tokio::spawn(super::super::reactive_loop(
            Arc::clone(&engine),
            Arc::clone(&ctx),
            scheduler,
            rx,
        ));

        let mut runs = ctx.cron_runs.lock().clone();
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            runs = ctx.cron_runs.lock().clone();
            if !runs.contains_key("r-cron-old") {
                break;
            }
        }
        handle.abort();
        assert!(
            !runs.contains_key("r-cron-old"),
            "lost RunFinished must be recovered by the loop's Lagged arm: {runs:?}"
        );
        assert_eq!(
            runs.len(),
            1,
            "next cron firing must have started: {runs:?}"
        );
    }
}

//! Reaction to a suspension (F5.5/F5.9, spec 106): arms the timer wheel or
//! creates the run-scoped pending approval. A suspension that cannot be
//! acted on is REPORTED, never skipped silently.

use std::sync::Arc;
use std::time::Duration;

use pii_abi::{NodeId, RunEvent, RunId, SuspendReason};

use super::{approvals, lag, scheduler::Scheduler};
use crate::Engine;

/// Journal lookups tried before giving up. The `RunSuspended` event is
/// notified when the event is appended, BEFORE the write-behind journal
/// flush makes it readable, so the first read can legitimately miss it.
const LOOKUP_ATTEMPTS: u32 = 40;
const LOOKUP_BACKOFF: Duration = Duration::from_millis(25);

/// Handles one `RunSuspended` off the reactive loop, so a lookup that must
/// wait for the journal flush never stalls the event stream.
pub(super) fn spawn(engine: &Arc<Engine>, scheduler: &Arc<Scheduler>, run_id: &str, key: &str) {
    let (engine, scheduler) = (Arc::clone(engine), Arc::clone(scheduler));
    let (run_id, key) = (run_id.to_owned(), key.to_owned());
    tokio::spawn(async move {
        handle(&engine, &scheduler, &run_id, &key, LOOKUP_ATTEMPTS).await;
    });
}

fn find(
    engine: &Engine,
    run_id: &str,
    key: &str,
) -> Result<Option<(NodeId, SuspendReason)>, String> {
    let journal = engine
        .store
        .read_journal(&RunId::from(run_id))
        .map_err(|e| e.to_string())?;
    Ok(journal.into_iter().rev().find_map(|(_, e)| match e {
        RunEvent::Suspended {
            node,
            reason,
            key: k,
        } if k.0 == key => Some((node, reason)),
        _ => None,
    }))
}

/// Reports a suspension that could not be turned into its follow-up.
fn report(engine: &Engine, run_id: &str, key: &str, why: &str) {
    let message = format!("suspension of run {run_id} (key {key}) not handled: {why}");
    lag::log_warn(&message);
    engine.report_unhandled(&format!("suspend:{run_id}"), &message);
}

pub(super) async fn handle(
    engine: &Arc<Engine>,
    scheduler: &Arc<Scheduler>,
    run_id: &str,
    key: &str,
    attempts: u32,
) {
    let mut last = String::from("no matching Suspended event in the journal");
    let mut found = None;
    for i in 0..attempts {
        match find(engine, run_id, key) {
            Ok(Some(hit)) => {
                found = Some(hit);
                break;
            }
            Ok(None) => {}
            Err(e) => last = format!("journal read failed: {e}"),
        }
        if i + 1 < attempts {
            tokio::time::sleep(LOOKUP_BACKOFF).await;
        }
    }
    let Some((node, reason)) = found else {
        report(engine, run_id, key, &last);
        return;
    };
    match reason {
        SuspendReason::Timer { .. } => {
            if let Some(fire_at) = key.rsplit(':').next().and_then(|s| s.parse::<u64>().ok()) {
                scheduler.arm(fire_at, run_id, key);
            } else {
                report(engine, run_id, key, "timer key carries no fire time");
            }
        }
        SuspendReason::Approval { policy, prompt } => {
            // The key is `approval:<id>` with a run-scoped id minted by the
            // gate; the policy name must never be the id.
            let Some(approval_id) = key.strip_prefix("approval:") else {
                report(engine, run_id, key, "approval key is not `approval:<id>`");
                return;
            };
            if let Err(e) =
                approvals::request(engine, approval_id, run_id, node.as_str(), &policy, &prompt)
            {
                report(
                    engine,
                    run_id,
                    key,
                    &format!("approval request failed: {e}"),
                );
            }
        }
        SuspendReason::Event { .. } | SuspendReason::HumanInput { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use pii_abi::{NodeId, ResumeKey, RunEvent, RunHeader, RunId, RunStatus, SuspendReason};
    use pii_core::{FixedClock, Metric, ObsEvent, Observer};
    use pii_store::timer::TimerWheel;

    use crate::daemon::scheduler::Scheduler;
    use crate::{Engine, EngineConfig};

    #[derive(Default)]
    struct Seen(parking_lot::Mutex<Vec<String>>);

    impl Observer for Seen {
        fn event(&self, e: &ObsEvent) {
            if let ObsEvent::PersistError { context, message } = e {
                self.0.lock().push(format!("{context}: {message}"));
            }
        }
        fn metric(&self, _m: &Metric) {}
        fn flush(&self) {}
    }

    fn rig(prefix: &str) -> (tempfile::TempDir, Arc<Engine>, Arc<Scheduler>, Arc<Seen>) {
        let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).unwrap();
        let dir = tmp.path().to_path_buf();
        let fixture = dir.join("fixture.json");
        std::fs::write(&fixture, r#"{"turns":[]}"#).unwrap();
        let engine = Engine::new(EngineConfig {
            workspace: dir.clone(),
            store_path: dir.join("store.redb"),
            model: format!("scripted:{}", fixture.display()),
            sandbox: pii_tools::sandbox::SandboxProfileName::Off,
            graph_dir: dir.clone(),
            route_policy: std::collections::BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
        })
        .unwrap();
        let seen = Arc::new(Seen::default());
        engine.attach_observer(Arc::clone(&seen) as Arc<dyn Observer>);
        let wheel = Arc::new(TimerWheel::open(&dir.join("timer.redb")).unwrap());
        let sched = Scheduler::new(Arc::clone(&engine), wheel, Arc::new(FixedClock::new()));
        (tmp, engine, sched, seen)
    }

    fn suspend_approval(engine: &Engine, run: &str, key: &str) {
        let mut header = RunHeader {
            run: RunId::from(run),
            graph: String::from("gated"),
            status: RunStatus::Suspended,
            seq: 0,
            dur_ms: 0,
            revision: String::new(),
            process_start_ms: 0,
            created_ms: 0,
        };
        engine
            .store
            .append_event(
                &mut header,
                &RunEvent::Suspended {
                    node: NodeId::from("gate"),
                    reason: SuspendReason::Approval {
                        policy: String::from("p"),
                        prompt: String::from("ok?"),
                    },
                    key: ResumeKey(String::from(key)),
                },
            )
            .unwrap();
    }

    /// R4: a suspension whose journal entry cannot be found is reported.
    #[tokio::test]
    async fn missing_journal_entry_is_reported() {
        let (_t, engine, sched, seen) = rig("suspend-miss-");
        super::handle(&engine, &sched, "r-ghost", "approval:abc", 2).await;
        let got = seen.0.lock().clone();
        assert_eq!(got.len(), 1, "one report expected: {got:?}");
        assert!(got.iter().all(|g| g.contains("r-ghost")), "{got:?}");
    }

    /// R4: a key that cannot yield an approval id is reported, not skipped.
    #[tokio::test]
    async fn unusable_approval_key_is_reported() {
        let (_t, engine, sched, seen) = rig("suspend-key-");
        suspend_approval(&engine, "r-badkey", "p");
        super::handle(&engine, &sched, "r-badkey", "p", 1).await;
        assert_eq!(seen.0.lock().len(), 1, "{:?}", seen.0.lock());
        assert!(engine.approvals.pending_approvals().unwrap().is_empty());
    }

    /// R3: the good path creates a row keyed by the run-scoped id, never the
    /// policy name, and reports nothing.
    #[tokio::test]
    async fn approval_row_uses_the_key_id_not_the_policy() {
        let (_t, engine, sched, seen) = rig("suspend-ok-");
        suspend_approval(&engine, "r1", "approval:id-r1");
        suspend_approval(&engine, "r2", "approval:id-r2");
        super::handle(&engine, &sched, "r1", "approval:id-r1", 1).await;
        super::handle(&engine, &sched, "r2", "approval:id-r2", 1).await;
        let mut ids: Vec<_> = engine
            .approvals
            .pending_approvals()
            .unwrap()
            .into_iter()
            .map(|a| a.approval_id)
            .collect();
        ids.sort();
        assert_eq!(ids, ["id-r1", "id-r2"]);
        assert!(seen.0.lock().is_empty());
    }
}

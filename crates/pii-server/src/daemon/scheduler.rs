//! Timer scheduler (F5.5/N5): ONE `tokio::time::sleep_until` on the
//! earliest wheel entry, rearmed on every insert — never a poll loop, never
//! a thread-per-timer, never a fixed tick.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pii_abi::{ResumeKey, RunId};
use pii_core::Clock;
use pii_store::timer::TimerWheel;
use tokio::sync::Notify;

use crate::Engine;

/// Drives the timer wheel (F5.5).
///
/// Firing resumes each due run through [`crate::engine::runs::spawn_resume`];
/// cross-process safety for "was this timer already taken" comes from
/// `TimerWheel::take_due`'s single write txn.
pub struct Scheduler {
    engine: Arc<Engine>,
    wheel: Arc<TimerWheel>,
    clock: Arc<dyn Clock>,
    wakeups: AtomicU64,
    notify: Notify,
}

impl Scheduler {
    /// Builds a scheduler over `wheel`, resuming through `engine`.
    #[must_use]
    pub fn new(engine: Arc<Engine>, wheel: Arc<TimerWheel>, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            engine,
            wheel,
            clock,
            wakeups: AtomicU64::new(0),
            notify: Notify::new(),
        })
    }

    /// Scheduler wakeups so far (N5's test hook): a real sleep firing, not
    /// an early rearm.
    #[must_use]
    pub fn wakeups(&self) -> u64 {
        self.wakeups.load(Ordering::Relaxed)
    }

    /// Registers a timer and rearms the sleep so a now-earlier entry is
    /// picked up immediately rather than waiting out the old target.
    pub fn arm(&self, fire_at_ms: u64, run: &str, key: &str) {
        if self.wheel.put_timer(fire_at_ms, run, key).is_ok() {
            self.notify.notify_one();
        }
    }

    /// The scheduler loop: spawned once by the daemon, runs until the
    /// process exits (F5.5).
    pub async fn run(self: Arc<Self>) {
        loop {
            let next = self.wheel.next_timer().ok().flatten();
            let Some(fire_at) = next else {
                self.notify.notified().await;
                continue;
            };
            let now = self.clock.now_ms();
            if fire_at <= now {
                self.fire_due_sync(now);
                continue;
            }
            let dur = Duration::from_millis(fire_at - now);
            let deadline = tokio::time::Instant::now() + dur;
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => {
                    self.wakeups.fetch_add(1, Ordering::Relaxed);
                    self.fire_due_sync(fire_at);
                }
                () = self.notify.notified() => {
                    // A new (possibly earlier) timer arrived; loop and
                    // recompute rather than waiting out the old target.
                }
            }
        }
    }

    fn fire_due_sync(&self, now_ms: u64) {
        let due = self.wheel.take_due(now_ms).unwrap_or_default();
        for d in due {
            let run_id = RunId::from(d.run.as_str());
            let key = ResumeKey(d.key.clone());
            if let Err(e) =
                crate::engine::runs::spawn_resume(Arc::clone(&self.engine), run_id, Some(key))
            {
                // Refusals are loud (spec 177/196): a due timer must not vanish in silence.
                self.engine.log().event(
                    &pii_observe::Event::new(
                        "scheduler",
                        pii_observe::Level::Warn,
                        pii_observe::names::TIMER_RESUME_REFUSED,
                    )
                    .field("run", d.run.as_str())
                    .field("key", d.key.as_str())
                    .field("error", e.to_string().as_str()),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use pii_core::FixedClock;
    use pii_store::timer::TimerWheel;

    use super::Scheduler;
    use crate::{Engine, EngineConfig};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/_tmp/phase3/scheduler-test"
        ));
        let _ = std::fs::create_dir_all(dir);
        dir.join(format!("{name}-{}.redb", std::process::id()))
    }

    fn engine() -> Arc<Engine> {
        let dir = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/_tmp/phase3/scheduler-test"
        ));
        let _ = std::fs::create_dir_all(dir);
        Engine::new(EngineConfig {
            grant_ceiling: None,
            workspace: dir.to_path_buf(),
            store_path: scratch("store"),
            model: format!("scripted:{}", write_empty_fixture(&scratch("fixture.json"))),
            sandbox: pii_tools::sandbox::SandboxProfileName::Off,
            graph_dir: dir.to_path_buf(),
            route_policy: std::collections::BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
            mqtt_tool_url: None,
        })
        .unwrap()
    }

    fn write_empty_fixture(path: &std::path::Path) -> String {
        std::fs::write(path, r#"{"turns":[]}"#).unwrap();
        path.display().to_string()
    }

    #[tokio::test(start_paused = true)]
    async fn arms_once_not_polls_then_fires() {
        let engine = engine();
        let wheel = Arc::new(TimerWheel::open(&scratch("wheel")).unwrap());
        let clock = Arc::new(FixedClock::new());
        let sched = Scheduler::new(engine, wheel, clock);
        sched.arm(600_000, "r-nonexistent", "timer:r-nonexistent:t:600000");
        let handle = tokio::spawn(Arc::clone(&sched).run());

        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        assert_eq!(
            sched.wakeups(),
            0,
            "10-minute-out timer must not wake early"
        );

        tokio::time::advance(std::time::Duration::from_mins(10)).await;
        // Yield so the woken task actually runs its continuation.
        tokio::task::yield_now().await;
        assert_eq!(sched.wakeups(), 1, "must fire exactly once at the deadline");
        handle.abort();
    }
}

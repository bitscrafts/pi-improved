//! The in-flight barrier: a node start that outlives the window is made durable while it runs (spec 168/179).

use std::time::Duration;

use super::dispatch::Env;

/// How long a node's start may sit unflushed while the node runs (D3).
const START_DURABLE_WITHIN: Duration = Duration::from_secs(2);

/// Runs `f`; if it outlives the window since the last flush, a watcher makes
/// the pending `NodeStarted` durable. A fast node finishes first and costs no
/// transaction.
pub(super) fn run_with_barrier<T>(env: &Env<'_>, f: impl FnOnce() -> T) -> T {
    let (done, wait) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|s| {
        s.spawn(move || {
            let left = START_DURABLE_WITHIN.saturating_sub(env.jr.since_flush());
            if matches!(
                wait.recv_timeout(left),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) && let Err(e) = env.jr.flush_if_stale(START_DURABLE_WITHIN)
            {
                env.jr.report_flush_failure(&e);
            }
        });
        let out = f();
        drop(done);
        out
    })
}

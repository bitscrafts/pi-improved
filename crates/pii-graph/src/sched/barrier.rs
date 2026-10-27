//! The in-flight barrier: a node start that outlives the window is made durable while it runs (spec 168/179), and a
//! running node's agent-call observations are journalled as it records them (spec 289).

use std::sync::mpsc::Receiver;
use std::time::Duration;

use pii_abi::PiiError;

use super::dispatch::Env;
use super::early_calls::{Calls, Tick, watch};

/// How long a node's start may sit unflushed while the node runs (D3).
pub(super) const START_DURABLE_WITHIN: Duration = Duration::from_secs(2);

/// Runs `f` with a watcher beside it. The watcher makes the node's start durable if it outlives the window, and journals
/// its agent-call observations as they are recorded. A fast node finishes first and costs no transaction. Returns `f`'s
/// value and the watcher's outcome.
pub(super) fn run_with_barrier<T>(
    env: &Env<'_>,
    calls: &Calls,
    wake: Receiver<Tick>,
    f: impl FnOnce() -> T,
) -> (T, Result<(), PiiError>) {
    std::thread::scope(|s| {
        let watcher = s.spawn(move || watch(env, calls, &wake));
        // Dropped on unwind too, so the watcher ends even when `f` panics.
        let done = calls.finish();
        let out = f();
        drop(done);
        let early = watcher
            .join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
        (out, early)
    })
}

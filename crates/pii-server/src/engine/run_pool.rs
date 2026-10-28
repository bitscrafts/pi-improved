//! Spec 308: the run pool. Runs execute on a fixed set of long-lived threads of their own, so the
//! shared blocking pool (heavy RPCs, channel polls, learn timers) never queues behind a run.
//!
//! A run is queued until a thread claims it. A cancel that arrives while it is queued claims it
//! instead, and the thread then drops it without executing anything ([`Claim`]).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, mpsc};

use parking_lot::Mutex;
use pii_abi::PiiError;

/// Pool size when `PII_DAEMON_RUN_THREADS` is unset, unparsable or below 1.
pub const DEFAULT_RUN_THREADS: usize = 8;

const QUEUED: u8 = 0;
const CLAIMED: u8 = 1;
const CANCELLED: u8 = 2;

type Job = Box<dyn FnOnce() + Send + 'static>;

/// Who gets a queued run first: its thread (the job runs) or a cancel (the job never does).
#[derive(Clone, Debug, Default)]
pub struct Claim(Arc<AtomicU8>);

impl Claim {
    /// A claim on a run that is already executing (its caller runs it): a cancel never finds it queued.
    #[must_use]
    pub fn executing() -> Self {
        Self(Arc::new(AtomicU8::new(CLAIMED)))
    }

    /// Job side: true when the run is still queued, now claimed for execution.
    #[must_use]
    pub fn start(&self) -> bool {
        self.0
            .compare_exchange(QUEUED, CLAIMED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Cancel side: true when the run was still queued, now cancelled; its job will not execute.
    #[must_use]
    pub fn cancel_queued(&self) -> bool {
        self.0
            .compare_exchange(QUEUED, CANCELLED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
}

/// The run threads (`pii-run-0` ...). Jobs are pulled from one shared queue.
pub struct RunPool {
    tx: mpsc::Sender<Job>,
    closing: Arc<AtomicBool>,
}

impl RunPool {
    /// Starts `size` (at least one) threads. Each enters the tokio runtime that is current here,
    /// if any, once for its whole life. A job that panics is caught; its thread carries on.
    ///
    /// # Errors
    /// [`PiiError::Internal`] when a thread cannot be spawned.
    pub fn new(size: usize) -> Result<Self, PiiError> {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let closing = Arc::new(AtomicBool::new(false));
        let runtime = tokio::runtime::Handle::try_current().ok();
        for i in 0..size.max(1) {
            let (rx, closing, runtime) = (Arc::clone(&rx), Arc::clone(&closing), runtime.clone());
            std::thread::Builder::new()
                .name(format!("pii-run-{i}"))
                .spawn(move || {
                    let _entered = runtime.as_ref().map(tokio::runtime::Handle::enter);
                    loop {
                        // The guard is a temporary: the lock is released before the job runs.
                        let next = rx.lock().recv();
                        let Ok(job) = next else { break };
                        // Shutdown: a job still queued is dropped, never awaited.
                        if closing.load(Ordering::SeqCst) {
                            continue;
                        }
                        // The default panic hook has already reported the message; the thread survives.
                        let _ = catch_unwind(AssertUnwindSafe(job));
                    }
                })
                .map_err(|e| PiiError::Internal(format!("run thread {i}: {e}")))?;
        }
        Ok(Self { tx, closing })
    }

    /// Queues `job` for the next free run thread.
    pub fn submit(&self, job: impl FnOnce() + Send + 'static) {
        // A send fails only once every thread has exited, which means the pool is shutting down.
        let _ = self.tx.send(Box::new(job));
    }
}

impl Drop for RunPool {
    fn drop(&mut self) {
        self.closing.store(true, Ordering::SeqCst);
    }
}

/// Spec 308 R3: true when a run thread must not execute the run. It was cancelled while queued (its
/// terminal is already journalled by the cancel) or its token tripped first. Its handle is dropped here.
pub fn skip_queued(
    engine: &crate::Engine,
    run_id: &pii_abi::RunId,
    cancel: &pii_core::CancelToken,
    claim: &Claim,
) -> bool {
    if !cancel.is_cancelled() && claim.start() {
        return false;
    }
    engine.unregister_run(run_id.as_str());
    true
}

/// The configured pool size: `PII_DAEMON_RUN_THREADS`, [`DEFAULT_RUN_THREADS`] when unset or below 1.
#[must_use]
pub fn configured_threads() -> usize {
    pii_core::env_var("PII_DAEMON_RUN_THREADS")
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(DEFAULT_RUN_THREADS)
}

//! Supervisor control plane (F3.5/F3.6): invoke, death, restart, reload.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use pii_abi::PiiError;
use pii_core::Ctx;

use crate::frame::WorkerFrame;
use crate::spec::{ReloadReport, WorkerState};
use crate::supervisor::{Queued, Slot, Supervisor};

const MAX_QUEUE: usize = 256;
const RELOAD_DRAIN_DEADLINE_MS: u64 = 5_000;
const RELOAD_SHUTDOWN_MS: u64 = 1_000;

impl Supervisor {
    /// One synchronous invoke (F3.4). During a drain, requests are queued
    /// (bounded) instead of answered with a reload-caused error (F3.6).
    ///
    /// # Errors
    /// [`PiiError::Unavailable`] for a dead worker / full queue;
    /// [`PiiError::Denied`] for a Failed worker.
    pub fn invoke(
        &self,
        _ctx: &Ctx,
        op: &str,
        args: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<serde_json::Value, PiiError> {
        let state = self.inner.state.lock().state;
        match state {
            WorkerState::Failed => {
                return Err(PiiError::Denied(format!(
                    "worker '{}' is failed",
                    self.inner.spec.name
                )));
            }
            WorkerState::Draining | WorkerState::Restarting => {
                // Queue (bounded 256); a full queue is retryable (F3.6).
                let (tx, rx) = channel();
                let mut q = self.inner.queue.lock();
                if q.len() >= MAX_QUEUE {
                    return Err(PiiError::Unavailable(format!(
                        "worker '{}' reload queue full",
                        self.inner.spec.name
                    )));
                }
                q.push(Queued {
                    op: String::from(op),
                    args,
                    tx,
                });
                drop(q);
                return rx
                    .recv_timeout(Duration::from_millis(timeout_ms.max(30_000)))
                    .map_err(|_| {
                        PiiError::Unavailable(format!("worker '{op}' queued too long"))
                    })?;
            }
            WorkerState::Starting | WorkerState::Ready => {}
        }
        self.invoke_now(op, &args, timeout_ms)
    }

    pub(crate) fn invoke_now(
        &self,
        op: &str,
        args: &serde_json::Value,
        timeout_ms: u64,
    ) -> Result<serde_json::Value, PiiError> {
        let (tx, rx) = channel();
        let id = {
            let mut sh = self.inner.state.lock();
            let id = sh.next_id;
            sh.next_id += 1;
            id
        };
        self.inner.slots.lock().insert(id, Slot { tx });
        self.send(WorkerFrame::Req {
            id,
            op: String::from(op),
            args: args.clone(),
        })?;
        let Ok(r) = rx.recv_timeout(Duration::from_millis(timeout_ms.max(1))) else {
            self.inner.slots.lock().remove(&id);
            return Err(PiiError::Unavailable(format!(
                "worker '{op}' timed out after {timeout_ms}ms"
            )));
        };
        r
    }

    /// Death handling (F3.5): complete in-flight with `WorkerDied`, record
    /// the event, respawn after the backoff unless capped or draining.
    pub(crate) fn apply(&self, frame: WorkerFrame) {
        if let WorkerFrame::Res { id, ok, err } = frame
            && let Some(slot) = self.inner.slots.lock().remove(&id)
        {
            let result = match (ok, err) {
                (Some(v), _) => Ok(v),
                (None, Some(e)) => Err(PiiError::Invalid(format!(
                    "worker error {}: {}",
                    e.code, e.message
                ))),
                (None, None) => Err(PiiError::Internal(String::from("worker empty res"))),
            };
            let _ = slot.tx.send(result);
        }
    }

    pub(crate) fn death(&self, code: Option<i32>) {
        // Destructure a snapshot, then release the state lock before any
        // blocking child teardown: `kill()` waits for the process to die,
        // and `invoke()` must observe a post-death state (Restarting) and
        // queue or fail WITHOUT waiting for that teardown.
        let (pid, in_flight, ids, capped) = {
            let mut sh = self.inner.state.lock();
            if sh.state == WorkerState::Draining {
                sh.child = None; // a reload in progress owns the respawn
                return;
            }
            let pid = sh.pid;
            // Move the child out into the stale slot: the kill (blocking)
            // runs after the lock is released, off the state mutex.
            if let Some(c) = sh.child.take() {
                *self.inner.stale_child.lock() = Some(c);
            }
            sh.writer = None;
            // Crash-loop cap (F3.5): a crash loop must not become a spawn
            // storm. Decided here, under the lock, so the state a concurrent
            // `invoke()` sees is final.
            let now = Instant::now();
            let window = self.inner.spec.restart_window_ms;
            sh.restart_times
                .retain(|t| now.duration_since(*t) < Duration::from_millis(window));
            sh.restart_times.push(now);
            let capped = sh.restart_times.len()
                > usize::try_from(self.inner.spec.max_restarts_in_window).unwrap_or(5);
            // Set the FINAL state under the lock, so a concurrent `invoke()`
            // never observes a transient Restarting that then flips to
            // Failed after invoke() already decided to queue.
            sh.state = if capped {
                WorkerState::Failed
            } else {
                WorkerState::Restarting
            };
            let (in_flight, ids) = {
                let slots = self.inner.slots.lock();
                (slots.len(), slots.keys().copied().collect::<Vec<u64>>())
            };
            (pid, in_flight, ids, capped)
        };
        self.note_state(if capped {
            WorkerState::Failed
        } else {
            WorkerState::Restarting
        });
        // Lock released: the kill+wait (potentially slow) runs off the state
        // lock. A concurrent `invoke()` now sees Restarting/Failed and
        // queues or fails immediately.
        let parked_child = self.inner.stale_child.lock().take();
        if let Some(mut c) = parked_child {
            let _ = c.kill();
        }
        for id in ids {
            #[allow(clippy::significant_drop_tightening)] // reason: the slot
            // MutexGuard must stay held across the send.
            let slot = self.inner.slots.lock().remove(&id);
            if let Some(slot) = slot {
                let _ = slot.tx.send(Err(PiiError::Unavailable(format!(
                    "WorkerDied: {} pid {pid} code {code:?}",
                    self.inner.spec.name
                ))));
            }
        }
        self.inner
            .events
            .lock()
            .push(crate::supervisor::WorkerEvent::Died {
                worker: self.inner.spec.name.clone(),
                pid,
                code,
                in_flight: u32::try_from(in_flight).unwrap_or(u32::MAX),
            });
        // Capped: the worker is Failed, no respawn — matches the original
        // (pre-restructuring) early return here.
        if capped {
            return;
        }
        let backoff = self.inner.spec.restart_backoff_ms;
        let sup = Self {
            inner: Arc::clone(&self.inner),
        };
        std::thread::Builder::new()
            .name("pii-worker-respawn".into())
            .spawn(move || respawn(sup, backoff))
            .map_err(|e| PiiError::Internal(format!("respawn: {e}")))
            .ok();
    }

    fn drain_queue(&self) -> u32 {
        let queued: Vec<Queued> = std::mem::take(&mut *self.inner.queue.lock());
        let n = u32::try_from(queued.len()).unwrap_or(u32::MAX);
        for q in queued {
            let r = self.invoke_now(&q.op, &q.args, 30_000);
            let _ = q.tx.send(r);
        }
        n
    }

    /// Hot reload (F3.6): queue → drain → shutdown → respawn → hello → flush.
    ///
    /// # Errors
    /// [`PiiError::Unavailable`] when the fresh child never says hello.
    pub fn reload(&self, ctx: &Ctx) -> Result<ReloadReport, PiiError> {
        let (old_pid, in_flight) = {
            let sh = self.inner.state.lock();
            (
                sh.pid,
                u32::try_from(self.inner.slots.lock().len()).unwrap_or(0),
            )
        };
        self.inner.state.lock().state = WorkerState::Draining;
        self.note_state(WorkerState::Draining);

        // (2) wait for in-flight to reach zero, deadline 5 s.
        let deadline = Instant::now() + Duration::from_millis(RELOAD_DRAIN_DEADLINE_MS);
        while !self.inner.slots.lock().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let drained_in_flight = u32::try_from(self.inner.slots.lock().len()).unwrap_or(0);
        let _ = drained_in_flight;

        // (3) shutdown, wait ≤1 s, else kill. The child is parked in
        // `stale_child` and torn down AFTER the state lock is released —
        // `kill()` blocks until the process dies.
        let parked = {
            let mut sh = self.inner.state.lock();
            if let Some(w) = sh.writer.take()
                && w.send(WorkerFrame::Req {
                    id: 0,
                    op: String::from("shutdown"),
                    args: serde_json::Value::Null,
                })
                .is_err()
            {
                // already dead
            }
            sh.child.take().map(|mut c| {
                let _ = c.wait_timeout(Duration::from_millis(RELOAD_SHUTDOWN_MS));
                c
            })
        };
        if let Some(mut c) = parked
            && c.try_wait().is_ok_and(|e| e.is_none())
        {
            let _ = c.kill();
        }

        // (4)+(5) spawn the CURRENT program path and say hello.
        *self.inner.ctx.lock() = Some(ctx.clone());
        self.spawn_and_hello(ctx)?;
        let new_pid = self.inner.state.lock().pid;

        // (6) Ready; flush the queue in FIFO order.
        self.inner.state.lock().state = WorkerState::Ready;
        self.note_state(WorkerState::Ready);
        let drained = self.drain_queue();
        Ok(ReloadReport {
            old_pid,
            new_pid,
            drained,
            in_flight_at_start: in_flight,
        })
    }
}

/// Respawn after death (F3.5): hello → restart event → flush queue.
#[allow(clippy::needless_pass_by_value)] // reason: `sup` is moved into the
// respawn thread; it is not consumed here but must be owned by the thread.
fn respawn(sup: Supervisor, backoff_ms: u64) {
    // t0 includes the backoff sleep: took_ms reports true wall-clock
    // from death to ready, not just the post-sleep spawn time.
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_millis(backoff_ms));
    let ctx = sup.inner.ctx.lock().clone();
    let Some(ctx) = ctx else {
        sup.inner.state.lock().state = WorkerState::Failed;
        sup.note_state(WorkerState::Failed);
        return;
    };
    if sup.spawn_and_hello(&ctx).is_ok() {
        {
            let mut sh = sup.inner.state.lock();
            sh.restarts += 1;
            let restarts = sh.restarts;
            let pid = sh.pid;
            sup.inner
                .events
                .lock()
                .push(crate::supervisor::WorkerEvent::Restarted {
                    worker: sup.inner.spec.name.clone(),
                    pid,
                    restarts,
                    took_ms: u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
                });
            drop(sh);
            sup.inner.state.lock().state = WorkerState::Ready;
            sup.note_state(WorkerState::Ready);
            sup.drain_queue_pub();
        }
    } else {
        sup.inner.state.lock().state = WorkerState::Failed;
        sup.note_state(WorkerState::Failed);
    }
}

impl Supervisor {
    /// Queue flush used by the respawn path.
    pub(crate) fn drain_queue_pub(&self) -> u32 {
        let queued: Vec<Queued> = std::mem::take(&mut *self.inner.queue.lock());
        let n = u32::try_from(queued.len()).unwrap_or(u32::MAX);
        for q in queued {
            let r = self.invoke_now(&q.op, &q.args, 30_000);
            let _ = q.tx.send(r);
        }
        n
    }

    /// Releases the slots map when the supervisor is dropped implicitly.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.inner.slots.lock().len()
    }

    /// Test hook: number of queued requests.
    #[must_use]
    pub fn queued_count(&self) -> usize {
        self.inner.queue.lock().len()
    }

    /// Test hook: clears the slots map without touching the child.
    pub fn clear_slots(&self) {
        *self.inner.slots.lock() = BTreeMap::new();
    }
}

const _: Mutex<()> = Mutex::new(());

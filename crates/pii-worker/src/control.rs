//! Supervisor control plane (F3.5/F3.6): invoke, death, restart, reload.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_observe::{Event, Level, Observed, names};

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
        // The state decision and the request's registration are ONE critical
        // section (spec 120): a reload that marks Draining afterwards sees this
        // request in `slots` and waits for it; one that marked Draining first is
        // seen here, and the request is queued. No third outcome exists.
        let mut sh = self.inner.state.lock();
        match sh.state {
            WorkerState::Failed => {
                return Err(PiiError::Denied(format!(
                    "worker '{}' is failed",
                    self.inner.spec.name
                )));
            }
            WorkerState::Draining | WorkerState::Restarting => {
                // Queue (bounded 256); a full queue is retryable (F3.6). Pushed
                // under the state lock so a reload flushing the queue after it
                // flips to Ready cannot miss this entry.
                let (tx, rx) = channel();
                let mut q = self.inner.queue.lock();
                if q.len() >= MAX_QUEUE {
                    drop(q);
                    drop(sh);
                    self.note_refused(op, "queue_full");
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
                drop(sh);
                return rx
                    .recv_timeout(Duration::from_millis(timeout_ms.max(30_000)))
                    .map_err(|_| {
                        self.note_refused(op, "queue_timeout");
                        PiiError::Unavailable(format!("worker '{op}' queued too long"))
                    })?;
            }
            WorkerState::Starting | WorkerState::Ready => {}
        }
        let (id, rx) = self.register_locked(&mut sh);
        drop(sh);
        self.seam_invoke_gap();
        self.send_and_wait(id, &rx, op, args, timeout_ms)
    }

    /// Allocates an id and registers its reply slot while the caller holds the state lock.
    fn register_locked(
        &self,
        sh: &mut crate::supervisor::WorkerShared,
    ) -> (u64, Receiver<Result<serde_json::Value, PiiError>>) {
        let (tx, rx) = channel();
        let id = sh.next_id;
        sh.next_id += 1;
        self.inner.slots.lock().insert(id, Slot { tx });
        (id, rx)
    }

    pub(crate) fn invoke_now(
        &self,
        op: &str,
        args: &serde_json::Value,
        timeout_ms: u64,
    ) -> Result<serde_json::Value, PiiError> {
        let (id, rx) = {
            let mut sh = self.inner.state.lock();
            self.register_locked(&mut sh)
        };
        self.send_and_wait(id, &rx, op, args.clone(), timeout_ms)
    }

    fn send_and_wait(
        &self,
        id: u64,
        rx: &Receiver<Result<serde_json::Value, PiiError>>,
        op: &str,
        args: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<serde_json::Value, PiiError> {
        if let Err(e) = self.send(WorkerFrame::Req {
            id,
            op: String::from(op),
            args,
        }) {
            // Never leave a slot behind: a reload would wait on it.
            self.inner.slots.lock().remove(&id);
            return Err(e);
        }
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

    /// `reporter_pid` is the child whose reader saw EOF: a report from a child
    /// that is no longer the current one (its replacement is already up) is
    /// stale and must not be taken for a death of the replacement (spec 120 R6).
    pub(crate) fn death(&self, reporter_pid: u32, code: Option<i32>) {
        // Destructure a snapshot, then release the state lock before any
        // blocking child teardown: `kill()` waits for the process to die,
        // and `invoke()` must observe a post-death state (Restarting) and
        // queue or fail WITHOUT waiting for that teardown.
        let (pid, in_flight, ids, capped) = {
            let mut sh = self.inner.state.lock();
            if sh.pid != reporter_pid {
                return;
            }
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
        // Mark Draining and count in-flight in one critical section: the count is
        // exactly the set of requests `invoke` registered before the flip.
        let (old_pid, in_flight) = {
            let mut sh = self.inner.state.lock();
            sh.state = WorkerState::Draining;
            (
                sh.pid,
                u32::try_from(self.inner.slots.lock().len()).unwrap_or(u32::MAX),
            )
        };
        self.note_state(WorkerState::Draining);

        // (2) wait for in-flight to reach zero, deadline 5 s.
        let deadline = Instant::now() + Duration::from_millis(RELOAD_DRAIN_DEADLINE_MS);
        while !self.inner.slots.lock().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }

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

        // Anything still pending at the deadline will never be answered by the
        // child just shut down: fail it NOW with a named error and event rather
        // than leave its caller to wait out its own timeout.
        let aborted = self.abort_pending();
        self.seam_reload_gap();

        // (4)+(5) spawn the CURRENT program path and say hello.
        *self.inner.ctx.lock() = Some(ctx.clone());
        self.spawn_and_hello(ctx)?;
        let new_pid = self.inner.state.lock().pid;

        // (6) Ready; flush the queue in FIFO order.
        self.inner.state.lock().state = WorkerState::Ready;
        self.note_state(WorkerState::Ready);
        let drained = self.drain_queue();
        self.emit(
            &Event::new("worker", Level::Info, names::WORKER_RELOAD_COMPLETED)
                .field("worker", self.inner.spec.name.as_str())
                .field("old_pid", old_pid)
                .field("new_pid", new_pid)
                .field("in_flight_at_start", in_flight)
                .field("queued_flushed", drained)
                .field("aborted", aborted),
        );
        Ok(ReloadReport {
            old_pid,
            new_pid,
            drained,
            in_flight_at_start: in_flight,
            aborted,
        })
    }

    /// Fails every still-registered request with a named error; returns how many.
    fn abort_pending(&self) -> u32 {
        let pending: Vec<Slot> = std::mem::take(&mut *self.inner.slots.lock())
            .into_values()
            .collect();
        for slot in &pending {
            self.note_refused("-", "reload_deadline");
            let _ = slot.tx.send(Err(PiiError::Unavailable(format!(
                "ReloadAborted: worker '{}' did not answer within the {RELOAD_DRAIN_DEADLINE_MS}ms drain deadline",
                self.inner.spec.name
            ))));
        }
        u32::try_from(pending.len()).unwrap_or(u32::MAX)
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::sync::Arc;
    use std::sync::mpsc;

    use parking_lot::Mutex as StdMutex;
    use std::time::Duration;

    use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};

    use crate::{Supervisor, WorkerEvent, WorkerSpec, WorkerState};

    fn pii_bin() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target/debug/pii")
    }

    fn ctx() -> Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::SPAWN | CapKind::PROC;
        caps.proc_argv0 = ArgvScope::new(vec![pii_bin().to_string_lossy().into_owned()]);
        Ctx::root(caps, Budget::unlimited())
    }

    fn spec() -> WorkerSpec {
        WorkerSpec {
            name: "w".into(),
            program: pii_bin(),
            args: ["worker-exec", "--name", "w", "--tag", "a"]
                .map(String::from)
                .to_vec(),
            env: Vec::new(),
            max_in_flight: 0,
            ready_timeout_ms: 5_000,
            restart_backoff_ms: 50,
            restart_window_ms: 60_000,
            max_restarts_in_window: 5,
        }
    }

    /// One-shot rendezvous: the hook announces it arrived, then parks until released.
    fn parked_hook() -> (
        Arc<dyn Fn() + Send + Sync>,
        mpsc::Receiver<()>,
        mpsc::Sender<()>,
    ) {
        let (arrived_tx, arrived_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let go_rx = StdMutex::new(go_rx);
        let first = std::sync::atomic::AtomicBool::new(true);
        let hook = Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = arrived_tx.send(());
                let _ = go_rx.lock().recv();
            }
        });
        (hook, arrived_rx, go_tx)
    }

    /// Spec 120 R1: an `invoke` that decided "Ready" and is then overtaken by a
    /// whole reload (draining seen empty, old child shut down) must not be lost.
    #[test]
    fn invoke_overtaken_by_a_reload_is_not_dropped() {
        let ctx = ctx();
        let sup = Supervisor::new(spec());
        sup.start(&ctx).unwrap();

        let (invoke_hook, invoke_at_gap, invoke_go) = parked_hook();
        let (reload_hook, reload_at_gap, reload_go) = parked_hook();
        *sup.inner.seams.invoke_gap.lock() = Some(invoke_hook);
        *sup.inner.seams.reload_gap.lock() = Some(reload_hook);

        let a = {
            let (sup, ctx) = (sup.clone(), ctx.clone());
            std::thread::spawn(move || {
                sup.invoke(&ctx, "echo", serde_json::json!({"v": 1}), 10_000)
            })
        };
        invoke_at_gap.recv_timeout(Duration::from_secs(10)).unwrap();
        let b = {
            let (sup, ctx) = (sup.clone(), ctx);
            std::thread::spawn(move || sup.reload(&ctx))
        };
        // An invoke that registered atomically with its state decision already holds a
        // slot, so the reload must wait for it and never reaches its gap until released.
        if sup.slot_count() == 0 {
            reload_at_gap.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        invoke_go.send(()).unwrap();
        let r = a.join().unwrap();
        reload_go.send(()).unwrap();
        b.join().unwrap().unwrap();
        assert!(r.is_ok(), "invoke lost across a reload: {r:?}");
    }

    /// Spec 120 R6: the OLD child's reader reporting EOF after the reload finished
    /// must not be mistaken for a death of the NEW child.
    #[test]
    fn stale_reader_eof_after_reload_does_not_kill_the_new_worker() {
        let ctx = ctx();
        let sup = Supervisor::new(spec());
        sup.start(&ctx).unwrap();
        let old_pid = sup.info().pid;

        let (eof_tx, eof_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel();
        let go_rx = StdMutex::new(go_rx);
        let seen = std::sync::atomic::AtomicBool::new(false);
        *sup.inner.seams.reader_eof.lock() = Some(Arc::new(move |after| {
            if after {
                let _ = done_tx.send(());
            } else if !seen.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let _ = eof_tx.send(());
                let _ = go_rx.lock().recv();
            }
        }));

        let b = {
            let (sup, ctx) = (sup.clone(), ctx.clone());
            std::thread::spawn(move || sup.reload(&ctx))
        };
        // The old reader sees EOF as soon as the old child exits, then parks.
        eof_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        // Let the reload finish completely while the old reader is parked.
        b.join().unwrap().unwrap();
        let new_pid = sup.info().pid;
        assert_ne!(new_pid, old_pid);
        // Now the stale reader reports.
        go_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let died: Vec<WorkerEvent> = sup
            .events()
            .into_iter()
            .filter(|e| matches!(e, WorkerEvent::Died { .. }))
            .collect();
        assert!(died.is_empty(), "stale EOF recorded a death: {died:?}");
        let info = sup.info();
        assert_eq!(info.state, WorkerState::Ready);
        assert_eq!(info.pid, new_pid);
        sup.invoke(&ctx, "echo", serde_json::json!({"v": 1}), 10_000)
            .unwrap();
    }

    /// Spec 120 R5: a full drain queue refuses with a named error, an event, and a count.
    #[test]
    fn a_refused_request_emits_its_event_and_moves_the_counter() {
        use pii_observe::{Recorder, names};
        let rec = Arc::new(Recorder::new());
        let sup = Supervisor::new(spec());
        sup.set_observer(rec.clone());
        sup.inner.state.lock().state = WorkerState::Draining;
        for _ in 0..super::MAX_QUEUE {
            let (tx, _rx) = mpsc::channel();
            sup.inner.queue.lock().push(super::Queued {
                op: "echo".into(),
                args: serde_json::Value::Null,
                tx,
            });
        }
        assert_eq!(sup.refused_count(), 0);
        let r = sup.invoke(&ctx(), "echo", serde_json::json!({}), 100);
        assert!(r.unwrap_err().to_string().contains("reload queue full"));
        assert_eq!(sup.refused_count(), 1);
        let ev = rec.events_named(names::WORKER_REQUEST_REFUSED);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].fields.get("reason"), Some(&"queue_full".into()));
        assert!(names::ALL.contains(&names::WORKER_REQUEST_REFUSED));
        assert!(names::ALL.contains(&names::WORKER_RELOAD_COMPLETED));
        assert!(names::ALL.contains(&names::METRIC_WORKER_REQUESTS_REFUSED_TOTAL));
    }

    /// Spec 120 R3: what is still pending at the drain deadline is failed with a
    /// named error and counted, never left to vanish.
    #[test]
    fn pending_at_the_deadline_is_aborted_with_a_named_error() {
        use pii_observe::{Recorder, names};
        let rec = Arc::new(Recorder::new());
        let sup = Supervisor::new(spec());
        sup.set_observer(rec.clone());
        let (tx, rx) = mpsc::channel();
        sup.inner.slots.lock().insert(7, super::Slot { tx });
        assert_eq!(sup.abort_pending(), 1);
        let err = rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err();
        assert!(err.to_string().contains("ReloadAborted"), "{err}");
        assert_eq!(sup.slot_count(), 0);
        assert_eq!(sup.refused_count(), 1);
        let ev = rec.events_named(names::WORKER_REQUEST_REFUSED);
        assert_eq!(ev[0].fields.get("reason"), Some(&"reload_deadline".into()));
    }

    /// Spec 120 R5: a completed reload reports how many requests it drained, and
    /// "nothing was in flight" reads differently from "one was waited for".
    #[test]
    fn a_reload_reports_what_it_drained() {
        use pii_observe::{Recorder, names};
        let ctx = ctx();
        let rec = Arc::new(Recorder::new());
        let sup = Supervisor::new(spec());
        sup.set_observer(rec.clone());
        sup.start(&ctx).unwrap();

        // Quiet reload: nothing in flight.
        let quiet = sup.reload(&ctx).unwrap();
        assert_eq!((quiet.in_flight_at_start, quiet.aborted), (0, 0));

        // Busy reload: one request registered and parked before the reload starts.
        let (hook, at_gap, go) = parked_hook();
        *sup.inner.seams.invoke_gap.lock() = Some(hook);
        let a = {
            let (sup, ctx) = (sup.clone(), ctx.clone());
            std::thread::spawn(move || {
                sup.invoke(&ctx, "echo", serde_json::json!({"v": 1}), 10_000)
            })
        };
        at_gap.recv_timeout(Duration::from_secs(10)).unwrap();
        let b = {
            let sup = sup.clone();
            std::thread::spawn(move || sup.reload(&ctx))
        };
        while sup.info().state != WorkerState::Draining {
            std::thread::yield_now();
        }
        go.send(()).unwrap();
        a.join().unwrap().unwrap();
        let busy = b.join().unwrap().unwrap();
        assert_eq!((busy.in_flight_at_start, busy.aborted), (1, 0));

        let ev = rec.events_named(names::WORKER_RELOAD_COMPLETED);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].fields.get("in_flight_at_start"), Some(&0.into()));
        assert_eq!(ev[1].fields.get("in_flight_at_start"), Some(&1.into()));
        assert_eq!(ev[1].fields.get("aborted"), Some(&0.into()));
    }
}

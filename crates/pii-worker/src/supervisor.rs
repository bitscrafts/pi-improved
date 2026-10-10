//! Supervisor (F3.4–F3.6).
//!
//! Spawn via `ProcCap::spawn_child`, one reader, one writer, one stderr drain;
//! death is retryable; crash loops are capped; hot reload drops nothing.
//! Invoke/reload/restart internals live in `crate::control`.
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use pii_abi::PiiError;
use pii_core::{ChildProc, Ctx};

use crate::frame::{Hello, WORKER_PROTOCOL, WorkerFrame, parse_frame, write_frame};
use crate::spec::{WorkerInfo, WorkerSpec};

/// The child runs in the current directory; programs live in the scratch tree.
fn cwd_for() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf())
}

/// argv for the child: program + args.
fn child_argv(spec: &WorkerSpec) -> Vec<String> {
    let mut v = vec![spec.program.display().to_string()];
    v.extend(spec.args.iter().cloned());
    v
}

/// One pending request slot (oneshot-style demux by id).
pub(crate) struct Slot {
    pub tx: Sender<Result<serde_json::Value, PiiError>>,
}

/// Queued invocation during a drain (F3.6).
pub(crate) struct Queued {
    pub op: String,
    pub args: serde_json::Value,
    pub tx: Sender<Result<serde_json::Value, PiiError>>,
}

/// Bus-equivalent events, recordable in tests (F3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerEvent {
    /// `worker_died { worker, pid, code, in_flight }`.
    Died {
        /// Worker name.
        worker: String,
        /// Dead pid.
        pid: u32,
        /// Exit code if observed.
        code: Option<i32>,
        /// Requests in flight at death.
        in_flight: u32,
    },
    /// `worker_restarted { worker, pid, restarts, took_ms }`.
    Restarted {
        /// Worker name.
        worker: String,
        /// Fresh pid.
        pid: u32,
        /// Restarts since start.
        restarts: u32,
        /// Death-to-hello milliseconds.
        took_ms: u64,
    },
}

/// Shared supervision state.
pub(crate) struct WorkerShared {
    /// Lifecycle state.
    pub state: crate::spec::WorkerState,
    /// Current pid (0 = none).
    pub pid: u32,
    /// Tag from the last hello.
    pub tag: String,
    /// Restarts since start.
    pub restarts: u32,
    /// Timestamps of recent restarts (crash-loop window).
    pub restart_times: Vec<Instant>,
    /// Next request id.
    pub next_id: u64,
    /// The live child, if any.
    pub child: Option<ChildProc>,
    /// The writer channel.
    pub writer: Option<Sender<WorkerFrame>>,
}

/// Shared inner: one `Arc`, so clones share the registry (§4.3.5 lesson).
pub(crate) struct Inner {
    pub spec: WorkerSpec,
    /// The start context (cloned into respawn threads; SPAWN-capable).
    pub ctx: Mutex<Option<Ctx>>,
    pub slots: Mutex<BTreeMap<u64, Slot>>,
    pub queue: Mutex<Vec<Queued>>,
    pub state: Mutex<WorkerShared>,
    pub events: Mutex<Vec<WorkerEvent>>,
    /// The `take()`n child awaiting its blocking `kill()` teardown, parked
    /// by `control::death` so the state lock is never held across the kill.
    /// Taken by `death` right after releasing the state lock.
    pub stale_child: Mutex<Option<ChildProc>>,
    /// Set once by [`Supervisor::set_observer`].
    pub observer: std::sync::OnceLock<Arc<dyn pii_observe::Observer>>,
    /// Requests refused or cancelled around a drain/reload (spec 120).
    pub refused: std::sync::atomic::AtomicU64,
    /// Test hooks that force an interleaving (spec 120).
    #[cfg(test)]
    pub seams: crate::seams::Seams,
}

impl WorkerShared {
    pub(crate) fn new() -> Self {
        Self {
            state: crate::spec::WorkerState::Starting,
            pid: 0,
            tag: String::new(),
            restarts: 0,
            restart_times: Vec::new(),
            next_id: 1,
            child: None,
            writer: None,
        }
    }
}

/// A supervised worker.
pub struct Supervisor {
    pub(crate) inner: Arc<Inner>,
}

impl Clone for Supervisor {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Supervisor {
    /// Creates a supervisor for `spec` (not yet started).
    #[must_use]
    pub fn new(spec: WorkerSpec) -> Self {
        Self {
            inner: Arc::new(Inner {
                spec: spec.with_defaults(),
                ctx: Mutex::new(None),
                slots: Mutex::new(BTreeMap::new()),
                queue: Mutex::new(Vec::new()),
                state: Mutex::new(WorkerShared::new()),
                events: Mutex::new(Vec::new()),
                stale_child: Mutex::new(None),
                observer: std::sync::OnceLock::new(),
                refused: std::sync::atomic::AtomicU64::new(0),
                #[cfg(test)]
                seams: crate::seams::Seams::default(),
            }),
        }
    }

    /// Snapshot for `info()`.
    #[must_use]
    pub fn info(&self) -> WorkerInfo {
        let sh = self.inner.state.lock();
        WorkerInfo {
            name: self.inner.spec.name.clone(),
            state: sh.state,
            pid: sh.pid,
            tag: sh.tag.clone(),
            restarts: sh.restarts,
            in_flight: u32::try_from(self.inner.slots.lock().len()).unwrap_or(u32::MAX),
        }
    }

    /// Recorded bus events (tests).
    #[must_use]
    pub fn events(&self) -> Vec<WorkerEvent> {
        self.inner.events.lock().clone()
    }

    /// Starts the child and waits for `hello` (F3.4).
    ///
    /// # Errors
    /// Spawn failure or `hello` timeout → [`PiiError::Unavailable`].
    pub fn start(&self, ctx: &Ctx) -> Result<(), PiiError> {
        *self.inner.ctx.lock() = Some(ctx.clone());
        self.spawn_and_hello(ctx)
    }

    #[allow(clippy::too_many_lines)] // reason: spawn + 3 threads + hello is one
    // cohesive F3.4 unit; splitting it would obscure the handshake ordering.
    pub(crate) fn spawn_and_hello(&self, ctx: &Ctx) -> Result<(), PiiError> {
        self.note_state(crate::spec::WorkerState::Starting);
        let spec = &self.inner.spec;
        // ETXTBSY retry: a concurrent exec/rename on the program file makes
        // one spawn fail with "Text file busy"; a short bounded retry is
        // the operator-honest fix (never a panic, never a busy-wait).
        let mut proc = None;
        for attempt in 0..10u32 {
            match ctx
                .proc()?
                .spawn_child(&child_argv(spec), &cwd_for(), &spec.env)
            {
                Ok(p) => {
                    proc = Some(p);
                    break;
                }
                Err(e) if e.to_string().contains("Text file busy") && attempt < 9 => {
                    std::thread::sleep(Duration::from_millis(20 * u64::from(attempt + 1)));
                }
                Err(e) => return Err(e),
            }
        }
        let proc = proc.ok_or_else(|| {
            PiiError::Unavailable(format!(
                "worker '{}' program stayed busy across retries",
                spec.name
            ))
        })?;
        let pid = proc.pid();
        let mut child = Some(proc);
        let Some(mut writer) = child.as_mut().and_then(ChildProc::stdin) else {
            return Err(PiiError::Internal(String::from("worker: no stdin")));
        };
        let (tx, rx) = channel::<WorkerFrame>();

        // stderr drain: never blocks the supervisor (F3.2).
        if let Some(stderr) = child.as_mut().and_then(ChildProc::stderr) {
            let mut r = BufReader::new(stderr);
            std::thread::Builder::new()
                .name("pii-worker-stderr".into())
                .spawn(move || {
                    let mut line = Vec::new();
                    while matches!(r.read_until(b'\n', &mut line), Ok(n) if n > 0) {
                        line.clear(); // drained and dropped: notes are logs only
                    }
                })
                .map_err(|e| PiiError::Internal(format!("stderr drain: {e}")))?;
        }

        // reader thread: stdout → demux by id (F3.4); EOF = death.
        if let Some(stdout) = child.as_mut().and_then(ChildProc::stdout) {
            let reader_sup = Self {
                inner: Arc::clone(&self.inner),
            };
            let mut r = BufReader::new(stdout);
            std::thread::Builder::new()
                .name("pii-worker-reader".into())
                .spawn(move || {
                    let mut line = Vec::new();
                    loop {
                        line.clear();
                        match r.read_until(b'\n', &mut line) {
                            Ok(0) | Err(_) => {
                                reader_sup.seam_reader_eof(false);
                                reader_sup.death(pid, None);
                                reader_sup.seam_reader_eof(true);
                                return;
                            }
                            Ok(_) => {}
                        }
                        if line.len() > crate::frame::MAX_FRAME {
                            reader_sup.death(pid, None);
                            return;
                        }
                        let s = String::from_utf8_lossy(&line);
                        if let Ok(frame) = parse_frame(s.trim_end_matches(['\n', '\r'])) {
                            reader_sup.apply(frame);
                        }
                    }
                })
                .map_err(|e| PiiError::Internal(format!("reader spawn: {e}")))?;
        }

        // writer thread (F3.4).
        std::thread::Builder::new()
            .name("pii-worker-writer".into())
            .spawn(move || {
                for f in rx {
                    if let Ok(bytes) = write_frame(&f)
                        && writer.write_all(&bytes).is_err()
                    {
                        return;
                    }
                }
            })
            .map_err(|e| PiiError::Internal(format!("writer spawn: {e}")))?;

        // hello handshake (F3.4).
        let hello_id = {
            let mut sh = self.inner.state.lock();
            sh.pid = pid;
            sh.child = child;
            sh.writer = Some(tx);
            let id = sh.next_id;
            sh.next_id += 1;
            id
        };
        match self.wait_hello(hello_id, pid) {
            Ok(tag) => {
                {
                    let mut sh = self.inner.state.lock();
                    sh.state = crate::spec::WorkerState::Ready;
                    sh.tag = tag;
                }
                self.note_state(crate::spec::WorkerState::Ready);
                self.inner.slots.lock().remove(&hello_id);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn wait_hello(&self, hello_id: u64, pid: u32) -> Result<String, PiiError> {
        let (rtx, rrx) = channel();
        self.inner.slots.lock().insert(hello_id, Slot { tx: rtx });
        self.send(WorkerFrame::Req {
            id: hello_id,
            op: String::from("hello"),
            args: serde_json::json!({}),
        })?;
        let ready = self.inner.spec.ready_timeout_ms;
        let deadline = Instant::now() + Duration::from_millis(ready);
        loop {
            match rrx.recv_timeout(Duration::from_millis(10)) {
                Ok(Ok(v)) => {
                    let h: Hello = serde_json::from_value(v)
                        .map_err(|e| PiiError::Invalid(format!("hello shape: {e}")))?;
                    if h.protocol != WORKER_PROTOCOL {
                        return Err(PiiError::Invalid(format!(
                            "worker protocol {} != {WORKER_PROTOCOL}",
                            h.protocol
                        )));
                    }
                    return Ok(h.tag);
                }
                Ok(Err(e)) => return Err(e),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        self.inner.state.lock().state = crate::spec::WorkerState::Failed;
                        self.note_state(crate::spec::WorkerState::Failed);
                        return Err(PiiError::Unavailable(format!(
                            "WorkerDied: {} pid {pid} — no hello within {ready}ms",
                            self.inner.spec.name
                        )));
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(PiiError::Unavailable(String::from(
                        "worker died during hello",
                    )));
                }
            }
        }
    }

    pub(crate) fn send(&self, frame: WorkerFrame) -> Result<(), PiiError> {
        self.inner
            .state
            .lock()
            .writer
            .as_ref()
            .ok_or_else(|| PiiError::Unavailable(String::from("worker writer closed")))?
            .send(frame)
            .map_err(|_| PiiError::Unavailable(String::from("worker writer closed")))
    }
}

//! Worker spec (F3.3): every field is a supervision-relevant choice; there
//! is no `Default` for the whole struct.

/// A worker's configuration (F3.3).
#[derive(Debug, Clone)]
pub struct WorkerSpec {
    /// Worker name (also the grant/rpc key).
    pub name: String,
    /// Program to `spawn_child` (replaced on disk = hot reload).
    pub program: std::path::PathBuf,
    /// argv after the program.
    pub args: Vec<String>,
    /// Extra child environment.
    pub env: Vec<(String, String)>,
    /// Concurrent in-flight cap.
    pub max_in_flight: u32,
    /// `hello` deadline after spawn.
    pub ready_timeout_ms: u64,
    /// Delay before respawn.
    pub restart_backoff_ms: u64,
    /// The crash-loop window.
    pub restart_window_ms: u64,
    /// Restart cap inside the window.
    pub max_restarts_in_window: u32,
}

impl WorkerSpec {
    /// Applies the documented defaults to the unset knobs.
    #[must_use]
    pub fn with_defaults(mut self) -> Self {
        if self.max_in_flight == 0 {
            self.max_in_flight = 16;
        }
        if self.ready_timeout_ms == 0 {
            self.ready_timeout_ms = 2_000;
        }
        if self.restart_backoff_ms == 0 {
            self.restart_backoff_ms = 50;
        }
        if self.restart_window_ms == 0 {
            self.restart_window_ms = 60_000;
        }
        if self.max_restarts_in_window == 0 {
            self.max_restarts_in_window = 5;
        }
        self
    }
}

/// Worker lifecycle states (F3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    /// Starting (waiting for `hello`).
    Starting,
    /// Serving invokes.
    Ready,
    /// Draining for a hot reload; new invokes are queued.
    Draining,
    /// Restart pending after death.
    Restarting,
    /// Crash-loop cap hit; invokes are refused.
    Failed,
}

/// Snapshot for `worker.list` / `info()`.
#[derive(Debug, Clone)]
pub struct WorkerInfo {
    /// Worker name.
    pub name: String,
    /// Lifecycle state.
    pub state: WorkerState,
    /// Current pid (0 when not running).
    pub pid: u32,
    /// Build tag from the last `hello`.
    pub tag: String,
    /// Restarts since start.
    pub restarts: u32,
    /// Live in-flight requests.
    pub in_flight: u32,
}

/// Hot-reload outcome (F3.6).
#[derive(Debug, Clone)]
pub struct ReloadReport {
    /// Pid of the replaced child.
    pub old_pid: u32,
    /// Pid of the fresh child.
    pub new_pid: u32,
    /// Queued requests drained after the swap.
    pub drained: u32,
    /// In-flight count when the reload started (they were waited for, not dropped).
    pub in_flight_at_start: u32,
    /// In-flight requests still pending at the drain deadline, cancelled with a named error.
    pub aborted: u32,
}

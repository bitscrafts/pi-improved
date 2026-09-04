//! Daemon-wide runtime context (F5.1/F5.12).
//!
//! The resources `sys.status`, `worker.list`, `cron.list`, `channel.list`
//! and `memory.*` read. Held by [`crate::Engine`] as
//! `Option<Arc<DaemonContext>>` so a bare `Engine` (oneshot/tui/tests)
//! reports an empty daemon context rather than needing a second
//! constructor path.

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::Mutex;

use pii_channels::Channel;
use pii_proto::sys::{ChannelStatus, ListenerStatus, MemoryStatus, StatusResult, WorkerStatus};

/// One configured channel's runtime state.
pub struct ChannelRuntime {
    /// The channel implementation.
    pub channel: Arc<dyn Channel>,
    /// Poll interval.
    pub poll_interval_ms: u64,
    /// Graph run per inbound message (F6.5).
    pub graph: String,
}

/// One `[[cron]]` entry, tracked so a finished run can be re-armed (F5.8).
#[derive(Debug, Clone)]
pub struct CronEntry {
    /// Cron entry name.
    pub name: String,
    /// Graph to run.
    pub graph: String,
    /// Display-only schedule mirror.
    pub schedule: String,
    /// Extra run input.
    pub input: serde_json::Value,
}

/// Everything a daemon adds on top of a bare [`crate::Engine`].
pub struct DaemonContext {
    /// The egress service (may be absent when `[net]` grants nothing).
    pub net: Arc<pii_net::NetService>,
    /// Supervised workers, keyed by name.
    pub workers: Mutex<BTreeMap<String, pii_worker::Supervisor>>,
    /// Configured channels, keyed by name.
    pub channels: Mutex<BTreeMap<String, ChannelRuntime>>,
    /// The channel cursor table.
    pub channel_store: Arc<pii_store::channel::ChannelStore>,
    /// Configured cron entries, keyed by name.
    pub cron: Mutex<BTreeMap<String, CronEntry>>,
    /// Run id -> cron name, for runs started by the cron driver (F5.8): a
    /// `RunFinished` for one of these starts the next firing.
    pub cron_runs: Mutex<BTreeMap<String, String>>,
    /// Run id -> (channel, chat), for runs started by the channel router
    /// (F6.5): a `RunFinished` for one of these sends the reply.
    pub channel_runs: Mutex<BTreeMap<String, (String, String)>>,
    /// Directory panic-hook crash reports are written to (F5.4).
    pub crashes_dir: std::path::PathBuf,
    /// The config file path (F5.12 `sys.reload` re-reads this).
    pub config_path: std::path::PathBuf,
}

/// Resident set size in KiB from `/proc/self/status` (F5.12); `0` when the
/// line cannot be read (never fatal — the daemon is not a `/proc` client of
/// last resort).
#[must_use]
pub fn rss_kb() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

impl DaemonContext {
    /// Assembles the `sys.status` result (F5.12). `memory_docs` comes from
    /// the caller (`engine.memory.n_docs()`) since the memory store itself
    /// lives on [`crate::Engine`], not here.
    #[must_use]
    pub fn status(
        &self,
        uptime_ms: u64,
        runs_active: u32,
        sessions: u32,
        sandbox: &str,
        memory_docs: u64,
    ) -> StatusResult {
        let workers = self
            .workers
            .lock()
            .values()
            .map(|s| {
                let info = s.info();
                WorkerStatus {
                    name: info.name,
                    state: worker_state_str(info.state),
                    pid: info.pid,
                    restarts: info.restarts,
                }
            })
            .collect();
        let listeners = self
            .net
            .listeners()
            .into_iter()
            .map(|l| ListenerStatus {
                id: l.id,
                protocol: format!("{:?}", l.proto),
                addr: l.addr.to_string(),
            })
            .collect();
        let channels = self
            .channels
            .lock()
            .values()
            .map(|c| {
                let cursor = self
                    .channel_store
                    .get(c.channel.name())
                    .ok()
                    .flatten()
                    .unwrap_or(pii_store::channel::ChannelCursor {
                        cursor: 0,
                        run_id: String::new(),
                        errors: 0,
                    });
                ChannelStatus {
                    name: c.channel.name().to_owned(),
                    kind: c.channel.kind().to_owned(),
                    errors: cursor.errors,
                    cursor: cursor.cursor,
                }
            })
            .collect();
        StatusResult {
            uptime_ms,
            runs_active,
            sessions,
            sandbox_profile: sandbox.to_owned(),
            protocol: pii_proto::PROTOCOL_VERSION.to_owned(),
            rss_kb: rss_kb(),
            workers,
            listeners,
            channels,
            memory: MemoryStatus { docs: memory_docs },
        }
    }
}

fn worker_state_str(s: pii_worker::spec::WorkerState) -> String {
    match s {
        pii_worker::spec::WorkerState::Starting => "starting",
        pii_worker::spec::WorkerState::Ready => "ready",
        pii_worker::spec::WorkerState::Draining => "draining",
        pii_worker::spec::WorkerState::Restarting => "restarting",
        pii_worker::spec::WorkerState::Failed => "failed",
    }
    .to_owned()
}

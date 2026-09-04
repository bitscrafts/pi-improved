//! `sys.*` params and results (F1.4).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Who is calling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerInfo {
    /// Program name.
    pub name: String,
    /// Program version.
    pub version: String,
    /// Build identifier.
    pub build: String,
}

/// `sys.hello` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelloParams {
    /// Protocol the client speaks.
    pub protocol: String,
    /// Client identity.
    pub client: PeerInfo,
    /// Encodings the client accepts.
    pub encodings: Vec<String>,
    /// Optional feature flags.
    pub features: Vec<String>,
}

/// Server resource limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Limits {
    /// Largest frame the server accepts.
    pub max_frame_bytes: u64,
    /// Subscriptions per connection.
    pub max_subscriptions: u32,
}

/// `sys.hello` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelloResult {
    /// Protocol the server speaks (always [`crate::PROTOCOL_VERSION`]).
    pub protocol: String,
    /// Server identity.
    pub server: PeerInfo,
    /// Negotiated encoding (always `"json"` in this phase).
    pub encoding: String,
    /// Capability strings.
    pub capabilities: Vec<String>,
    /// Resource limits.
    pub limits: Limits,
}

/// One supervised worker, as reported by `sys.status`/`worker.list` (F5.12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerStatus {
    /// Worker name.
    pub name: String,
    /// Lifecycle state (`starting|ready|draining|restarting|failed`).
    pub state: String,
    /// Current pid (0 when not running).
    pub pid: u32,
    /// Restarts since start.
    pub restarts: u32,
}

/// One open listener, as reported by `sys.status` (F5.12, `pii net ls`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListenerStatus {
    /// Listener id.
    pub id: u64,
    /// Protocol served.
    pub protocol: String,
    /// Bound address.
    pub addr: String,
}

/// One configured channel, as reported by `sys.status`/`channel.list` (F5.12,
/// F6.8: never carries the token).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChannelStatus {
    /// Channel name.
    pub name: String,
    /// Channel kind (`discord`, `cli`, ...).
    pub kind: String,
    /// API errors absorbed so far (F6.3).
    pub errors: u64,
    /// Last processed cursor.
    pub cursor: u64,
}

/// Memory subsystem summary (F5.12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryStatus {
    /// Documents currently indexed.
    pub docs: u64,
}

/// `sys.status` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusResult {
    /// Server uptime.
    pub uptime_ms: u64,
    /// Currently running graphs.
    pub runs_active: u32,
    /// Open sessions.
    pub sessions: u32,
    /// Active sandbox profile name.
    pub sandbox_profile: String,
    /// Protocol version served (always [`crate::PROTOCOL_VERSION`]).
    pub protocol: String,
    /// Resident set size in KiB, from `/proc/self/status` (F5.12).
    pub rss_kb: u64,
    /// Supervised workers.
    pub workers: Vec<WorkerStatus>,
    /// Open listeners (the control socket plus any `NetService` listeners).
    pub listeners: Vec<ListenerStatus>,
    /// Configured channels.
    pub channels: Vec<ChannelStatus>,
    /// Memory subsystem summary.
    pub memory: MemoryStatus,
}

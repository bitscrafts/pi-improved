//! `sys.*` params and results (F1.4).

use std::collections::BTreeMap;

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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryStatus {
    /// Documents currently indexed.
    pub docs: u64,
}

/// One durable store file, as reported by `sys.status` (spec 99).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoreStatus {
    /// Role of the store (`main`, `memory`).
    pub name: String,
    /// Resolved path of the file the process has open.
    pub path: String,
    /// File size in bytes.
    pub bytes: u64,
    /// Persisted format version; `None` when the store records none.
    pub format_version: Option<u64>,
    /// File modification time (ms since the Unix epoch), when readable.
    pub last_write_ms: Option<u64>,
    /// Entry count per table (or subsystem-specific counts).
    pub entries: BTreeMap<String, u64>,
}

/// One filesystem root the process actually allows (spec 99 R2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MountStatus {
    /// What the root is for (`workspace`, `channels_scope`, `runtime_dir`).
    pub name: String,
    /// The RESOLVED absolute path, never a profile name.
    pub root: String,
    /// `rw` or `ro`.
    pub mode: String,
    /// The policy that granted the root.
    pub policy: String,
}

/// Which binary is answering (spec 99 R3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildIdentity {
    /// Crate version.
    pub version: String,
    /// Source revision the binary was built from (`unknown` when the build
    /// had no git checkout).
    pub revision: String,
    /// Target triple.
    pub target: String,
}

/// Which config the daemon loaded: path and a hash of its contents (never
/// the contents).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigIdentity {
    /// Config file path.
    pub path: String,
    /// Hex SHA-256 of the file's bytes; `None` when it cannot be read.
    pub sha256: Option<String>,
}

/// Cheap totals (spec 99 R4). `memory_docs` mirrors `memory.docs`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Counts {
    /// Runs in the store.
    pub runs: u64,
    /// Journal events in the store.
    pub journal_events: u64,
    /// Approvals awaiting a decision.
    pub approvals_pending: u64,
    /// Configured cron entries.
    pub cron_entries: u64,
    /// Memory documents.
    pub memory_docs: u64,
    /// MQTT messages accepted from brokers (spec 117).
    #[serde(default)]
    pub mqtt_received: u64,
    /// MQTT messages dropped and counted (spec 117).
    #[serde(default)]
    pub mqtt_dropped: u64,
    /// MQTT messages published (spec 117).
    #[serde(default)]
    pub mqtt_published: u64,
}

/// One subsystem's self-reported health (mirror of `pii_observe::SubsystemHealth`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubsystemStatus {
    /// Subsystem name.
    pub name: String,
    /// `ok`, `degraded` or `down`.
    pub state: String,
    /// Human-readable detail, if any.
    pub detail: Option<String>,
    /// What the subsystem actually is: paths, versions, counts.
    pub identity: BTreeMap<String, String>,
}

/// Run-lifecycle latency for one outcome, from a bounded in-process histogram
/// (spec 115 R4). Percentiles are bucket upper bounds; see
/// `pii_observe::LATENCY_RESOLUTION` for exactly what that means.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LatencyStatus {
    /// `succeeded`, `failed`, `suspended` or `refused`.
    pub outcome: String,
    /// Runs recorded with this outcome since the daemon started.
    pub count: u64,
    /// 50th percentile, ms.
    pub p50_ms: u64,
    /// 95th percentile, ms.
    pub p95_ms: u64,
    /// 99th percentile, ms.
    pub p99_ms: u64,
    /// Largest observed, ms (exact).
    pub max_ms: u64,
}

/// `sys.status` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// Durable stores (spec 99). Additive: absent from older servers.
    #[serde(default)]
    pub stores: Vec<StoreStatus>,
    /// Roots the process allows (spec 99).
    #[serde(default)]
    pub mounts: Vec<MountStatus>,
    /// Binary identity (spec 99).
    #[serde(default)]
    pub build: BuildIdentity,
    /// Config identity (spec 99).
    #[serde(default)]
    pub config: ConfigIdentity,
    /// Totals (spec 99).
    #[serde(default)]
    pub counts: Counts,
    /// Per-subsystem health reports (spec 99 R5).
    #[serde(default)]
    pub subsystems: Vec<SubsystemStatus>,
    /// Run-lifecycle latency by outcome (spec 115). Additive.
    #[serde(default)]
    pub latency: Vec<LatencyStatus>,
    /// Standing operator notes (spec 115), e.g. that the store never shrinks.
    #[serde(default)]
    pub notes: Vec<String>,
    /// Bytes of RSS growth per run over the recent window of polls (spec 116);
    /// 0 until enough runs separate the window ends. Additive.
    #[serde(default)]
    pub rss_bytes_per_run: u64,
}

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
}

//! `run.*` and `graph.*` params/results (F1.6).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::Event;

/// Run lifecycle (wire copy of `RunStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WireRunStatus {
    /// Handed an id and held by the broker, but its journal has not
    /// materialised yet (spec 115). Only `run.get` answers this; it is never
    /// stored and `run.list` cannot filter on it.
    Accepted,
    /// Executing.
    Running,
    /// Awaiting resume.
    Suspended,
    /// Completed successfully.
    Succeeded,
    /// Failed.
    Failed,
    /// Refused before any agent call spawned (spec 221).
    Refused,
}

/// `run.start` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunStartParams {
    /// Graph name (as stored via `graph.apply`).
    pub graph: String,
    /// Run input.
    pub input: serde_json::Value,
    /// Optional capability names.
    pub caps: Option<Vec<String>>,
}

/// `run.start` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunStartResult {
    /// The new run id.
    pub run_id: String,
}

/// Wire run header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireRunHeader {
    /// Run id.
    pub run_id: String,
    /// Graph name or hash.
    pub graph: String,
    /// Status.
    pub status: WireRunStatus,
    /// Next journal sequence.
    pub seq: u64,
    /// Wall duration in ms.
    pub dur_ms: u32,
}

/// `run.list` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunListParams {
    /// Filter by status.
    pub status: Option<WireRunStatus>,
    /// Return runs with id counter greater than this.
    pub since: Option<u64>,
}

/// One journalled event with its sequence number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SeqEvent {
    /// Journal sequence.
    pub seq: u64,
    /// The event.
    pub event: Event,
}

/// `run.trace` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TraceResult {
    /// Run header.
    pub header: WireRunHeader,
    /// Journal events in order.
    pub events: Vec<SeqEvent>,
    /// The journal as the store holds it: one `{"seq":<u64>,"event":<RunEvent>}` object per entry, in order
    /// (spec 220 R3). Additive: absent from older daemons.
    #[serde(default)]
    pub journal: Vec<serde_json::Value>,
}

/// `graph.apply` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GraphApplyParams {
    /// Graph name.
    pub name: String,
    /// TOML source.
    pub toml: String,
}

/// `graph.apply` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GraphApplyResult {
    /// Graph name.
    pub name: String,
    /// Content hash.
    pub hash: String,
}

/// `graph.lint` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GraphLintParams {
    /// TOML source.
    pub toml: String,
}

/// `graph.lint` result: `V<n>`-prefixed strings verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GraphLintResult {
    /// True when validation passed.
    pub ok: bool,
    /// Validation errors.
    pub errors: Vec<String>,
    /// Validation warnings.
    pub warnings: Vec<String>,
}

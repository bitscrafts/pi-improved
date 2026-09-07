//! `session.*` params and results (F1.5).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One content block. Closed: only text in this phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// A UTF-8 text chunk.
    Text {
        /// The text.
        text: String,
    },
}

/// Lifecycle of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// No active run.
    Idle,
    /// A run is executing.
    Running,
    /// Closed.
    Closed,
}

/// `session.create` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionCreateParams {
    /// Working directory for the session's tools.
    pub cwd: String,
    /// Model override.
    pub model: Option<String>,
    /// Tool allowlist.
    pub tools: Vec<String>,
    /// Graph override.
    pub graph: Option<String>,
}

/// `session.create` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionCreateResult {
    /// The new session id.
    pub session_id: String,
}

/// `session.prompt` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionPromptParams {
    /// Target session.
    pub session_id: String,
    /// Prompt content.
    pub content: Vec<ContentBlock>,
    /// Optional extra graph input (F5): merged into the run input so
    /// verify-fix graphs receive `test_command`/`workspace`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
}

/// `session.prompt` result — returned before the run finishes (< 50 ms).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionPromptResult {
    /// The run executing the prompt.
    pub run_id: String,
}

/// A message in the session transcript (wire shape; duplicated by design).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireMessage {
    /// `system|user|assistant|tool`.
    pub role: String,
    /// Text content.
    pub content: String,
}

/// `session.state` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionStateResult {
    /// Session id.
    pub session_id: String,
    /// Session cwd.
    pub cwd: String,
    /// Session status.
    pub status: SessionStatus,
    /// Transcript so far.
    pub messages: Vec<WireMessage>,
    /// Input tokens (server-computed).
    pub tokens_in: u64,
    /// Output tokens (server-computed).
    pub tokens_out: u64,
    /// Active run, if any.
    pub run_id: Option<String>,
}

/// One entry in `session.list` (spec 16 part2 R3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionListEntry {
    /// Session id.
    pub session_id: String,
    /// Session cwd.
    pub cwd: String,
    /// Session status.
    pub status: SessionStatus,
    /// Active run, if any.
    pub run_id: Option<String>,
}

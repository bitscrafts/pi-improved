//! `pii-model`: the model provider seam (F3.1–F3.4).
//!
//! [`ModelProvider`] is **sync** on purpose: `GraphNode::run` is sync and
//! runs execute on a blocking thread. Providers never read configuration
//! themselves except from the explicit spec string handed to
//! [`provider_from_env`] by the CLI.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pii_abi::PiiError;
use pii_core::Ctx;
use serde::{Deserialize, Serialize};

/// Chat roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// System prompt.
    System,
    /// User turn.
    User,
    /// Assistant turn.
    Assistant,
    /// Tool result.
    Tool,
}

/// One tool invocation the assistant requested.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider call id.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// JSON arguments.
    pub arguments: serde_json::Value,
}

/// A chat transcript message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Role.
    pub role: Role,
    /// Text content.
    pub content: String,
    /// Tool result id when `role == Tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool calls requested by an assistant turn.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

impl ChatMessage {
    /// Convenience constructor.
    #[must_use]
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }
}

/// A tool exposed to the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Tool name.
    pub name: String,
    /// One-line description.
    pub description: String,
    /// JSON schema of the arguments.
    pub schema: serde_json::Value,
}

/// A chat request.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// Model name.
    pub model: String,
    /// Transcript.
    pub messages: Vec<ChatMessage>,
    /// Tools the model may call.
    pub tools: Vec<ToolSpec>,
    /// Output cap.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
}

/// A completed chat turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Assistant text (may be empty when only tool calls are present).
    pub text: String,
    /// Tool calls requested.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// Input token count as reported.
    #[serde(default)]
    pub tokens_in: u64,
    /// Output token count as reported.
    #[serde(default)]
    pub tokens_out: u64,
}

/// A chat completion backend.
pub trait ModelProvider: Send + Sync {
    /// Provider name.
    fn name(&self) -> &str;
    /// One chat completion over the transcript.
    ///
    /// # Errors
    /// Provider-specific; retryable errors surface as [`PiiError::Timeout`].
    fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError>;
}

/// Deterministic fixture provider (F3.2): turn *n* on call *n*.
pub struct ScriptedProvider {
    turns: Vec<ChatResponse>,
    calls: AtomicUsize,
}

/// Wire shape for `scripted:<path>` fixtures: `{"turns": [ChatResponse, ...]}`.
#[derive(Deserialize)]
struct ScriptedFixture {
    turns: Vec<ChatResponse>,
}

impl ScriptedProvider {
    /// Loads `{"turns": [ChatResponse, ...]}`.
    ///
    /// # Errors
    /// I/O or JSON failure → [`PiiError::Internal`] / [`PiiError::Invalid`].
    pub fn from_fixture(path: &Path) -> Result<Self, PiiError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| PiiError::Internal(format!("fixture {}: {e}", path.display())))?;
        let f: ScriptedFixture = serde_json::from_str(&raw)
            .map_err(|e| PiiError::Invalid(format!("fixture {}: {e}", path.display())))?;
        Ok(Self {
            turns: f.turns,
            calls: AtomicUsize::new(0),
        })
    }
}

impl ModelProvider for ScriptedProvider {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn chat(&self, _ctx: &Ctx, _req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed);
        self.turns
            .get(n)
            .cloned()
            .ok_or_else(|| PiiError::Invalid(String::from("scripted provider exhausted")))
    }
}

/// Resolves a model spec: `scripted:<path>` (F3.4). The `ollama:` backend
/// lands with the HTTP provider pass; asking for it here is `Invalid`.
///
/// # Errors
/// [`PiiError::Invalid`] for an unknown spec or fixture failure.
pub fn provider_from_spec(spec: &str) -> Result<Arc<dyn ModelProvider>, PiiError> {
    let Some((kind, rest)) = spec.split_once(':') else {
        return Err(PiiError::Invalid(format!("unknown model spec '{spec}'")));
    };
    match kind {
        "scripted" => Ok(Arc::new(ScriptedProvider::from_fixture(Path::new(rest))?)),
        other => Err(PiiError::Invalid(format!(
            "unknown model spec kind '{other}' (scripted:<path> is supported)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{ChatRequest, ModelProvider, Role, ScriptedProvider, provider_from_spec};

    #[test]
    fn scripted_returns_turns_in_order() {
        let dir = tempfile::Builder::new()
            .prefix("pii-model-")
            .tempdir_in("/projects/pi-improved/_tmp")
            .unwrap();
        let f = dir.path().join("two.json");
        std::fs::write(&f, r#"{"turns":[{"text":"one"},{"text":"two"}]}"#).unwrap();
        let p = ScriptedProvider::from_fixture(&f).unwrap();
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![super::ChatMessage::new(Role::User, "hi")],
            tools: Vec::new(),
            max_tokens: 100,
            temperature: 0.0,
        };
        let ctx = pii_core::Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited());
        assert_eq!(p.chat(&ctx, &req).unwrap().text, "one");
        assert_eq!(p.chat(&ctx, &req).unwrap().text, "two");
        assert_eq!(
            p.chat(&ctx, &req).unwrap_err().to_string(),
            "invalid: scripted provider exhausted"
        );
    }

    #[test]
    fn provider_spec_is_validated() {
        assert!(provider_from_spec("bogus:x").is_err());
    }
}

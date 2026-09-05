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
    /// Optional per-call sleep (ms) before returning, for latency tests.
    sleep_ms: u64,
}

/// Wire shape for `scripted:<path>` fixtures: `{"turns": [ChatResponse, ...]}`.
#[derive(Deserialize)]
struct ScriptedFixture {
    turns: Vec<ChatResponse>,
    /// Optional per-call sleep (ms) before returning.
    #[serde(default)]
    sleep_ms: u64,
}

impl ScriptedProvider {
    /// A provider with no turns (used where a provider is required but never
    /// called, e.g. static linting).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            turns: Vec::new(),
            calls: AtomicUsize::new(0),
            sleep_ms: 0,
        }
    }

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
            sleep_ms: f.sleep_ms,
        })
    }
}

impl ModelProvider for ScriptedProvider {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn chat(&self, _ctx: &Ctx, _req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed);
        if self.sleep_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(self.sleep_ms));
        }
        self.turns
            .get(n)
            .cloned()
            .ok_or_else(|| PiiError::Invalid(String::from("scripted provider exhausted")))
    }
}

pub mod ollama;
pub use ollama::OllamaProvider;

/// Resolves a model spec: `scripted:<path>` needs no transport. `ollama:`
/// (B1, opus review) is now wired through [`ollama::provider_from_env`],
/// which was correct and complete but had zero non-test call sites — every
/// green exit criterion touching an `agent` node was satisfied by a
/// hand-written fixture, never a live LLM turn. This entry point still
/// cannot construct an `ollama:` provider itself (it has no
/// `HttpTransport` — the real transport is `pii-net`'s `NetService`,
/// constructed by the caller with its egress policy already wired), so a
/// caller that wants `ollama:` must call [`ollama::provider_from_env`]
/// directly with its own transport, or use [`resolve_with_transport`],
/// which does both in one call for callers that don't care about the
/// distinction.
///
/// # Errors
/// [`PiiError::Invalid`] for an unknown spec, a fixture failure, or an
/// `ollama:` spec (which needs a transport — see above).
pub fn provider_from_spec(spec: &str) -> Result<Arc<dyn ModelProvider>, PiiError> {
    let Some((kind, rest)) = spec.split_once(':') else {
        return Err(PiiError::Invalid(format!("unknown model spec '{spec}'")));
    };
    match kind {
        "scripted" => Ok(Arc::new(ScriptedProvider::from_fixture(Path::new(rest))?)),
        "ollama" => Err(PiiError::Invalid(String::from(
            "ollama: needs a transport — call ollama::provider_from_env or resolve_with_transport",
        ))),
        other => Err(PiiError::Invalid(format!(
            "unknown model spec kind '{other}' (scripted:<path> and ollama:<model> are supported)"
        ))),
    }
}

/// Resolves any supported model spec, constructing an `ollama:` provider
/// through `transport` when the spec calls for one (B1, opus review). This
/// is the entry point a real caller (the daemon, `pii -p`) should use
/// instead of [`provider_from_spec`] alone, since it is the one place that
/// can actually honour `ollama:`.
///
/// # Errors
/// [`PiiError::Invalid`] for an unknown spec or a fixture/env failure.
pub fn resolve_with_transport(
    spec: &str,
    transport: Arc<dyn pii_core::HttpTransport>,
) -> Result<Arc<dyn ModelProvider>, PiiError> {
    if spec.starts_with("ollama:") {
        return ollama::provider_from_env(spec, transport);
    }
    provider_from_spec(spec)
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

    /// B1 (opus review): `provider_from_spec("ollama:...")` must give a
    /// clear, distinguishable error (not the old generic "unknown model
    /// spec kind") — proving the dispatch actually recognizes `ollama:`
    /// now, even though this entry point (by design, no transport) cannot
    /// construct the provider itself. `resolve_with_transport` is the one
    /// that does, and is exercised by `pii-server`'s own tests where a
    /// transport is available.
    #[test]
    fn provider_from_spec_dispatches_ollama() {
        let err = match provider_from_spec("ollama:qwen3") {
            Ok(_) => panic!("ollama: must fail without a transport"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("ollama:") && err.contains("transport"),
            "expected an ollama-specific 'needs a transport' error, got: {err}"
        );
        // A truly unknown kind still gets the generic message, proving the
        // ollama arm is a distinct branch, not a fallthrough.
        let unknown = match provider_from_spec("bogus:x") {
            Ok(_) => panic!("bogus: must fail"),
            Err(e) => e.to_string(),
        };
        assert!(unknown.contains("unknown model spec kind"));
    }
}

//! The Anthropic chat provider: Messages API, `POST {base}/v1/messages`.
//!
//! Auth is `x-api-key` (`ANTHROPIC_API_KEY`, required) plus a pinned
//! `anthropic-version`. Base URL: `ANTHROPIC_BASE_URL`, default
//! [`DEFAULT_ANTHROPIC_BASE_URL`]. Streaming is NOT supported by this kind:
//! `chat_streaming` says so instead of buffering the whole body.

use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::{Ctx, HttpTransport};
use pii_observe::Observer;

use crate::wire::Wire;
use crate::{ChatMessage, ChatRequest, ChatResponse, ModelProvider, Role, ToolCall, ToolSpec};

/// Default base URL when `ANTHROPIC_BASE_URL` is unset.
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
/// The pinned `anthropic-version` header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// One chat completion against the Anthropic Messages API.
pub struct AnthropicProvider {
    wire: Wire,
    model: String,
    api_key: String,
}

#[derive(serde::Deserialize)]
struct Reply {
    #[serde(default)]
    content: Vec<Block>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(serde::Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    #[serde(other)]
    Other,
}

impl AnthropicProvider {
    /// Builds a provider; `base` has any trailing `/` trimmed.
    #[must_use]
    pub fn new(
        transport: Arc<dyn HttpTransport>,
        base: &str,
        model: String,
        api_key: String,
    ) -> Self {
        Self {
            wire: Wire::new(transport, "anthropic", base, Some(api_key.clone())),
            model,
            api_key,
        }
    }

    /// Attaches the observer that receives the named `model.provider.*` events.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.wire.observer = Some(observer);
        self
    }

    fn build_request(&self, req: &ChatRequest) -> serde_json::Value {
        let system = req
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "messages": req
                .messages
                .iter()
                .filter(|m| m.role != Role::System)
                .map(message_value)
                .collect::<Vec<_>>(),
        });
        if let Some(o) = body.as_object_mut() {
            if !system.is_empty() {
                o.insert("system".into(), system.into());
            }
            if !req.tools.is_empty() {
                o.insert(
                    "tools".into(),
                    req.tools.iter().map(tool_value).collect::<Vec<_>>().into(),
                );
            }
        }
        body
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("x-api-key".to_owned(), self.api_key.clone()),
            ("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned()),
        ]
    }
}

impl ModelProvider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        let resp = self.wire.post(
            ctx,
            &self.wire.url("/v1/messages"),
            self.headers(),
            &self.build_request(req),
        )?;
        let parsed: Reply =
            serde_json::from_slice(&resp.body).map_err(|e| self.wire.bad_body(e))?;
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        for b in parsed.content {
            match b {
                Block::Text { text: t } => text.push_str(&t),
                Block::ToolUse { id, name, input } => tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments: input,
                }),
                Block::Other => {}
            }
        }
        let usage = parsed.usage.unwrap_or_default();
        Ok(ChatResponse {
            text,
            tool_calls,
            tokens_in: usage.input_tokens,
            tokens_out: usage.output_tokens,
            micro_usd: 0,
        })
    }

    /// Not supported for this kind: refuses instead of silently buffering.
    fn chat_streaming(
        &self,
        _ctx: &Ctx,
        _req: &ChatRequest,
        _on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, PiiError> {
        Err(PiiError::Invalid(
            "anthropic: streaming unsupported for this kind (use chat)".into(),
        ))
    }
}

fn message_value(m: &ChatMessage) -> serde_json::Value {
    match m.role {
        Role::Tool => serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                "content": m.content,
            }],
        }),
        Role::Assistant if !m.tool_calls.is_empty() => {
            let mut blocks = Vec::new();
            if !m.content.is_empty() {
                blocks.push(serde_json::json!({"type": "text", "text": m.content}));
            }
            blocks.extend(m.tool_calls.iter().map(|c| {
                serde_json::json!({
                    "type": "tool_use", "id": c.id, "name": c.name, "input": c.arguments,
                })
            }));
            serde_json::json!({"role": "assistant", "content": blocks})
        }
        Role::Assistant => serde_json::json!({"role": "assistant", "content": m.content}),
        Role::User | Role::System => serde_json::json!({"role": "user", "content": m.content}),
    }
}

fn tool_value(t: &ToolSpec) -> serde_json::Value {
    serde_json::json!({
        "name": t.name, "description": t.description, "input_schema": t.schema,
    })
}

/// Resolves `anthropic:<model>` from the process environment
/// (`ANTHROPIC_BASE_URL`, `ANTHROPIC_API_KEY`) and `transport`.
///
/// # Errors
/// [`PiiError::Invalid`] for an empty model name or a missing key (the error
/// names the variable, never a value).
pub fn provider_from_env(
    spec: &str,
    transport: Arc<dyn HttpTransport>,
) -> Result<Arc<dyn ModelProvider>, PiiError> {
    provider_from_lookup(spec, transport, &pii_core::env_var, None)
}

/// [`provider_from_env`] with an injectable environment lookup (tests) and an
/// optional observer for the named `model.provider.*` events.
///
/// # Errors
/// As [`provider_from_env`].
pub fn provider_from_lookup(
    spec: &str,
    transport: Arc<dyn HttpTransport>,
    env: &dyn Fn(&str) -> Option<String>,
    observer: Option<Arc<dyn Observer>>,
) -> Result<Arc<dyn ModelProvider>, PiiError> {
    let model = spec
        .strip_prefix("anthropic:")
        .filter(|m| !m.is_empty())
        .ok_or_else(|| PiiError::Invalid("anthropic: spec must be anthropic:<model>".into()))?;
    let key = env("ANTHROPIC_API_KEY")
        .filter(|k| !k.is_empty())
        .ok_or_else(|| PiiError::Invalid("anthropic: ANTHROPIC_API_KEY is not set".into()))?;
    let base = env("ANTHROPIC_BASE_URL").unwrap_or_else(|| DEFAULT_ANTHROPIC_BASE_URL.to_owned());
    let provider = AnthropicProvider::new(transport, &base, model.to_owned(), key);
    Ok(match observer {
        Some(o) => Arc::new(provider.with_observer(o)),
        None => Arc::new(provider),
    })
}

//! The Gemini chat provider: `POST {base}/v1beta/models/{model}:generateContent`.
//!
//! The key (`GEMINI_API_KEY`, required) travels in the `x-goog-api-key` HEADER,
//! not the `?key=` query parameter: URLs are echoed in errors and logs, headers
//! are never logged. Base URL: `GEMINI_BASE_URL`, default
//! [`DEFAULT_GEMINI_BASE_URL`]. Streaming is NOT supported by this kind.
//!
//! Gemini tool calls carry no id, so a call's id is its function name and a
//! tool result's `tool_call_id` is read back as the function name.

use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::{Ctx, HttpTransport};
use pii_observe::Observer;

use crate::wire::Wire;
use crate::{ChatMessage, ChatRequest, ChatResponse, ModelProvider, Role, ToolCall, ToolSpec};

/// Default base URL when `GEMINI_BASE_URL` is unset.
pub const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";

/// One chat completion against Gemini `generateContent`.
pub struct GeminiProvider {
    wire: Wire,
    model: String,
    api_key: String,
}

#[derive(serde::Deserialize)]
struct Reply {
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(default, rename = "usageMetadata")]
    usage: Option<Usage>,
}

#[derive(serde::Deserialize, Default)]
struct Usage {
    #[serde(default, rename = "promptTokenCount")]
    prompt: u64,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates: u64,
}

#[derive(serde::Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<Content>,
}

#[derive(serde::Deserialize)]
struct Content {
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(serde::Deserialize)]
struct Part {
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "functionCall")]
    function_call: Option<FunctionCall>,
}

#[derive(serde::Deserialize)]
struct FunctionCall {
    name: String,
    #[serde(default)]
    args: serde_json::Value,
}

impl GeminiProvider {
    /// Builds a provider; `base` has any trailing `/` trimmed.
    #[must_use]
    pub fn new(
        transport: Arc<dyn HttpTransport>,
        base: &str,
        model: String,
        api_key: String,
    ) -> Self {
        Self {
            wire: Wire::new(transport, "gemini", base, Some(api_key.clone())),
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

    fn build_request(req: &ChatRequest) -> serde_json::Value {
        let system = req
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let mut body = serde_json::json!({
            "contents": req
                .messages
                .iter()
                .filter(|m| m.role != Role::System)
                .map(content_value)
                .collect::<Vec<_>>(),
            "generationConfig": {
                "temperature": req.temperature,
                "maxOutputTokens": req.max_tokens,
            },
        });
        if let Some(o) = body.as_object_mut() {
            if !system.is_empty() {
                o.insert(
                    "systemInstruction".into(),
                    serde_json::json!({"parts": [{"text": system}]}),
                );
            }
            if !req.tools.is_empty() {
                o.insert(
                    "tools".into(),
                    serde_json::json!([{
                        "functionDeclarations": req.tools.iter().map(tool_value).collect::<Vec<_>>(),
                    }]),
                );
            }
        }
        body
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("x-goog-api-key".to_owned(), self.api_key.clone()),
        ]
    }
}

impl ModelProvider for GeminiProvider {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        let resp = self.wire.post(
            ctx,
            &self
                .wire
                .url(&format!("/v1beta/models/{}:generateContent", self.model)),
            self.headers(),
            &Self::build_request(req),
        )?;
        let parsed: Reply =
            serde_json::from_slice(&resp.body).map_err(|e| self.wire.bad_body(e))?;
        let parts = parsed
            .candidates
            .into_iter()
            .next()
            .and_then(|c| c.content)
            .map(|c| c.parts)
            .unwrap_or_default();
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        for p in parts {
            if let Some(t) = p.text {
                text.push_str(&t);
            }
            if let Some(f) = p.function_call {
                tool_calls.push(ToolCall {
                    id: f.name.clone(),
                    name: f.name,
                    arguments: f.args,
                });
            }
        }
        let usage = parsed.usage.unwrap_or_default();
        Ok(ChatResponse {
            text,
            tool_calls,
            tokens_in: usage.prompt,
            tokens_out: usage.candidates,
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
            "gemini: streaming unsupported for this kind (use chat)".into(),
        ))
    }
}

fn content_value(m: &ChatMessage) -> serde_json::Value {
    match m.role {
        Role::Tool => serde_json::json!({
            "role": "user",
            "parts": [{"functionResponse": {
                "name": m.tool_call_id.clone().unwrap_or_default(),
                "response": {"content": m.content},
            }}],
        }),
        Role::Assistant => {
            let mut parts = Vec::new();
            if !m.content.is_empty() || m.tool_calls.is_empty() {
                parts.push(serde_json::json!({"text": m.content}));
            }
            parts.extend(m.tool_calls.iter().map(
                |c| serde_json::json!({"functionCall": {"name": c.name, "args": c.arguments}}),
            ));
            serde_json::json!({"role": "model", "parts": parts})
        }
        Role::User | Role::System => {
            serde_json::json!({"role": "user", "parts": [{"text": m.content}]})
        }
    }
}

fn tool_value(t: &ToolSpec) -> serde_json::Value {
    serde_json::json!({
        "name": t.name, "description": t.description, "parameters": t.schema,
    })
}

/// Resolves `gemini:<model>` from the process environment
/// (`GEMINI_BASE_URL`, `GEMINI_API_KEY`) and `transport`.
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
        .strip_prefix("gemini:")
        .filter(|m| !m.is_empty())
        .ok_or_else(|| PiiError::Invalid("gemini: spec must be gemini:<model>".into()))?;
    let key = env("GEMINI_API_KEY")
        .filter(|k| !k.is_empty())
        .ok_or_else(|| PiiError::Invalid("gemini: GEMINI_API_KEY is not set".into()))?;
    let base = env("GEMINI_BASE_URL").unwrap_or_else(|| DEFAULT_GEMINI_BASE_URL.to_owned());
    let provider = GeminiProvider::new(transport, &base, model.to_owned(), key);
    Ok(match observer {
        Some(o) => Arc::new(provider.with_observer(o)),
        None => Arc::new(provider),
    })
}

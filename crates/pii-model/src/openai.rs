//! The OpenAI-compatible chat provider: `POST {base}/chat/completions`.
//!
//! The base URL is configurable (`OPENAI_BASE_URL`, default
//! [`DEFAULT_OPENAI_BASE_URL`]) because this one kind covers every vendor that
//! speaks the shape. The key (`OPENAI_API_KEY`) is optional — local servers
//! take none — and travels only as `Authorization: Bearer`. Streams via SSE.

use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::{Ctx, HttpTransport};
use pii_observe::Observer;

use crate::wire::{SseReader, Wire};
use crate::{ChatMessage, ChatRequest, ChatResponse, ModelProvider, Role, ToolCall, ToolSpec};

/// Default base URL when `OPENAI_BASE_URL` is unset.
pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// Most tool calls accepted from one streamed reply.
const MAX_STREAM_CALLS: usize = 64;

/// One chat completion against an OpenAI-compatible endpoint.
pub struct OpenAiProvider {
    wire: Wire,
    model: String,
    api_key: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

#[derive(serde::Deserialize)]
struct Completion {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(serde::Deserialize)]
struct Choice {
    message: Message,
}

#[derive(serde::Deserialize)]
struct Message {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireCall>,
}

#[derive(serde::Deserialize)]
struct WireCall {
    #[serde(default)]
    id: String,
    function: WireFunction,
}

#[derive(serde::Deserialize)]
struct WireFunction {
    name: String,
    #[serde(default)]
    arguments: String,
}

#[derive(serde::Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(serde::Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: Delta,
}

#[derive(serde::Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<DeltaCall>,
}

#[derive(serde::Deserialize)]
struct DeltaCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<DeltaFunction>,
}

#[derive(serde::Deserialize, Default)]
struct DeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

fn parse_call(c: &WireCall) -> Result<ToolCall, String> {
    let arguments = if c.function.arguments.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&c.function.arguments)
            .map_err(|e| format!("tool call '{}' arguments: {e}", c.function.name))?
    };
    Ok(ToolCall {
        id: c.id.clone(),
        name: c.function.name.clone(),
        arguments,
    })
}

impl OpenAiProvider {
    /// Builds a provider; `base` has any trailing `/` trimmed.
    #[must_use]
    pub fn new(
        transport: Arc<dyn HttpTransport>,
        base: &str,
        model: String,
        api_key: Option<String>,
    ) -> Self {
        Self {
            wire: Wire::new(transport, "openai", base, api_key.clone()),
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

    fn build_request(&self, req: &ChatRequest, stream: bool) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": req.messages.iter().map(message_value).collect::<Vec<_>>(),
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "stream": stream,
        });
        if let Some(o) = body.as_object_mut() {
            if !req.tools.is_empty() {
                o.insert(
                    "tools".into(),
                    req.tools.iter().map(tool_value).collect::<Vec<_>>().into(),
                );
            }
            if stream {
                o.insert(
                    "stream_options".into(),
                    serde_json::json!({"include_usage": true}),
                );
            }
        }
        body
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut h = vec![("Content-Type".to_owned(), "application/json".to_owned())];
        if let Some(k) = &self.api_key {
            h.push(("Authorization".to_owned(), format!("Bearer {k}")));
        }
        h
    }
}

impl ModelProvider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        let resp = self.wire.post(
            ctx,
            &self.wire.url("/chat/completions"),
            self.headers(),
            &self.build_request(req, false),
        )?;
        let parsed: Completion =
            serde_json::from_slice(&resp.body).map_err(|e| self.wire.bad_body(e))?;
        let msg = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message)
            .ok_or_else(|| self.wire.bad_body("no choices"))?;
        let tool_calls = msg
            .tool_calls
            .iter()
            .map(parse_call)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| self.wire.bad_body(e))?;
        let usage = parsed.usage.unwrap_or_default();
        Ok(ChatResponse {
            text: msg.content.unwrap_or_default(),
            tool_calls,
            tokens_in: usage.prompt_tokens,
            tokens_out: usage.completion_tokens,
            micro_usd: 0,
        })
    }

    /// SSE streaming: each `delta.content` is forwarded through `on_delta` and
    /// emitted as a `model.provider.stream_delta` event; usage comes from the
    /// final chunk (`stream_options.include_usage`).
    fn chat_streaming(
        &self,
        ctx: &Ctx,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, PiiError> {
        let mut sse = SseReader::default();
        let mut text = String::new();
        let mut usage = Usage::default();
        // (id, name, argument fragments) by stream index.
        let mut calls: Vec<(String, String, String)> = Vec::new();
        self.wire.post_streaming(
            ctx,
            &self.wire.url("/chat/completions"),
            self.headers(),
            &self.build_request(req, true),
            &mut |bytes| {
                sse.feed(bytes, |data| {
                    let Ok(chunk) = serde_json::from_str::<Chunk>(data) else {
                        return; // `[DONE]` and any non-chunk line
                    };
                    if let Some(u) = chunk.usage {
                        usage = u;
                    }
                    for c in chunk.choices {
                        if let Some(t) = c.delta.content.filter(|t| !t.is_empty()) {
                            self.wire.delta(&t);
                            on_delta(&t);
                            text.push_str(&t);
                        }
                        for d in c.delta.tool_calls {
                            // Bounded: a hostile index must not grow the vec.
                            if d.index >= MAX_STREAM_CALLS {
                                continue;
                            }
                            if calls.len() <= d.index {
                                calls.resize(d.index + 1, Default::default());
                            }
                            let Some(slot) = calls.get_mut(d.index) else {
                                continue;
                            };
                            if let Some(id) = d.id {
                                slot.0 = id;
                            }
                            if let Some(f) = d.function {
                                slot.1.push_str(f.name.as_deref().unwrap_or(""));
                                slot.2.push_str(f.arguments.as_deref().unwrap_or(""));
                            }
                        }
                    }
                });
            },
        )?;
        let tool_calls = calls
            .into_iter()
            .map(|(id, name, arguments)| {
                parse_call(&WireCall {
                    id,
                    function: WireFunction { name, arguments },
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| self.wire.bad_body(e))?;
        Ok(ChatResponse {
            text,
            tool_calls,
            tokens_in: usage.prompt_tokens,
            tokens_out: usage.completion_tokens,
            micro_usd: 0,
        })
    }
}

fn message_value(m: &ChatMessage) -> serde_json::Value {
    let role = match m.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut obj = serde_json::json!({ "role": role, "content": m.content });
    if let Some(map) = obj.as_object_mut() {
        if !m.tool_calls.is_empty() {
            map.insert(
                "tool_calls".into(),
                m.tool_calls
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "id": c.id,
                            "type": "function",
                            // The wire carries arguments as a JSON *string*.
                            "function": { "name": c.name, "arguments": c.arguments.to_string() },
                        })
                    })
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        if let Some(id) = &m.tool_call_id {
            map.insert("tool_call_id".into(), id.clone().into());
        }
    }
    obj
}

fn tool_value(t: &ToolSpec) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": { "name": t.name, "description": t.description, "parameters": t.schema },
    })
}

/// Resolves `openai:<model>` from the process environment
/// (`OPENAI_BASE_URL`, `OPENAI_API_KEY`) and `transport`.
///
/// # Errors
/// [`PiiError::Invalid`] for an empty model name or a foreign spec kind.
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
        .strip_prefix("openai:")
        .filter(|m| !m.is_empty())
        .ok_or_else(|| PiiError::Invalid("openai: spec must be openai:<model>".into()))?;
    let base = env("OPENAI_BASE_URL").unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_owned());
    let provider = OpenAiProvider::new(transport, &base, model.to_owned(), env("OPENAI_API_KEY"));
    Ok(match observer {
        Some(o) => Arc::new(provider.with_observer(o)),
        None => Arc::new(provider),
    })
}

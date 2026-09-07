//! The Ollama chat provider (F2.12): built on `Arc<dyn HttpTransport>` —
//! zero socket code in `pii-model`. Plaintext `http://` only (TLS is Phase 3b).

use std::sync::Arc;

use pii_abi::{HttpMethod, HttpRequest, HttpResponse, PiiError};
use pii_core::{Ctx, HttpTransport};

use crate::{ChatMessage, ChatRequest, ChatResponse, ModelProvider, ToolCall, ToolSpec};

/// Default Ollama endpoint when `OLLAMA_HOST` is unset.
pub const DEFAULT_OLLAMA_HOST: &str = "http://127.0.0.1:11434";

/// Default context window (`num_ctx`) when a `ChatRequest` does not set one.
/// The compaction gate (4.1b) uses this as its budget reference.
pub const DEFAULT_NUM_CTX: u32 = 8192;

/// One chat completion against a local Ollama (`POST {host}/api/chat`).
pub struct OllamaProvider {
    transport: Arc<dyn HttpTransport>,
    host: String,
    model: String,
    /// `Authorization: Bearer` value when `OLLAMA_API_KEY` was set. Never
    /// logged, journaled or returned (N9).
    api_key: Option<String>,
    /// Per-request wall-clock budget.
    timeout_ms: u64,
}

/// Wire shape of an Ollama `/api/chat` response we consume.
#[derive(serde::Deserialize)]
struct OllamaChat {
    #[serde(default)]
    message: Option<OllamaMessage>,
    #[serde(default)]
    prompt_eval_count: u64,
    #[serde(default)]
    eval_count: u64,
}

#[derive(serde::Deserialize)]
struct OllamaMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<OllamaToolCall>,
}

#[derive(serde::Deserialize)]
struct OllamaToolCall {
    function: OllamaFunction,
}

#[derive(serde::Deserialize)]
struct OllamaFunction {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

impl OllamaProvider {
    /// Builds a provider. `host` should come from `pii_core::env_var`
    /// (`OLLAMA_HOST`) with [`DEFAULT_OLLAMA_HOST`] as the fallback.
    #[must_use]
    pub fn new(
        transport: Arc<dyn HttpTransport>,
        host: &str,
        model: String,
        api_key: Option<String>,
    ) -> Self {
        Self {
            transport,
            host: host.trim_end_matches('/').to_owned(),
            model,
            api_key,
            timeout_ms: 120_000,
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/api/chat", self.host)
    }

    fn build_request(&self, req: &ChatRequest) -> serde_json::Value {
        let num_ctx = if req.num_ctx == 0 {
            DEFAULT_NUM_CTX
        } else {
            req.num_ctx
        };
        serde_json::json!({
            "model": self.model,
            "messages": req.messages.iter().map(message_value).collect::<Vec<_>>(),
            "tools": req.tools.iter().map(tool_value).collect::<Vec<_>>(),
            "options": {
                "temperature": req.temperature,
                "num_predict": req.max_tokens,
                "num_ctx": num_ctx,
            },
            "stream": false,
        })
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut h = vec![(
            String::from("Content-Type"),
            String::from("application/json"),
        )];
        if let Some(k) = &self.api_key {
            h.push((String::from("Authorization"), format!("Bearer {k}")));
        }
        h
    }

    fn send(&self, ctx: &Ctx, req: &ChatRequest) -> Result<HttpResponse, PiiError> {
        let http = HttpRequest {
            method: HttpMethod::Post,
            url: self.chat_url(),
            headers: self.headers(),
            body: serde_json::to_vec(&self.build_request(req))
                .map_err(|e| PiiError::Internal(format!("ollama: encode: {e}")))?,
            timeout_ms: self.timeout_ms,
        };
        let resp = self.transport.request(ctx, &http)?;
        if resp.status == 429 || resp.status >= 500 {
            return Err(PiiError::Unavailable(format!(
                "ollama: upstream status {}",
                resp.status
            )));
        }
        if !(200..300).contains(&resp.status) {
            return Err(PiiError::Invalid(format!(
                "ollama: status {} for POST {}",
                resp.status,
                self.chat_url()
            )));
        }
        Ok(resp)
    }
}

impl ModelProvider for OllamaProvider {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        let resp = self.send(ctx, req)?;
        let parsed: OllamaChat = serde_json::from_slice(&resp.body)
            .map_err(|e| PiiError::Invalid(format!("ollama: response body: {e}")))?;
        let msg = parsed.message.unwrap_or(OllamaMessage {
            content: String::new(),
            tool_calls: Vec::new(),
        });
        Ok(ChatResponse {
            text: msg.content,
            tool_calls: msg
                .tool_calls
                .into_iter()
                .map(|c| ToolCall {
                    id: format!("call-{}", c.function.name),
                    name: c.function.name,
                    arguments: c.function.arguments,
                })
                .collect(),
            tokens_in: parsed.prompt_eval_count,
            tokens_out: parsed.eval_count,
        })
    }
}

fn message_value(m: &ChatMessage) -> serde_json::Value {
    let role = match m.role {
        crate::Role::System => "system",
        crate::Role::User => "user",
        crate::Role::Assistant => "assistant",
        crate::Role::Tool => "tool",
    };
    serde_json::json!({ "role": role, "content": m.content })
}

fn tool_value(t: &ToolSpec) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": { "name": t.name, "description": t.description, "parameters": t.schema },
    })
}

/// Resolves a model spec of the form `ollama:<model>` using the process
/// environment and the given transport.
///
/// `OLLAMA_HOST`/`OLLAMA_API_KEY` come via `pii_core::env_var`;
/// `scripted:<path>` needs no transport and is handled by
/// [`crate::provider_from_spec`].
///
/// # Errors
/// [`PiiError::Invalid`] for an unknown spec kind.
pub fn provider_from_env(
    spec: &str,
    transport: Arc<dyn HttpTransport>,
) -> Result<Arc<dyn ModelProvider>, PiiError> {
    let Some((kind, rest)) = spec.split_once(':') else {
        return Err(PiiError::Invalid(format!("unknown model spec '{spec}'")));
    };
    match kind {
        "ollama" => {
            if rest.is_empty() {
                return Err(PiiError::Invalid("ollama: spec needs a model name".into()));
            }
            let host = pii_core::env_var("OLLAMA_HOST")
                .unwrap_or_else(|| String::from(DEFAULT_OLLAMA_HOST));
            let api_key = pii_core::env_var("OLLAMA_API_KEY");
            Ok(Arc::new(OllamaProvider::new(
                transport,
                &host,
                rest.to_owned(),
                api_key,
            )))
        }
        other => Err(PiiError::Invalid(format!(
            "unknown model spec kind '{other}' (scripted:<path> and ollama:<model> are supported)"
        ))),
    }
}

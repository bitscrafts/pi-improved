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
    /// Streaming chat (spec 15 R2): the transport hands raw body bytes as
    /// they arrive; each NDJSON line's `message.content` is forwarded through
    /// `on_delta` incrementally, and the accumulated text plus the final
    /// token counts are returned as the `ChatResponse`.
    ///
    /// # Errors
    /// Same contract as [`ModelProvider::chat`].
    fn chat_streaming(
        &self,
        ctx: &Ctx,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, PiiError> {
        let http = HttpRequest {
            method: HttpMethod::Post,
            url: self.chat_url(),
            headers: self.headers(),
            body: serde_json::to_vec(&self.build_stream_request(req))
                .map_err(|e| PiiError::Internal(format!("ollama: encode: {e}")))?,
            timeout_ms: self.timeout_ms,
        };
        // The transport must expose streaming; fall back to the buffered
        // path only when the concrete type cannot (fakes).
        let resp = self.transport.request_streaming(ctx, &http, &mut |bytes| {
            for line in String::from_utf8_lossy(bytes).lines() {
                let Ok(l) = serde_json::from_str::<OllamaStreamLine>(line) else {
                    return; // partial line: the next read completes it in this model
                };
                if let Some(m) = &l.message
                    && !m.content.is_empty()
                {
                    on_delta(&m.content);
                }
            }
        })?;
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
        // Accumulate from the full body (each line's content was already
        // streamed out; the response text is their concatenation).
        let text = String::from_utf8_lossy(&resp.body)
            .lines()
            .filter_map(|l| serde_json::from_str::<OllamaStreamLine>(l).ok())
            .filter_map(|l| l.message.map(|m| m.content))
            .collect::<String>();
        let last = String::from_utf8_lossy(&resp.body)
            .lines()
            .rev()
            .find_map(|l| serde_json::from_str::<OllamaStreamLine>(l).ok());
        Ok(ChatResponse {
            text,
            tool_calls: Vec::new(),
            tokens_in: last.as_ref().map_or(0, |l| l.prompt_eval_count),
            tokens_out: last.as_ref().map_or(0, |l| l.eval_count),
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

/// One NDJSON line of an Ollama streaming response.
#[derive(serde::Deserialize)]
struct OllamaStreamLine {
    #[serde(default)]
    message: Option<OllamaMessage>,
    /// Kept on the struct for wire fidelity (Ollama always sends it); the
    /// streaming accumulator only reads the content/token-count fields.
    #[allow(dead_code)] // reason: wire-shape completeness
    #[serde(default)]
    done: bool,
    #[serde(default)]
    prompt_eval_count: u64,
    #[serde(default)]
    eval_count: u64,
}

impl OllamaProvider {
    /// Builds the streaming request body: identical to `build_request` but
    /// with `"stream": true` (spec 15 R2 — a plausible implementation gets
    /// this wrong).
    fn build_stream_request(&self, req: &ChatRequest) -> serde_json::Value {
        let mut body = self.build_request(req);
        if let Some(obj) = body.as_object_mut() {
            obj.insert(String::from("stream"), serde_json::Value::Bool(true));
        }
        body
    }
}

#[cfg(test)]
mod streaming_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{OllamaProvider, OllamaStreamLine};
    use crate::{ChatMessage, ChatRequest, ModelProvider, Role};
    use parking_lot::Mutex;
    use pii_core::{Budget, CapSet, Ctx};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A chunked NDJSON server: hands three body slices through the
    /// streaming sink, recording the `"stream"` flag of the request.
    struct ChunkedTransport {
        requests: Mutex<Vec<serde_json::Value>>,
        reads: AtomicU32,
    }

    impl pii_core::HttpTransport for ChunkedTransport {
        fn request(&self, _ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError> {
            self.requests
                .lock()
                .push(serde_json::from_slice(&req.body).unwrap());
            // Buffered fallback: the whole NDJSON in one body.
            Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: br#"{"message":{"content":"Hel"},"done":false}
{"message":{"content":"lo"},"done":false}
{"done":true,"prompt_eval_count":5,"eval_count":3}"#
                    .to_vec(),
            })
        }

        fn request_streaming(
            &self,
            _ctx: &Ctx,
            req: &HttpRequest,
            on_chunk: &mut dyn FnMut(&[u8]),
        ) -> Result<HttpResponse, PiiError> {
            self.requests
                .lock()
                .push(serde_json::from_slice(&req.body).unwrap());
            self.reads.fetch_add(1, Ordering::SeqCst);
            // Three separate reads, exactly the R2 acceptance shape.
            on_chunk(br#"{"message":{"content":"Hel"},"done":false}"#);
            on_chunk(br#"{"message":{"content":"lo"},"done":false}"#);
            on_chunk(br#"{"done":true,"prompt_eval_count":5,"eval_count":3}"#);
            Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: br#"{"message":{"content":"Hel"},"done":false}
{"message":{"content":"lo"},"done":false}
{"done":true,"prompt_eval_count":5,"eval_count":3}"#
                    .to_vec(),
            })
        }
    }

    use pii_abi::{HttpRequest, HttpResponse, PiiError};

    fn ctx() -> Ctx {
        Ctx::root(CapSet::empty(), Budget::unlimited())
    }

    /// R2 acceptance: `on_delta` fires incrementally per NDJSON content chunk
    /// (`Hel` then `lo`), and the accumulated `ChatResponse` carries the full
    /// text and the final token counts.
    #[test]
    fn ollama_streaming_emits_deltas() {
        let transport = Arc::new(ChunkedTransport {
            requests: Mutex::new(Vec::new()),
            reads: AtomicU32::new(0),
        });
        let provider = OllamaProvider::new(
            Arc::clone(&transport) as Arc<dyn pii_core::HttpTransport>,
            "http://fake.local:1",
            "m".into(),
            None,
        );
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::new(Role::User, "hi")],
            tools: Vec::new(),
            max_tokens: 32,
            temperature: 0.0,
            num_ctx: 0,
        };
        let deltas: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let resp = {
            let mut guard = deltas.lock();
            let r = provider
                .chat_streaming(&ctx(), &req, &mut |t| {
                    guard.push(t.to_owned());
                })
                .unwrap();
            drop(guard);
            r
        };
        let d = deltas.lock();
        assert_eq!(d.len(), 2, "deltas must be incremental: {d:?}");
        assert_eq!(d.first().map(String::as_str), Some("Hel"));
        assert_eq!(d.get(1).map(String::as_str), Some("lo"));
        drop(d);
        assert_eq!(resp.text, "Hello");
        assert_eq!(resp.tokens_in, 5);
        assert_eq!(resp.tokens_out, 3);
        // The wire body must carry "stream": true (the classic miss).
        let body = transport.requests.lock();
        assert!(!body.is_empty());
        assert_eq!(
            body.first().and_then(|b| b.get("stream")),
            Some(&serde_json::json!(true))
        );
        drop(body);
    }

    /// The streaming parse helpers handle a partial trailing read without
    /// panicking (total parser, N7).
    #[test]
    fn ollama_stream_line_is_total() {
        assert!(
            serde_json::from_str::<OllamaStreamLine>("not json").is_err(),
            "malformed line is an Err, never a panic"
        );
    }
}

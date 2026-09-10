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
            micro_usd: 0,
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
        // Cross-chunk line buffer (R3): a TCP read can split a NDJSON line
        // mid-JSON; the residual is carried to the next chunk. Tool calls
        // are accumulated across lines (R1) — the streaming path must return
        // them exactly like the buffered `chat`.
        let mut line_buf = String::new();
        let mut streamed_tool_calls: Vec<ToolCall> = Vec::new();
        // Token counts of the final streamed line (the `done` line).
        let mut streamed_tokens = (0u64, 0u64);
        let resp = self.transport.request_streaming(ctx, &http, &mut |bytes| {
            line_buf.push_str(&String::from_utf8_lossy(bytes));
            drain_lines(
                &mut line_buf,
                on_delta,
                &mut streamed_tool_calls,
                &mut streamed_tokens,
            );
        })?;
        // Flush the residual (the last value usually has no trailing
        // newline): it must be parsed too.
        if !line_buf.trim().is_empty() {
            drain_lines(
                &mut line_buf,
                on_delta,
                &mut streamed_tool_calls,
                &mut streamed_tokens,
            );
        }
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
        // streamed out; the response text is their concatenation). Tool
        // calls are taken from the streamed accumulation (R1) — the body is
        // the same NDJSON.
        let body = String::from_utf8_lossy(&resp.body);
        let lines: Vec<OllamaStreamLine> = body
            .lines()
            .filter_map(|l| serde_json::from_str::<OllamaStreamLine>(l).ok())
            .collect();
        let text = lines
            .iter()
            .filter_map(|l| l.message.as_ref().map(|m| m.content.clone()))
            .collect::<String>();
        let last = lines.last();
        let body_tool_calls: Vec<ToolCall> = lines
            .iter()
            .filter_map(|l| l.message.as_ref())
            .flat_map(|m| {
                m.tool_calls.iter().map(|c| ToolCall {
                    id: format!("call-{}", c.function.name),
                    name: c.function.name.clone(),
                    arguments: c.function.arguments.clone(),
                })
            })
            .collect();
        let tool_calls = if streamed_tool_calls.is_empty() {
            body_tool_calls
        } else {
            streamed_tool_calls
        };
        Ok(ChatResponse {
            text,
            tool_calls,
            tokens_in: last.map_or(streamed_tokens.0, |l| l.prompt_eval_count),
            tokens_out: last.as_ref().map_or(streamed_tokens.1, |l| l.eval_count),
            micro_usd: 0,
        })
    }
}

/// Parses every complete JSON value currently in `buf`, keeping the residual
/// (a TCP read may split a value mid-JSON, R3). Handles both newline-separated
/// NDJSON and values delivered back-to-back with no separator. Deltas and tool
/// calls are independent: a tool-call line typically carries empty content.
fn drain_lines(
    buf: &mut String,
    on_delta: &mut dyn FnMut(&str),
    streamed_tool_calls: &mut Vec<ToolCall>,
    streamed_tokens: &mut (u64, u64),
) {
    let mut de = serde_json::Deserializer::from_str(buf).into_iter::<OllamaStreamLine>();
    while let Some(Ok(l)) = de.next() {
        if l.eval_count > 0 || l.prompt_eval_count > 0 {
            *streamed_tokens = (l.prompt_eval_count, l.eval_count);
        }
        if let Some(m) = &l.message {
            if !m.content.is_empty() {
                on_delta(&m.content);
            }
            for c in &m.tool_calls {
                streamed_tool_calls.push(ToolCall {
                    id: format!("call-{}", c.function.name),
                    name: c.function.name.clone(),
                    arguments: c.function.arguments.clone(),
                });
            }
        }
    }
    // Consume everything successfully parsed; the residual (an incomplete
    // trailing value) stays buffered for the next chunk.
    let consumed = de.byte_offset();
    buf.drain(..consumed);
}

fn message_value(m: &ChatMessage) -> serde_json::Value {
    let role = match m.role {
        crate::Role::System => "system",
        crate::Role::User => "user",
        crate::Role::Assistant => "assistant",
        crate::Role::Tool => "tool",
    };
    let mut obj = serde_json::json!({ "role": role, "content": m.content });
    if let Some(map) = obj.as_object_mut() {
        // R4 (A7): multi-step tool use needs tool_calls (assistant) and
        // tool_call_id (tool result) on the wire — dropping them makes the
        // model re-request the same tool.
        if !m.tool_calls.is_empty() {
            map.insert(
                String::from("tool_calls"),
                serde_json::json!(
                    m.tool_calls
                        .iter()
                        .map(|c| serde_json::json!({
                            "function": {
                                "name": c.name,
                                "arguments": c.arguments,
                            },
                        }))
                        .collect::<Vec<_>>()
                ),
            );
        }
        if let Some(id) = &m.tool_call_id {
            map.insert(
                String::from("tool_call_id"),
                serde_json::Value::String(id.clone()),
            );
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
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::significant_drop_tightening
    )]

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
            let mut n = 0u32;
            let r = provider
                .chat_streaming(&ctx(), &req, &mut |t| {
                    guard.push(t.to_owned());
                    n += 1;
                })
                .unwrap();
            drop(guard);
            let _ = n;
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
        assert_eq!(
            transport.reads.load(Ordering::SeqCst),
            1,
            "streaming path used"
        );
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

    /// R1 (spec 22): a streaming response line carrying
    /// `message.tool_calls` must surface them in `ChatResponse.tool_calls`.
    #[test]
    fn ollama_streaming_accumulates_tool_calls() {
        struct ToolTransport(Mutex<Vec<serde_json::Value>>);
        impl pii_core::HttpTransport for ToolTransport {
            fn request(&self, _ctx: &Ctx, _req: &HttpRequest) -> Result<HttpResponse, PiiError> {
                unreachable!("streaming path should be used");
            }
            fn request_streaming(
                &self,
                _ctx: &Ctx,
                req: &HttpRequest,
                on_chunk: &mut dyn FnMut(&[u8]),
            ) -> Result<HttpResponse, PiiError> {
                self.0
                    .lock()
                    .push(serde_json::from_slice(&req.body).unwrap());
                on_chunk(
                    br#"{"model":"m","message":{"content":"let me check"},"done":false}
{"model":"m","message":{"content":"","tool_calls":[{"function":{"name":"grep","arguments":{"pattern":"TODO"}}}]},"done":false}
{"done":true,"prompt_eval_count":7,"eval_count":4}"#,
                );
                Ok(HttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: Vec::new(),
                })
            }
        }
        let transport = Arc::new(ToolTransport(Mutex::new(Vec::new())));
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
        let resp = provider.chat_streaming(&ctx(), &req, &mut |_| {}).unwrap();
        assert_eq!(resp.tool_calls.len(), 1, "tool_calls must accumulate");
        assert_eq!(resp.tool_calls[0].name, "grep");
        assert_eq!(
            resp.tool_calls[0].arguments,
            serde_json::json!({"pattern": "TODO"})
        );
        assert_eq!(resp.tokens_in, 7);
        assert_eq!(resp.tokens_out, 4);
        // "stream": true on the wire, as for any streaming request.
        let body = transport.0.lock();
        assert_eq!(
            body.first().and_then(|b| b.get("stream")),
            Some(&serde_json::json!(true))
        );
    }

    /// R1 edge: a tool-call line SPLIT mid-JSON across two chunk reads must
    /// still be parsed once the line completes (cross-chunk buffer, R3).
    #[test]
    fn ollama_streaming_survives_split_tool_call_line() {
        struct SplitTransport;
        impl pii_core::HttpTransport for SplitTransport {
            fn request(&self, _ctx: &Ctx, _req: &HttpRequest) -> Result<HttpResponse, PiiError> {
                unreachable!()
            }
            fn request_streaming(
                &self,
                _ctx: &Ctx,
                _req: &HttpRequest,
                on_chunk: &mut dyn FnMut(&[u8]),
            ) -> Result<HttpResponse, PiiError> {
                // One NDJSON tool-call line split mid-JSON.
                let full = br#"{"message":{"tool_calls":[{"function":{"name":"bash","arguments":{"cmd":"ls"}}}]},"done":false}"#;
                let (a, b) = full.split_at(full.len() / 2);
                on_chunk(a);
                on_chunk(b);
                on_chunk(b"\n{\"done\":true,\"prompt_eval_count\":1,\"eval_count\":1}");
                Ok(HttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: Vec::new(),
                })
            }
        }
        let provider = OllamaProvider::new(
            Arc::new(SplitTransport) as Arc<dyn pii_core::HttpTransport>,
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
        let resp = provider.chat_streaming(&ctx(), &req, &mut |_| {}).unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "bash");
    }
}

/// R4 (spec 22): `message_value` must serialize `tool_calls` on assistant
/// messages and `tool_call_id` on tool messages.
#[cfg(test)]
mod message_value_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use crate::ollama::message_value;
    use crate::{ChatMessage, Role, ToolCall};

    #[test]
    fn message_value_includes_tool_calls() {
        let mut msg = ChatMessage::new(Role::Assistant, "");
        msg.tool_calls.push(ToolCall {
            id: "call-grep".into(),
            name: "grep".into(),
            arguments: serde_json::json!({"pattern": "TODO"}),
        });
        let v = message_value(&msg);
        assert_eq!(v["role"], "assistant");
        let calls = v["tool_calls"].as_array().expect("tool_calls serialized");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "grep");
        // A tool result message carries tool_call_id.
        let mut tool_msg = ChatMessage::new(Role::Tool, "result");
        tool_msg.tool_call_id = Some(String::from("call-grep"));
        let tv = message_value(&tool_msg);
        assert_eq!(tv["role"], "tool");
        assert_eq!(tv["tool_call_id"], "call-grep");
        // A plain user message carries neither field.
        let uv = message_value(&ChatMessage::new(Role::User, "hi"));
        assert!(uv.get("tool_calls").is_none());
        assert!(uv.get("tool_call_id").is_none());
    }
}

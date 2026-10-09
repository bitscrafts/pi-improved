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
    /// Context window (`num_ctx`); `0` means "provider default"
    /// ([`crate::ollama::DEFAULT_NUM_CTX`] for Ollama).
    pub num_ctx: u32,
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
    /// Cost reported by the provider, in micro-USD (0 = unknown/not charged).
    #[serde(default)]
    pub micro_usd: u64,
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

    /// Streaming variant (spec 15 R2): same request shape (with
    /// `"stream": true` set by streaming-capable providers), delivering
    /// text deltas through `on_delta` as they arrive, and returning the
    /// accumulated `ChatResponse`. Default: run `chat` once and hand the
    /// full text to `on_delta` as a single delta — non-streaming
    /// implementations keep working unchanged (R4's contract).
    ///
    /// # Errors
    /// Provider-specific; retryable errors surface as [`PiiError::Timeout`].
    fn chat_streaming(
        &self,
        ctx: &Ctx,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, PiiError> {
        let resp = self.chat(ctx, req)?;
        on_delta(&resp.text);
        Ok(resp)
    }
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

#[cfg(feature = "llamacpp")]
pub mod llamacpp;
#[cfg(feature = "llamacpp")]
pub use llamacpp::LlamaCppProvider;

pub mod llamacpp_tools;

mod observed;
pub use observed::ObservedProvider;

/// Resolves a model spec: `scripted:<path>` needs no transport.
///
/// `ollama:` (B1, opus review) is now wired through
/// [`ollama::provider_from_env`], which was correct and complete but had
/// zero non-test call sites — every green exit criterion touching an
/// `agent` node was satisfied by a hand-written fixture, never a live LLM
/// turn. This entry point still cannot construct an `ollama:` provider
/// itself (it has no `HttpTransport` — the real transport is `pii-net`'s
/// `NetService`, constructed by the caller with its egress policy already
/// wired), so a caller that wants `ollama:` must call
/// [`ollama::provider_from_env`] directly with its own transport, or use
/// [`resolve_with_transport`], which does both in one call for callers
/// that don't care about the distinction.
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
        #[cfg(feature = "llamacpp")]
        "llamacpp" => llamacpp::provider_from_env(spec),
        #[cfg(not(feature = "llamacpp"))]
        "llamacpp" => Err(PiiError::Invalid(String::from(
            "llamacpp: support not compiled in — rebuild pii-model with feature `llamacpp`",
        ))),
        "ollama" => Err(PiiError::Invalid(String::from(
            "ollama: needs a transport — call ollama::provider_from_env or resolve_with_transport",
        ))),
        other => Err(PiiError::Invalid(format!(
            "unknown model spec kind '{other}' (scripted:<path> and ollama:<model> are supported)"
        ))),
    }
}

/// Resolves any supported model spec.
///
/// Constructs an `ollama:` provider through `transport` when the spec
/// calls for one (B1, opus review). This is the entry point a real caller
/// (the daemon, `pii -p`) should use instead of [`provider_from_spec`]
/// alone, since it is the one place that can actually honour `ollama:`.
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

    use super::{ChatRequest, Role, ScriptedProvider, provider_from_spec};
    use crate::ModelProvider as _;
    use pii_core::{Budget, CapSet, Ctx};
    use std::path::PathBuf;

    fn contract_tmp_root() -> PathBuf {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn scripted_returns_turns_in_order() {
        let dir = tempfile::Builder::new()
            .prefix("pii-model-")
            .tempdir_in(contract_tmp_root())
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
            num_ctx: 0,
        };
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
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

    /// T3 (spec 35): `provider_from_spec("llamacpp:...")` dispatches to the
    /// llamacpp arm — a missing model file yields a clear `llamacpp:`-specific
    /// error, NOT the generic "unknown model spec kind".
    #[cfg(feature = "llamacpp")]
    #[test]
    fn provider_spec_dispatches_llamacpp() {
        let err = match provider_from_spec("llamacpp:/nonexistent/does-not-exist.gguf") {
            Ok(_) => panic!("llamacpp: must fail on a missing model file"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("llamacpp:") && err.contains("not found"),
            "expected a llamacpp-specific 'model file not found' error, got: {err}"
        );
    }

    /// Without the `llamacpp` feature the `llamacpp:` spec fails loud.
    #[cfg(not(feature = "llamacpp"))]
    #[test]
    fn llamacpp_spec_errors_when_feature_off() {
        let err = match provider_from_spec("llamacpp:/x.gguf") {
            Ok(_) => panic!("llamacpp: must fail when the feature is off"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("not compiled in"), "got: {err}");
    }

    /// T4 (spec 35): an empty `llamacpp:` path is rejected with a clear error.
    #[cfg(feature = "llamacpp")]
    #[test]
    fn llamacpp_requires_path() {
        let err = match provider_from_spec("llamacpp:") {
            Ok(_) => panic!("llamacpp: must fail on an empty path"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("llamacpp:") && err.contains("model path"),
            "expected a 'needs a model path' error, got: {err}"
        );
    }
}

// F2 (4.1a TDD contract): the request body must forward sampling options and
// the context window, and must keep non-streaming. `build_request` is private,
// so the contract is verified through a scripted transport that records the
// last body to `_tmp` (per-test files — tests run in parallel).
#[cfg(test)]
mod contract {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing)] // reason: json pointer-style asserts on test bodies

    use super::{ChatMessage, ChatRequest, OllamaProvider, Role, ScriptedProvider};
    use crate::ModelProvider as _;
    use pii_abi::PiiError;
    use pii_core::{Budget, CapSet, Ctx};
    use std::path::PathBuf;
    use std::sync::Arc;

    /// Per-test scratch file (N8: artifacts under _tmp). Tests run in parallel,
    /// so a shared path would race one assertion against another test's body.
    struct RecordingTransport {
        path: PathBuf,
    }
    impl pii_core::HttpTransport for RecordingTransport {
        fn request(
            &self,
            _ctx: &Ctx,
            req: &pii_abi::HttpRequest,
        ) -> Result<pii_abi::HttpResponse, PiiError> {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            std::fs::write(&self.path, serde_json::to_vec(&body).unwrap()).unwrap();
            Ok(pii_abi::HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: serde_json::to_vec(&serde_json::json!({
                    "message": {"role": "assistant", "content": "ok"}
                }))
                .unwrap(),
            })
        }
    }

    fn contract_tmp_root() -> PathBuf {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn provider_for_contract(path: &std::path::Path) -> OllamaProvider {
        OllamaProvider::new(
            Arc::new(RecordingTransport {
                path: path.to_path_buf(),
            }),
            "http://fake.local:1",
            "m".into(),
            None,
        )
    }

    fn last_body(path: &str) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn ollama_request_forwards_provider_options() {
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::new(Role::User, "hi")],
            tools: Vec::new(),
            max_tokens: 2048,
            temperature: 0.7,
            num_ctx: 16_384,
        };
        let _ =
            provider_for_contract(&contract_tmp_root().join("ollama-last-request-forwards.json"))
                .chat(&ctx, &req)
                .unwrap();
        let body = last_body(
            &contract_tmp_root()
                .join("ollama-last-request-forwards.json")
                .to_string_lossy(),
        );
        assert_eq!(body["stream"], serde_json::json!(false));
        // The wire carries f32; JSON numbers widen 0.7f32 to 0.699999988079071.
        let got = body["options"]["temperature"].as_f64().unwrap();
        assert!((got - 0.7).abs() < 1e-6, "temperature {got} != 0.7");
        assert_eq!(body["options"]["num_predict"], serde_json::json!(2048));
        assert_eq!(body["options"]["num_ctx"], serde_json::json!(16_384));
    }

    #[test]
    fn ollama_request_defaults_num_ctx() {
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::new(Role::User, "hi")],
            tools: Vec::new(),
            max_tokens: 100,
            temperature: 0.0,
            num_ctx: 0,
        };
        let _ =
            provider_for_contract(&contract_tmp_root().join("ollama-last-request-defaults.json"))
                .chat(&ctx, &req)
                .unwrap();
        let body = last_body(
            &contract_tmp_root()
                .join("ollama-last-request-defaults.json")
                .to_string_lossy(),
        );
        assert_eq!(body["options"]["num_ctx"], serde_json::json!(8192));
    }
    /// R4 (spec 15): the scripted provider's streaming path hands the full
    /// turn text to `on_delta` exactly once, and the buffered `chat` contract
    /// is unchanged (G7).
    #[test]
    fn scripted_provider_streams_full_text() {
        let dir = tempfile::Builder::new()
            .prefix("scripted-stream-")
            .tempdir_in(contract_tmp_root())
            .unwrap();
        let fixture = dir.path().join("turns.json");
        std::fs::write(
            &fixture,
            r#"{"turns":[{"text":"full turn text","tool_calls":[],"tokens_in":2,"tokens_out":3},{"text":"full turn text","tool_calls":[],"tokens_in":2,"tokens_out":3}]}"#,
        )
        .unwrap();
        let provider = ScriptedProvider::from_fixture(&fixture).unwrap();
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::new(Role::User, "hi")],
            tools: Vec::new(),
            max_tokens: 64,
            temperature: 0.0,
            num_ctx: 0,
        };
        let deltas: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
        let resp = provider
            .chat_streaming(&ctx, &req, &mut |t| {
                deltas.borrow_mut().push(t.to_owned());
            })
            .unwrap();
        assert_eq!(resp.text, "full turn text");
        assert_eq!(deltas.borrow().len(), 1, "one delta, full text");
        assert_eq!(deltas.borrow()[0], "full turn text");
        // Buffered chat contract unchanged (G7).
        let resp2 = provider.chat(&ctx, &req).unwrap();
        assert_eq!(resp2.text, "full turn text");
    }
}

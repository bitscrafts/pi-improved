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
    /// Context window (`num_ctx`); `0` means "provider default". Providers
    /// whose wire has no such knob (the OpenAI-compatible kinds) ignore it.
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

mod wire;

pub mod anthropic;
pub mod gemini;
pub mod llamacpp_server;
pub mod openai;
pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use openai::OpenAiProvider;

#[cfg(feature = "llamacpp")]
pub mod llamacpp;
#[cfg(feature = "llamacpp")]
pub use llamacpp::LlamaCppProvider;

pub mod llamacpp_tools;

mod observed;
pub use observed::ObservedProvider;

/// Resolves a model spec: `scripted:<path>` needs no transport.
///
/// The networked kinds (`llamacpp-server:`, `openai:`, `anthropic:`,
/// `gemini:`) cannot be built here: they need an `HttpTransport` (the real one
/// is `pii-net`'s `NetService`, constructed by the caller with its egress
/// policy already wired), so use [`resolve_with_transport`] or
/// [`resolve_with_observer`]. `ollama:` was retired (spec 129) and is refused
/// by name.
///
/// # Errors
/// [`PiiError::Invalid`] for an unknown spec, a fixture failure, a networked
/// spec (needs a transport), or a retired `ollama:` spec.
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
        "ollama" => Err(ollama_retired()),
        "llamacpp-server" | "openai" | "anthropic" | "gemini" => Err(PiiError::Invalid(format!(
            "{kind}: needs a transport — call {kind}::provider_from_env or resolve_with_transport"
        ))),
        other => Err(PiiError::Invalid(format!(
            "unknown model spec kind '{other}' (supported: scripted:<path>, llamacpp-server:<model>, \
             openai:<model>, anthropic:<model>, gemini:<model>, llamacpp:<path>)"
        ))),
    }
}

/// The refusal for the retired `ollama:` kind (spec 129): the local model kind
/// is `llamacpp-server:<model>` (an HTTP llama.cpp server), never ollama.
fn ollama_retired() -> PiiError {
    PiiError::Invalid(String::from(
        "ollama: is retired (spec 129) — serve the model with llama.cpp and use \
         llamacpp-server:<model> with `[llama] host`; llamacpp:<path> is the separate in-process GGUF loader",
    ))
}

/// The egress host a spec's provider will dial, or `None` for a local spec.
///
/// This is what the `http:<host>` grant is keyed on. `scripted:`, `llamacpp:`
/// and unknown specs need no network. Derived from the same env var the provider reads for
/// its base URL, falling back to the same default. Only the host: never a
/// port, userinfo, path or key. A `llamacpp-server:` spec uses the default
/// loopback host; use [`egress_host_with`] to supply the configured one.
#[must_use]
pub fn egress_host(spec: &str) -> Option<String> {
    egress_host_with(spec, None)
}

/// [`egress_host`] with the configured `[llama] host` for `llamacpp-server:`
/// specs (`None` = [`llamacpp_server::DEFAULT_LLAMA_HOST`]).
#[must_use]
pub fn egress_host_with(spec: &str, llama_host: Option<&str>) -> Option<String> {
    let (base_var, default) = match spec.split_once(':')?.0 {
        "llamacpp-server" => {
            return Some(wire::host_of(
                llama_host.unwrap_or(llamacpp_server::DEFAULT_LLAMA_HOST),
            ));
        }
        "openai" => ("OPENAI_BASE_URL", openai::DEFAULT_OPENAI_BASE_URL),
        "anthropic" => ("ANTHROPIC_BASE_URL", anthropic::DEFAULT_ANTHROPIC_BASE_URL),
        "gemini" => ("GEMINI_BASE_URL", gemini::DEFAULT_GEMINI_BASE_URL),
        _ => return None,
    };
    let base = pii_core::env_var(base_var).unwrap_or_else(|| default.to_owned());
    Some(wire::host_of(&base))
}

/// Resolves any supported model spec.
///
/// Constructs a networked provider through `transport` when the spec calls
/// for one. This is the entry point a real caller (the daemon, `pii -p`)
/// should use instead of [`provider_from_spec`] alone.
///
/// `llama_host` is the configured `[llama] host` for `llamacpp-server:` specs
/// (`None` = loopback default).
///
/// # Errors
/// [`PiiError::Invalid`] for an unknown spec or a fixture/env failure.
pub fn resolve_with_transport(
    spec: &str,
    transport: Arc<dyn pii_core::HttpTransport>,
    llama_host: Option<&str>,
) -> Result<Arc<dyn ModelProvider>, PiiError> {
    resolve(spec, transport, None, llama_host)
}

/// [`resolve_with_transport`] plus an observer for the networked kinds.
///
/// `llamacpp-server:`, `openai:`, `anthropic:` and `gemini:` providers get `observer`, so their
/// named `model.provider.*` events (auth failure, rate limit, bad body, egress denial, stream delta)
/// reach it.
///
/// # Errors
/// As [`resolve_with_transport`].
pub fn resolve_with_observer(
    spec: &str,
    transport: Arc<dyn pii_core::HttpTransport>,
    observer: Arc<dyn pii_observe::Observer>,
) -> Result<Arc<dyn ModelProvider>, PiiError> {
    resolve(spec, transport, Some(observer), None)
}

fn resolve(
    spec: &str,
    transport: Arc<dyn pii_core::HttpTransport>,
    observer: Option<Arc<dyn pii_observe::Observer>>,
    llama_host: Option<&str>,
) -> Result<Arc<dyn ModelProvider>, PiiError> {
    let env = &pii_core::env_var;
    match spec.split_once(':').map(|(k, _)| k) {
        Some("ollama") => Err(ollama_retired()),
        Some("llamacpp-server") => llamacpp_server::provider(spec, transport, llama_host, observer),
        Some("openai") => openai::provider_from_lookup(spec, transport, env, observer),
        Some("anthropic") => anthropic::provider_from_lookup(spec, transport, env, observer),
        Some("gemini") => gemini::provider_from_lookup(spec, transport, env, observer),
        _ => provider_from_spec(spec),
    }
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

    /// Spec 129: `ollama:` is refused by name, pointing at `llamacpp-server:`,
    /// and a truly unknown kind still gets the generic message.
    #[test]
    fn an_ollama_model_is_refused_pointing_at_llamacpp_server() {
        let err = match provider_from_spec("ollama:granite4.2:3b") {
            Ok(_) => panic!("ollama: must be refused"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("llamacpp-server:"), "got: {err}");
        assert!(err.contains("retired"), "got: {err}");
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

#[cfg(test)]
mod contract {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing)] // reason: json pointer-style asserts on test bodies

    use super::{ChatMessage, ChatRequest, Role, ScriptedProvider};
    use crate::ModelProvider as _;
    use pii_core::{Budget, CapSet, Ctx};
    use std::path::PathBuf;

    fn contract_tmp_root() -> PathBuf {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
        std::fs::create_dir_all(&root).unwrap();
        root
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

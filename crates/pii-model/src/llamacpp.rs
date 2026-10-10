//! In-process LLM support via `llama-cpp-2` (spec 35, research doc 015).
//!
//! [`LlamaCppProvider`] embeds a GGUF model (e.g. Granite 4.2) directly in the
//! Pii process — no external `llama-server`, no `NET` capability. The
//! model load is a pure `fs_read` on the GGUF path (R5).
//!
//! `ModelProvider` is sync on purpose (`GraphNode::run` is sync, runs on a
//! blocking thread). `llama-cpp-2` is also fully sync — `ctx.decode()` is a
//! CPU-bound call, not an I/O wait — so `chat` calls straight into the
//! tokenize→batch→decode→sample loop. `LlamaContext` is not `Send`/`Sync`, so a
//! fresh context is created per call (borrowing the shared `Arc<LlamaModel>`),
//! serialized by a `Mutex<()>` — the "one model slot at a time" semantics the
//! granite probe already designed around.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use parking_lot::Mutex;
use pii_abi::{Origin, PiiError, ProviderFailure};
use pii_core::Ctx;

use crate::llamacpp_tools::{build_schema_from_tools, parse_tool_calls};
use crate::{ChatMessage, ChatRequest, ChatResponse, ModelProvider, Role};

/// Default context window when `PII_LLAMACPP_NUM_CTX` is unset.
pub const DEFAULT_NUM_CTX: u32 = 4096;

/// Builds the R5 context-overflow `ProviderFailure` (spec 39): the two
/// `i32::try_from` guards in [`LlamaCppProvider::run`] share this constructor
/// so the message text stays byte-identical to `classify`'s `CONTEXT`
/// pattern (`"prompt too long"`).
fn context_overflow() -> PiiError {
    PiiError::Provider(ProviderFailure {
        status: None,
        origin: Origin::Provider,
        provider: String::from("llamacpp"),
        message: String::from("llamacpp: prompt too long"),
    })
}

/// One in-process GGUF model provider.
pub struct LlamaCppProvider {
    model: Arc<LlamaModel>,
    backend: Arc<LlamaBackend>,
    n_ctx: u32,
    n_threads: i32,
    /// Serializes context creation/use (one model slot at a time).
    lock: Mutex<()>,
}

impl LlamaCppProvider {
    /// Loads a GGUF model from `path`. The file must exist (a missing file is
    /// a clear [`PiiError::Invalid`], not a silent fallback).
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when the file is missing or the model fails to
    /// load; [`PiiError::Internal`] on backend init failure.
    pub fn load(path: &Path, n_ctx: u32, n_threads: i32) -> Result<Self, PiiError> {
        if !path.exists() {
            return Err(PiiError::Invalid(format!(
                "llamacpp: model file not found: {}",
                path.display()
            )));
        }
        let backend = LlamaBackend::init()
            .map_err(|e| PiiError::Internal(format!("llamacpp: backend init: {e}")))?;
        let model = LlamaModel::load_from_file(&backend, path, &LlamaModelParams::default())
            .map_err(|e| PiiError::Invalid(format!("llamacpp: load {}: {e}", path.display())))?;
        Ok(Self {
            model: Arc::new(model),
            backend: Arc::new(backend),
            n_ctx,
            n_threads,
            lock: Mutex::new(()),
        })
    }

    /// Runs one completion over the transcript, returning the accumulated text
    /// and token counts. `on_delta` receives each decoded piece (streaming).
    #[allow(clippy::too_many_lines)]
    fn run(
        &self,
        req: &ChatRequest,
        mut on_delta: impl FnMut(&str),
    ) -> Result<ChatResponse, PiiError> {
        let _g = self.lock.lock();
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(self.n_ctx))
            .with_n_threads(self.n_threads)
            .with_n_threads_batch(self.n_threads);
        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| PiiError::Internal(format!("llamacpp: context: {e}")))?;

        // Hand-built Granite prompt (non-thinking by construction, R3).
        let prompt = granite::build_prompt(&req.messages);
        let tokens = self
            .model
            .str_to_token(&prompt, AddBos::Always)
            .map_err(|e| PiiError::Internal(format!("llamacpp: tokenize: {e}")))?;

        // Prefill: one batch with all prompt tokens, logits on the last.
        let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
        for (i, &tok) in tokens.iter().enumerate() {
            let is_last = i == tokens.len() - 1;
            let pos = i32::try_from(i).map_err(|_| context_overflow())?;
            batch
                .add(tok, pos, &[0], is_last)
                .map_err(|e| PiiError::Internal(format!("llamacpp: batch add: {e}")))?;
        }
        ctx.decode(&mut batch)
            .map_err(|e| PiiError::Internal(format!("llamacpp: decode: {e}")))?;

        // Sample loop. When tools are requested, constrain generation to a
        // tool-call JSON via llama.cpp's grammar (R1). Otherwise stay greedy
        // (deterministic, non-thinking) for plain text.
        let sampler = if req.tools.is_empty() {
            LlamaSampler::chain_simple([LlamaSampler::greedy()])
        } else {
            let schema = build_schema_from_tools(&req.tools);
            let grammar = llama_cpp_2::json_schema_to_grammar(&schema)
                .map_err(|e| PiiError::Internal(format!("llamacpp: schema_to_grammar: {e}")))?;
            let grammar_sampler = LlamaSampler::grammar(&self.model, &grammar, "root")
                .map_err(|e| PiiError::Internal(format!("llamacpp: grammar: {e}")))?;
            // The grammar sampler only constrains logits; a chain still needs
            // a final selection sampler or llama.cpp's candidate-selection
            // assertion aborts the process (GGML_ASSERT cur_p.selected>=0).
            LlamaSampler::chain_simple([grammar_sampler, LlamaSampler::greedy()])
        };
        let mut sampler = sampler;
        let mut text = String::new();
        let mut tokens_out: u64 = 0;
        let mut pos = i32::try_from(tokens.len()).map_err(|_| context_overflow())?;
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        loop {
            // `LlamaSampler::sample` already accepts the token into every
            // sampler in the chain (it's llama.cpp's combined "sample and
            // accept" entry point). A second explicit `accept()` here would
            // replay the same bytes into the grammar sampler, which has
            // already advanced past them — the grammar's parse stacks would
            // be driven empty, and the *next* decode step's logit-masking
            // pass aborts the process via a C++ `GGML_ASSERT`. Do not call
            // `sampler.accept()` again after `sample()`.
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            if self.model.is_eog_token(token) {
                break;
            }
            let piece = self
                .model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|e| PiiError::Internal(format!("llamacpp: decode piece: {e}")))?;
            on_delta(&piece);
            text.push_str(&piece);
            tokens_out += 1;
            if req.max_tokens > 0 && tokens_out >= u64::from(req.max_tokens) {
                break;
            }
            batch.clear();
            batch
                .add(token, pos, &[0], true)
                .map_err(|e| PiiError::Internal(format!("llamacpp: batch add: {e}")))?;
            pos += 1;
            ctx.decode(&mut batch)
                .map_err(|e| PiiError::Internal(format!("llamacpp: decode: {e}")))?;
        }

        let tool_calls = if req.tools.is_empty() {
            Vec::new()
        } else {
            parse_tool_calls(&text)
        };
        Ok(ChatResponse {
            text,
            tool_calls,
            tokens_in: tokens.len() as u64,
            tokens_out,
            micro_usd: 0,
        })
    }
}

impl ModelProvider for LlamaCppProvider {
    fn name(&self) -> &'static str {
        "llamacpp"
    }

    fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        let _ = ctx; // no capability check beyond the fs_read at load time
        self.run(req, |_| {})
    }

    fn chat_streaming(
        &self,
        ctx: &Ctx,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, PiiError> {
        let _ = ctx;
        self.run(req, |piece| on_delta(piece))
    }
}

/// Granite 4.x wire-format prompt builder (R3).
///
/// Builds the `<|start_of_role|>{role}<|end_of_role|>{content}<|end_of_text|>`
/// format **by hand** — NOT `apply_chat_template` — so non-thinking mode is an
/// explicit, unit-testable function (doc 015 §3 verification gap). The legacy
/// llama.cpp renderer's Granite branch emits no ` thinking` scaffolding, and
/// this hand-built form matches that shape by construction.
pub mod granite {
    use super::{ChatMessage, Role};

    /// Renders a transcript to the Granite 4.x wire format.
    ///
    /// Ends with the `<|start_of_role|>assistant<|end_of_role|>` header so the
    /// model generates the assistant turn (the template's generation trigger),
    /// not a continuation of the wire-format echo.
    #[must_use]
    pub fn build_prompt(messages: &[ChatMessage]) -> String {
        let mut out = String::new();
        for m in messages {
            out.push_str("<|start_of_role|>");
            out.push_str(role_name(m.role));
            out.push_str("<|end_of_role|>");
            out.push_str(&m.content);
            out.push_str("<|end_of_text|>");
        }
        // Generation trigger: the assistant role header with no content yet.
        out.push_str("<|start_of_role|>assistant<|end_of_role|>");
        out
    }

    fn role_name(role: Role) -> &'static str {
        match role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

/// Resolves a `llamacpp:<path>` spec using the process environment.
///
/// `PII_LLAMACPP_NUM_CTX` (default [`DEFAULT_NUM_CTX`]) and
/// `PII_LLAMACPP_THREADS` (default: all cores, `-1`) come via
/// `pii_core::env_var`. No transport needed (unlike `llamacpp-server:`).
///
/// # Errors
/// [`PiiError::Invalid`] for an empty path or a missing/unloadable model file.
pub fn provider_from_env(spec: &str) -> Result<Arc<dyn ModelProvider>, PiiError> {
    let Some((kind, rest)) = spec.split_once(':') else {
        return Err(PiiError::Invalid(format!("unknown model spec '{spec}'")));
    };
    match kind {
        "llamacpp" => {
            if rest.is_empty() {
                return Err(PiiError::Invalid(
                    "llamacpp: spec needs a model path".into(),
                ));
            }
            let n_ctx = pii_core::env_var("PII_LLAMACPP_NUM_CTX")
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_NUM_CTX);
            let n_threads = pii_core::env_var("PII_LLAMACPP_THREADS")
                .and_then(|s| s.parse().ok())
                .unwrap_or(-1);
            Ok(Arc::new(LlamaCppProvider::load(
                Path::new(rest),
                n_ctx,
                n_threads,
            )?))
        }
        other => Err(PiiError::Invalid(format!(
            "unknown model spec kind '{other}' (scripted:<path>, llamacpp:<path> are supported)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::granite::build_prompt;
    use crate::{ChatMessage, Role};

    #[test]
    fn granite_prompt_builds_wire_format() {
        let msgs = vec![
            ChatMessage::new(Role::System, "You are helpful."),
            ChatMessage::new(Role::User, "Hello"),
        ];
        let p = build_prompt(&msgs);
        assert_eq!(
            p,
            "<|start_of_role|>system<|end_of_role|>You are helpful.<|end_of_text|>\
             <|start_of_role|>user<|end_of_role|>Hello<|end_of_text|>\
             <|start_of_role|>assistant<|end_of_role|>"
        );
    }

    #[test]
    fn granite_prompt_omits_thinking() {
        let msgs = vec![
            ChatMessage::new(Role::System, "sys"),
            ChatMessage::new(Role::User, "hi"),
            ChatMessage::new(Role::Assistant, "hello there"),
        ];
        let p = build_prompt(&msgs);
        assert!(!p.contains(" thinking"), "no thinking scaffolding: {p}");
        assert!(!p.contains("enable_thinking"), "no enable_thinking: {p}");
        assert!(p.contains("hello there"));
    }

    /// R3: a request with no tools must not build a grammar; the pure prompt
    /// builder still renders a wire-format transcript.
    #[test]
    fn empty_tools_keeps_text_path() {
        let msgs = vec![
            ChatMessage::new(Role::System, "sys"),
            ChatMessage::new(Role::User, "hi"),
        ];
        let p = build_prompt(&msgs);
        assert!(p.contains("<|start_of_role|>assistant<|end_of_role|>"));
        assert!(!p.contains("tool"), "no tool scaffolding in text path: {p}");
    }

    /// T9 (spec 39 R5): the shared context-overflow constructor produces a
    /// `ProviderFailure` that `classify_provider` maps to `ContextOverflow`
    /// via the `CONTEXT` pattern's literal `"prompt too long"` entry.
    #[test]
    fn llamacpp_context_overflow_constructs_provider_failure() {
        use pii_abi::{FailoverReason, PiiError, classify_provider};

        let err = super::context_overflow();
        match err {
            PiiError::Provider(f) => {
                assert_eq!(f.status, None);
                assert_eq!(f.provider, "llamacpp");
                assert_eq!(classify_provider(&f), FailoverReason::ContextOverflow);
            }
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    /// Real-model acceptance (N4: `#[ignore]` — not run in CI). Loads the
    /// actual granite GGUF and runs one chat through the in-process provider,
    /// proving the `llamacpp:` config entry works end-to-end. Run manually:
    /// `cargo test -p pii-model -- --ignored real_granite_chat`.
    #[test]
    #[ignore = "loads a 2.2GB model; manual acceptance only"]
    fn real_granite_chat() {
        use super::LlamaCppProvider;
        use crate::{ChatRequest, ModelProvider};
        use pii_core::{Budget, CapSet, Ctx};

        let path = std::path::Path::new("/projects/_tmp/granite-4.2-3b-Q4_K_M.gguf");
        assert!(path.exists(), "granite GGUF must be present");
        let provider = LlamaCppProvider::load(path, 4096, 16).unwrap();
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        let req = ChatRequest {
            model: "granite".into(),
            messages: vec![ChatMessage::new(
                Role::User,
                "Reply with exactly: GRANITE_OK",
            )],
            tools: Vec::new(),
            max_tokens: 100,
            temperature: 0.0,
            num_ctx: 4096,
        };
        let resp = provider.chat(&ctx, &req).unwrap();
        assert!(
            resp.text.contains("GRANITE_OK"),
            "expected GRANITE_OK in output, got: {:?}",
            resp.text
        );
        assert!(resp.tokens_out > 0, "must generate tokens");
    }

    /// Real-model tool-call acceptance (N4: `#[ignore]` — not run in CI).
    /// Loads granite, gives it one tool, and checks the grammar-constrained
    /// path actually emits a structured `tool_calls` entry — the point of
    /// spec 36 (part1 helpers + part2 wiring). Run manually:
    /// `cargo test -p pii-model -- --ignored real_granite_tool_call`.
    #[test]
    #[ignore = "loads a 2.2GB model; manual acceptance only"]
    fn real_granite_tool_call() {
        use super::LlamaCppProvider;
        use crate::{ChatRequest, ModelProvider, ToolSpec};
        use pii_core::{Budget, CapSet, Ctx};

        let path = std::path::Path::new("/projects/_tmp/granite-4.2-3b-Q4_K_M.gguf");
        assert!(path.exists(), "granite GGUF must be present");
        let provider = LlamaCppProvider::load(path, 4096, 16).unwrap();
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        let tools = vec![ToolSpec {
            name: "get_weather".to_string(),
            description: "Get the current weather for a city".to_string(),
            schema: serde_json::json!({
                "type": "object",
                "properties": { "city": { "type": "string" } },
                "required": ["city"]
            }),
        }];
        let req = ChatRequest {
            model: "granite".into(),
            messages: vec![ChatMessage::new(
                Role::User,
                "What is the weather in Paris? Use the get_weather tool.",
            )],
            tools,
            max_tokens: 200,
            temperature: 0.0,
            num_ctx: 4096,
        };
        let resp = provider.chat(&ctx, &req).unwrap();
        assert!(
            !resp.tool_calls.is_empty(),
            "expected a tool call, got text: {:?}, tool_calls: {:?}",
            resp.text,
            resp.tool_calls
        );
        assert_eq!(
            resp.tool_calls.first().expect("checked above").name,
            "get_weather"
        );
    }
}

//! In-process LLM support via `llama-cpp-2` (spec 35, research doc 015).
//!
//! [`LlamaCppProvider`] embeds a GGUF model (e.g. Granite 4.2) directly in the
//! Pii process — no external `llama-server`/Ollama, no `NET` capability. The
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
use pii_abi::PiiError;
use pii_core::Ctx;

use crate::{ChatMessage, ChatRequest, ChatResponse, ModelProvider, Role};

/// Default context window when `PII_LLAMACPP_NUM_CTX` is unset.
pub const DEFAULT_NUM_CTX: u32 = 4096;

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
            let pos = i32::try_from(i)
                .map_err(|_| PiiError::Internal("llamacpp: prompt too long".into()))?;
            batch
                .add(tok, pos, &[0], is_last)
                .map_err(|e| PiiError::Internal(format!("llamacpp: batch add: {e}")))?;
        }
        ctx.decode(&mut batch)
            .map_err(|e| PiiError::Internal(format!("llamacpp: decode: {e}")))?;

        // Sample loop: greedy (deterministic) — matches the probe's
        // non-thinking, content>0 requirement without a reasoning trace.
        let mut sampler = LlamaSampler::chain_simple([LlamaSampler::greedy()]);
        let mut text = String::new();
        let mut tokens_out: u64 = 0;
        let mut pos = i32::try_from(tokens.len())
            .map_err(|_| PiiError::Internal("llamacpp: prompt too long".into()))?;
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        loop {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);
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

        Ok(ChatResponse {
            text,
            tool_calls: Vec::new(),
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
/// `pii_core::env_var`. No transport needed (unlike `ollama:`).
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
            "unknown model spec kind '{other}' (scripted:<path>, ollama:<model>, llamacpp:<path> are supported)"
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
             <|start_of_role|>user<|end_of_role|>Hello<|end_of_text|>"
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
}

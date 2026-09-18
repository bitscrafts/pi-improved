//! ds4-style in-process benchmark for `LlamaCppProvider` (spec 36 follow-up).
//!
//! ds4-bench speaks `OpenAI` `/v1/chat/completions` over HTTP; Pii's in-process
//! model has no HTTP surface of its own (by design — `pii-web` is
//! viewer-only, no write RPC). This example links `pii-model` directly and
//! replicates ds4-bench's B1 (tool-calling) and B2 (speed) checks in-process,
//! against the real granite GGUF, with no server involved.
//!
//! B3/B4 (multi-round autonomy loops) are NOT replicated here: they require
//! simulating tool execution and feeding results back through a full agent
//! loop, which is `pii_server`'s `agent.rs` concern, not `pii_model`'s. This
//! example benchmarks the provider seam only.
//!
//! Run manually (loads a 2.2GB model, so it is an example, not a test):
//! ```text
//! cargo run -p pii-model --example ds4_inprocess --release
//! ```

// reason: this is a diagnostic example binary; its output IS its purpose.
#![allow(clippy::print_stdout, clippy::print_stderr, clippy::disallowed_macros)]

use std::path::Path;
use std::time::Instant;

use pii_core::{Budget, CapSet, Ctx};
use pii_model::{ChatMessage, ChatRequest, LlamaCppProvider, ModelProvider, Role, ToolSpec};

const MODEL_PATH: &str = "/projects/_tmp/granite-4.2-3b-Q4_K_M.gguf";

fn main() {
    let path = Path::new(MODEL_PATH);
    if !path.exists() {
        eprintln!("SKIP: granite GGUF not found at {MODEL_PATH}");
        std::process::exit(2);
    }
    println!("Loading {MODEL_PATH} ...");
    let load_start = Instant::now();
    let provider = match LlamaCppProvider::load(path, 4096, 16) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("model load failed: {e}");
            std::process::exit(1);
        }
    };
    println!("Loaded in {:.2}s\n", load_start.elapsed().as_secs_f64());

    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());

    b1_tool_calling(&provider, &ctx);
    b2_speed(&provider, &ctx);
}

/// B1 — tool calling: minimal 7-tool payload (mirrors ds4-bench's minimal
/// class), one user turn, checks the model emits a structured tool call.
fn b1_tool_calling(provider: &LlamaCppProvider, ctx: &Ctx) {
    println!("=== B1: tool calling ===");
    let tools = vec![
        ToolSpec {
            name: "terminal".to_string(),
            description: "Run a shell command and return stdout+stderr.".to_string(),
            schema: serde_json::json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }),
        },
        ToolSpec {
            name: "read_file".to_string(),
            description: "Read a file's contents.".to_string(),
            schema: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        },
    ];
    let req = ChatRequest {
        model: "granite".into(),
        messages: vec![ChatMessage::new(
            Role::User,
            "List the files in /root/.hermes/ using the terminal tool.",
        )],
        tools,
        max_tokens: 200,
        temperature: 0.0,
        num_ctx: 4096,
    };
    let start = Instant::now();
    match provider.chat(ctx, &req) {
        Ok(resp) => {
            let elapsed = start.elapsed().as_secs_f64();
            let passed = !resp.tool_calls.is_empty();
            println!(
                "  tool_calls_emitted={} passed={} wall_secs={:.2}",
                resp.tool_calls.len(),
                passed,
                elapsed
            );
            for tc in &resp.tool_calls {
                println!("    -> {} {}", tc.name, tc.arguments);
            }
            if !passed {
                println!("  text (no tool call): {:?}", resp.text);
            }
        }
        Err(e) => println!("  ERROR: {e}"),
    }
    println!();
}

/// B2 — speed: output tok/s on a plain-text generation (no tools).
fn b2_speed(provider: &LlamaCppProvider, ctx: &Ctx) {
    println!("=== B2: speed ===");
    let req = ChatRequest {
        model: "granite".into(),
        messages: vec![ChatMessage::new(
            Role::User,
            "Write a 200-word story about a robot learning to paint.",
        )],
        tools: Vec::new(),
        max_tokens: 300,
        temperature: 0.0,
        num_ctx: 4096,
    };
    let start = Instant::now();
    match provider.chat(ctx, &req) {
        Ok(resp) => {
            let elapsed = start.elapsed().as_secs_f64();
            let tps = if elapsed > 0.0 {
                u32::try_from(resp.tokens_out).map_or(f64::MAX, f64::from) / elapsed
            } else {
                0.0
            };
            println!(
                "  tokens_out={} wall_secs={:.2} tok_per_sec={:.2} tokens_in={}",
                resp.tokens_out, elapsed, tps, resp.tokens_in
            );
        }
        Err(e) => println!("  ERROR: {e}"),
    }
    println!();
}

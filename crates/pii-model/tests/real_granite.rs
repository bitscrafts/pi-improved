//! Real-model integration test suite for the in-process `LlamaCppProvider`
//! ("Pii as a live entity using granite", spec 35/36 follow-up).
//!
//! Every test here loads the real 2.2GB granite GGUF, so all are `#[ignore]`
//! (N4: the automated suite must not load a real model). Run the whole battery
//! manually, single-threaded (the provider shares one model slot):
//!
//! ```text
//! cargo test -p pii-model --test real_granite -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! The model is loaded at most once per test binary via `OnceLock`; all tests
//! share it. Requires `/models/granite/granite-4.2-3b-Q4_K_M.gguf` present (the
//! canonical model store mounted read-only as /models; superseded the old
//! /projects/_tmp path on 2026-10-20 — see docs/03-reference/local-model-store.md).

// reason: integration tests use unwrap/expect/indexing for concise assertions,
// matching the in-crate test modules' convention.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::OnceLock;

use pii_core::{Budget, CapSet, Ctx};
use pii_model::{ChatMessage, ChatRequest, LlamaCppProvider, ModelProvider, Role, ToolSpec};

const MODEL_PATH: &str = "/models/granite/granite-4.2-3b-Q4_K_M.gguf";

/// Shared provider: the 2.2GB model is loaded once for the whole test binary.
fn provider() -> &'static LlamaCppProvider {
    static PROVIDER: OnceLock<LlamaCppProvider> = OnceLock::new();
    PROVIDER.get_or_init(|| {
        let path = Path::new(MODEL_PATH);
        assert!(
            path.exists(),
            "granite GGUF must be present at {MODEL_PATH}"
        );
        LlamaCppProvider::load(path, 4096, 16).expect("granite model must load")
    })
}

fn ctx() -> Ctx {
    Ctx::root(CapSet::empty(), Budget::unlimited())
}

fn tool(name: &str, description: &str, arg: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": { arg: { "type": "string" } },
            "required": [arg]
        }),
    }
}

fn req(user: &str, tools: Vec<ToolSpec>, max_tokens: u32) -> ChatRequest {
    ChatRequest {
        model: "granite".into(),
        messages: vec![ChatMessage::new(Role::User, user)],
        tools,
        max_tokens,
        temperature: 0.0,
        num_ctx: 4096,
    }
}

// ---------------------------------------------------------------------------
// 1. Text path (no tools)
// ---------------------------------------------------------------------------

/// Text-only generation returns non-empty content and no tool calls.
#[test]
#[ignore = "loads a 2.2GB model"]
fn text_completion_returns_content() {
    let r = provider().chat(
        &ctx(),
        &req("Reply with exactly: PIITEXT_OK", Vec::new(), 60),
    );
    let r = r.expect("chat must succeed");
    assert!(
        r.text.contains("PIITEXT_OK"),
        "expected PIITEXT_OK, got {:?}",
        r.text
    );
    assert!(r.tool_calls.is_empty(), "no tools -> no tool_calls");
    assert!(r.tokens_out > 0, "must generate tokens");
}

// ---------------------------------------------------------------------------
// 2. Tool-call structure
// ---------------------------------------------------------------------------

/// A single tool request yields a structured tool call with the right name.
#[test]
#[ignore = "loads a 2.2GB model"]
fn single_tool_call_is_structured() {
    let tools = vec![tool("get_weather", "Get the weather for a city", "city")];
    let r = provider().chat(
        &ctx(),
        &req("What is the weather in Paris? Use get_weather.", tools, 200),
    );
    let r = r.expect("chat must succeed");
    assert!(
        !r.tool_calls.is_empty(),
        "expected a tool call; text was {:?}",
        r.text
    );
    assert_eq!(r.tool_calls[0].name, "get_weather");
}

/// Tool-call arguments are a JSON object (the grammar guarantees shape) and
/// carry the expected argument key.
#[test]
#[ignore = "loads a 2.2GB model"]
fn tool_arguments_are_json_object() {
    let tools = vec![tool("get_weather", "Get the weather for a city", "city")];
    let r = provider().chat(
        &ctx(),
        &req("What is the weather in Paris? Use get_weather.", tools, 200),
    );
    let r = r.expect("chat must succeed");
    let call = r.tool_calls.first().expect("one tool call");
    assert!(
        call.arguments.is_object(),
        "arguments must be a JSON object, got {:?}",
        call.arguments
    );
    assert!(
        call.arguments.get("city").is_some(),
        "arguments must contain 'city', got {:?}",
        call.arguments
    );
}

// ---------------------------------------------------------------------------
// 3. Tool selection among many (the real agent skill)
// ---------------------------------------------------------------------------

/// Given several tools, the model picks the one the task names.
#[test]
#[ignore = "loads a 2.2GB model"]
fn selects_named_tool_among_many() {
    let tools = vec![
        tool("get_weather", "Get the weather for a city", "city"),
        tool("read_file", "Read a file's contents", "path"),
        tool("send_email", "Send an email", "to"),
    ];
    let r = provider().chat(
        &ctx(),
        &req(
            "Read the file /etc/hosts using the read_file tool.",
            tools,
            200,
        ),
    );
    let r = r.expect("chat must succeed");
    assert!(
        !r.tool_calls.is_empty(),
        "expected a tool call; text was {:?}",
        r.text
    );
    assert_eq!(
        r.tool_calls[0].name, "read_file",
        "must select read_file; got {:?}",
        r.tool_calls
    );
}

/// Selecting a different tool from the same set (weather, not file).
#[test]
#[ignore = "loads a 2.2GB model"]
fn selects_other_tool_from_same_set() {
    let tools = vec![
        tool("get_weather", "Get the weather for a city", "city"),
        tool("read_file", "Read a file's contents", "path"),
        tool("send_email", "Send an email", "to"),
    ];
    let r = provider().chat(
        &ctx(),
        &req("Email alice using the send_email tool.", tools, 200),
    );
    let r = r.expect("chat must succeed");
    assert!(
        !r.tool_calls.is_empty(),
        "expected a tool call; text was {:?}",
        r.text
    );
    assert_eq!(r.tool_calls[0].name, "send_email");
}

// ---------------------------------------------------------------------------
// 4. Text vs tool-call branch (R3)
// ---------------------------------------------------------------------------

/// With no tools, the model never emits a tool call even if asked to "call a tool".
#[test]
#[ignore = "loads a 2.2GB model"]
fn empty_tools_never_emits_tool_calls() {
    let r = provider().chat(
        &ctx(),
        &req(
            "Call the read_file tool to read /etc/hosts.",
            Vec::new(),
            120,
        ),
    );
    let r = r.expect("chat must succeed");
    assert!(
        r.tool_calls.is_empty(),
        "no tools configured -> no tool_calls, got {:?}",
        r.tool_calls
    );
}

// ---------------------------------------------------------------------------
// 5. Streaming path (R4)
// ---------------------------------------------------------------------------

/// `chat_streaming` forwards the same text as `chat` and carries tool calls.
#[test]
#[ignore = "loads a 2.2GB model"]
fn streaming_carries_tool_calls() {
    let tools = vec![tool("get_weather", "Get the weather for a city", "city")];
    let r = req("What is the weather in Paris? Use get_weather.", tools, 200);
    let mut deltas = String::new();
    let resp = provider()
        .chat_streaming(&ctx(), &r, &mut |piece| deltas.push_str(piece))
        .expect("streaming chat must succeed");
    // The buffered response and the streamed text must agree.
    assert_eq!(
        resp.text, deltas,
        "streamed deltas must reconstruct the buffered text"
    );
    assert!(
        !resp.tool_calls.is_empty(),
        "streaming path must still carry tool_calls; text was {:?}",
        resp.text
    );
}

// ---------------------------------------------------------------------------
// 6. Determinism (greedy decode, temperature 0)
// ---------------------------------------------------------------------------

/// Two identical requests produce identical results (greedy, temp 0).
#[test]
#[ignore = "loads a 2.2GB model"]
fn deterministic_under_greedy() {
    let tools = vec![tool("get_weather", "Get the weather for a city", "city")];
    let r = req("What is the weather in Paris? Use get_weather.", tools, 200);
    let a = provider().chat(&ctx(), &r).expect("chat a");
    let b = provider().chat(&ctx(), &r).expect("chat b");
    assert_eq!(a.text, b.text, "greedy decode must be deterministic");
    assert_eq!(
        a.tool_calls.len(),
        b.tool_calls.len(),
        "tool_calls must be stable"
    );
}

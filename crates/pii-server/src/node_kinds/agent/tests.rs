//! Tests for the `agent` node — moved verbatim (2026-09-17) from the
//! `mod tests` block of the former single-file `agent.rs`. A move, not a
//! rewrite: no assertion, name, or test count changed.
//!
//! The module is compiled only under `cfg(test)` (mirrored by the inner
//! attribute below, matching the `#[cfg(test)] mod tests;` declaration in
//! `mod.rs`); per the 450-line gate's F15 semantics, test code does not
//! count against the budget.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(test)]

use parking_lot::Mutex;
use std::sync::Arc;

use pii_abi::{
    FailoverReason, FailurePayload, NodeInput, Origin, Outcome, PiiError, PortName, ProviderFailure,
};
use pii_core::{Budget, CapSet, Ctx};
use pii_graph::node::{GraphNode, NodeFactory as _};
use pii_model::{ChatRequest, Role};

use super::{AgentNode, ModelProvider};

/// Records every [`ChatRequest`] it receives and always answers with
/// a fixed, tool-free reply — enough to inspect exactly what
/// `AgentNode::run` sent the model without needing a real transport
/// or the `scripted:` fixture format (which ignores the request
/// entirely and can't tell us whether history was seeded).
struct RecordingProvider {
    seen: Mutex<Vec<ChatRequest>>,
}

impl ModelProvider for RecordingProvider {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn chat(&self, _ctx: &Ctx, req: &ChatRequest) -> Result<pii_model::ChatResponse, PiiError> {
        self.seen.lock().push(req.clone());
        Ok(pii_model::ChatResponse {
            text: String::from("ok"),
            tool_calls: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            micro_usd: 0,
        })
    }
}

fn test_node(provider: Arc<RecordingProvider>) -> AgentNode {
    AgentNode {
        tier: None,
        system: String::from("You are pii."),
        prompt: None,
        route_step: None,
        max_turns: 4,
        model: None,
        provider,
        toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
        workspace: std::path::PathBuf::from("."),
        live: None,
        compaction: crate::node_kinds::compaction::Compaction::default(),
        memory: None,
    }
}

fn test_ctx() -> Ctx {
    Ctx::root(CapSet::empty(), Budget::unlimited())
}

/// R1 (spec 40): the agent factory advertises the `fail` port after
/// `ok`/`err`/`budget` (appended last, order-preserving for the existing
/// positional fixtures).
#[test]
fn agent_factory_ports_include_fail() {
    let ports = crate::node_kinds::agent::AgentFactory {
        provider: Arc::new(RecordingProvider {
            seen: Mutex::new(Vec::new()),
        }),
        route_policy: std::collections::BTreeMap::new(),
        tiers: None,
        workspace: std::path::PathBuf::from("."),
        live: None,
        memory: None,
    }
    .ports(&serde_json::json!({}))
    .expect("ports");
    let names: Vec<&str> = ports.iter().map(PortName::as_str).collect();
    assert_eq!(
        names,
        vec!["ok", "err", "budget", "fail"],
        "fail appended last, ok/err/budget order preserved"
    );
}

/// A provider whose `chat` always returns a fixed error. The default
/// `chat_streaming` routes through `chat`, so both the streaming call and
/// (when retried) the plain call fail with the same error.
struct FailingProvider {
    error: PiiError,
}

impl ModelProvider for FailingProvider {
    fn name(&self) -> &'static str {
        "failing"
    }

    fn chat(&self, _ctx: &Ctx, _req: &ChatRequest) -> Result<pii_model::ChatResponse, PiiError> {
        Err(self.error.clone())
    }
}

fn fail_input() -> NodeInput {
    NodeInput {
        input: serde_json::Value::Null,
        payload: serde_json::json!({ "prompt": "hi" }),
        nodes: serde_json::Value::Null,
    }
}

/// T3 (spec 40, the 026 acceptance test): a `PiiError::Provider`
/// classified `ContextOverflow` routes to the `fail` port carrying a
/// `FailurePayload`, not to `err` and not a hard node `Err`.
#[test]
fn provider_failure_with_context_overflow_routes_to_fail() {
    let err = PiiError::Provider(ProviderFailure {
        status: None,
        origin: Origin::Provider,
        provider: String::from("llamacpp"),
        message: String::from("llamacpp: prompt too long"),
    });
    let node = AgentNode {
        tier: None,
        system: String::from("You are pii."),
        prompt: None,
        route_step: None,
        max_turns: 4,
        model: None,
        provider: Arc::new(FailingProvider { error: err }),
        toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
        workspace: std::path::PathBuf::from("."),
        live: None,
        compaction: crate::node_kinds::compaction::Compaction::default(),
        memory: None,
    };
    let outcome = node.run(&test_ctx(), &fail_input()).expect("run");
    let Outcome::Next { port, value } = outcome else {
        panic!("expected an outcome, got a termination");
    };
    assert_eq!(port.as_str(), "fail", "provider failure must route to fail");
    let payload: FailurePayload = serde_json::from_value(value).expect("fail payload deserializes");
    assert_eq!(
        payload,
        FailurePayload {
            status: None,
            reason: FailoverReason::ContextOverflow,
            provider: String::from("llamacpp"),
        }
    );
}

/// T4 (spec 40): a retryable provider failure (503 -> `ServerError` ->
/// `RetrySame`) whose retry also fails routes to `fail` after the retry
/// is exhausted — the retry-then-classify path, not just the
/// immediately-non-retryable one.
#[test]
fn provider_failure_5xx_routes_to_fail_after_retry_exhausted() {
    let err = PiiError::Provider(ProviderFailure {
        status: Some(503),
        origin: Origin::Provider,
        provider: String::from("llamacpp-server"),
        message: String::from("upstream unavailable"),
    });
    assert!(err.is_retryable(), "503 must be retryable per spec-39");
    let node = AgentNode {
        tier: None,
        system: String::from("You are pii."),
        prompt: None,
        route_step: None,
        max_turns: 4,
        model: None,
        provider: Arc::new(FailingProvider { error: err }),
        toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
        workspace: std::path::PathBuf::from("."),
        live: None,
        compaction: crate::node_kinds::compaction::Compaction::default(),
        memory: None,
    };
    let outcome = node.run(&test_ctx(), &fail_input()).expect("run");
    let Outcome::Next { port, value } = outcome else {
        panic!("expected an outcome, got a termination");
    };
    assert_eq!(
        port.as_str(),
        "fail",
        "retry-exhausted provider goes to fail"
    );
    let payload: FailurePayload = serde_json::from_value(value).expect("fail payload deserializes");
    assert_eq!(
        payload.reason,
        FailoverReason::Overloaded,
        "503 must classify to Overloaded (RetrySame) per spec-39"
    );
}

/// T5 (spec 40, negative case): a non-`Provider` error still routes to
/// `err` with the pre-A3 free-form `{"error": <Display>}` shape — R3 must
/// not widen what counts as a `fail`.
#[test]
fn non_provider_error_still_routes_to_err() {
    let node = AgentNode {
        tier: None,
        system: String::from("You are pii."),
        prompt: None,
        route_step: None,
        max_turns: 4,
        model: None,
        provider: Arc::new(FailingProvider {
            error: PiiError::Invalid(String::from("bad request shape")),
        }),
        toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
        workspace: std::path::PathBuf::from("."),
        live: None,
        compaction: crate::node_kinds::compaction::Compaction::default(),
        memory: None,
    };
    let outcome = node.run(&test_ctx(), &fail_input()).expect("run");
    let Outcome::Next { port, value } = outcome else {
        panic!("expected an outcome, got a termination");
    };
    assert_eq!(port.as_str(), "err", "non-provider error stays on err");
    assert_eq!(
        value,
        serde_json::json!({ "error": "invalid: bad request shape" }),
        "err port keeps the free-form {{error: Display}} shape"
    );
}

/// 2026-09-05 (session memory): `history` in the run's input payload
/// must be seeded into the model's message list, in order, BEFORE
/// the current turn's user message — this is the exact fix for
/// "do you remember previous questions?" always saying no.
#[test]
fn history_from_input_seeds_prior_turns() {
    let provider = Arc::new(RecordingProvider {
        seen: Mutex::new(Vec::new()),
    });
    let node = test_node(provider.clone());
    let input = NodeInput {
        input: serde_json::Value::Null,
        payload: serde_json::json!({
            "prompt": "what did I ask first?",
            "history": [
                {"role": "user", "content": "what is 3 times 2?"},
                {"role": "assistant", "content": "6"},
            ],
        }),
        nodes: serde_json::Value::Null,
    };
    node.run(&test_ctx(), &input).expect("run");

    let sent = provider
        .seen
        .lock()
        .first()
        .expect("one call recorded")
        .messages
        .clone();
    // [System, User(prior q), Assistant(prior a), User(current q)].
    assert_eq!(sent.len(), 4, "unexpected message shape: {sent:?}");
    assert_eq!(sent.get(1).expect("msg 1").content, "what is 3 times 2?");
    assert_eq!(sent.get(2).expect("msg 2").content, "6");
    assert_eq!(sent.get(3).expect("msg 3").content, "what did I ask first?");
}

/// 2026-09-05: absent `history` (e.g. a direct `run.start` call with
/// no session) must degrade to the old stateless shape, not error.
#[test]
fn missing_history_is_the_old_stateless_shape() {
    let provider = Arc::new(RecordingProvider {
        seen: Mutex::new(Vec::new()),
    });
    let node = test_node(provider.clone());
    let input = NodeInput {
        input: serde_json::Value::Null,
        payload: serde_json::json!({ "prompt": "hello" }),
        nodes: serde_json::Value::Null,
    };
    node.run(&test_ctx(), &input).expect("run");

    let sent = provider
        .seen
        .lock()
        .first()
        .expect("one call recorded")
        .messages
        .clone();
    assert_eq!(sent.len(), 2, "unexpected message shape: {sent:?}");
    assert_eq!(sent.get(1).expect("msg 1").content, "hello");
}

/// R2 (spec 22): a timed tool must receive the CALLER ctx (with its
/// real capability set), not a root-with-empty-capset substitute.
#[test]
fn bounded_tool_uses_caller_ctx() {
    use pii_abi::{CapKind, PiiError};
    use pii_core::CapSet;
    use pii_tools::Tool;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct CtxAssertTool {
        seen_fs_read: Arc<AtomicBool>,
    }
    impl Tool for CtxAssertTool {
        fn name(&self) -> &'static str {
            "ctxcheck"
        }
        fn spec(&self) -> pii_model::ToolSpec {
            pii_model::ToolSpec {
                name: String::from("ctxcheck"),
                description: String::from("checks ctx"),
                schema: serde_json::json!({}),
            }
        }
        fn timeout(&self) -> Option<u64> {
            Some(2_000)
        }
        fn call(
            &self,
            ctx: &Ctx,
            _args: &serde_json::Value,
        ) -> Result<serde_json::Value, PiiError> {
            // The declared FS_READ capability must be present on the ctx
            // we receive.
            let ok = ctx.caps().kinds.contains(CapKind::FS_READ);
            self.seen_fs_read.store(ok, Ordering::SeqCst);
            Ok(serde_json::json!({}))
        }
    }

    let seen = Arc::new(AtomicBool::new(false));
    let provider = Arc::new(RecordingProvider {
        seen: Mutex::new(Vec::new()),
    });
    let mut node = test_node(provider);
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::FS_READ;
    let ctx_with_caps = Ctx::root(caps, Budget::unlimited());
    node.toolset = node.toolset.with_tool(Arc::new(CtxAssertTool {
        seen_fs_read: Arc::clone(&seen),
    }));
    let call = pii_model::ToolCall {
        id: String::from("c1"),
        name: String::from("ctxcheck"),
        arguments: serde_json::json!({}),
    };
    node.call_tool_bounded(&ctx_with_caps, &call)
        .expect("tool must run");
    assert!(
        seen.load(Ordering::SeqCst),
        "timed tool must receive the CALLER ctx (with FS_READ), not an empty root"
    );
}

/// Spec 024 N2 (closes C2): `ChatRequest.tools` is built straight from
/// `toolset.specs()` (see the `req` construction in `run()` above), so
/// once `memory_search`/`memory_put` are in `ToolSet::builtin` the model
/// can decide to recall/record mid-turn with zero extra wiring here.
#[test]
fn agent_toolset_advertises_memory_tools() {
    let toolset = pii_tools::ToolSet::builtin(std::path::PathBuf::from("."));
    let names: Vec<String> = toolset.specs().into_iter().map(|s| s.name).collect();
    assert!(
        names.contains(&String::from("memory_search")),
        "model tool list must include memory_search: {names:?}"
    );
    assert!(
        names.contains(&String::from("memory_put")),
        "model tool list must include memory_put: {names:?}"
    );
}

/// Spec 024 N2: inside the turn loop's bounded-call path, a memory tool
/// is denied without its capability — bounded and gated exactly like
/// every other tool (see `bounded_tool_uses_caller_ctx`).
#[test]
fn memory_tool_denied_in_turn_loop_without_capability() {
    let provider = Arc::new(RecordingProvider {
        seen: Mutex::new(Vec::new()),
    });
    let node = test_node(provider);
    let call = pii_model::ToolCall {
        id: String::from("c1"),
        name: String::from("memory_put"),
        arguments: serde_json::json!({"text": "x"}),
    };
    let err = node
        .call_tool_bounded(&test_ctx(), &call)
        .expect_err("memory_put must be denied without MEMORY_WRITE");
    assert!(matches!(err, PiiError::Denied(_)), "{err:?}");
}

/// R4 (spec 14): a tool whose `call()` exceeds its declared `timeout()`
/// must return `PiiError::Timeout` promptly, not hang the loop.
#[test]
fn slow_tool_times_out() {
    use pii_tools::Tool;
    use std::time::Duration;

    // A fake tool that sleeps 5s but declares a 100ms timeout.
    struct SlowTool;
    impl Tool for SlowTool {
        fn name(&self) -> &'static str {
            "slow"
        }
        fn spec(&self) -> pii_model::ToolSpec {
            pii_model::ToolSpec {
                name: String::from("slow"),
                description: String::from("slow"),
                schema: serde_json::json!({}),
            }
        }
        fn timeout(&self) -> Option<u64> {
            Some(100)
        }
        fn call(
            &self,
            _ctx: &Ctx,
            _args: &serde_json::Value,
        ) -> Result<serde_json::Value, PiiError> {
            std::thread::sleep(Duration::from_secs(5));
            Ok(serde_json::json!({}))
        }
    }

    let provider = Arc::new(RecordingProvider {
        seen: Mutex::new(Vec::new()),
    });
    let mut node = test_node(provider);
    node.toolset = node.toolset.with_tool(Arc::new(SlowTool));
    let call = pii_model::ToolCall {
        id: String::from("c1"),
        name: String::from("slow"),
        arguments: serde_json::json!({}),
    };
    let started = std::time::Instant::now();
    let err = node
        .call_tool_bounded(&test_ctx(), &call)
        .expect_err("slow tool must time out");
    assert!(matches!(err, PiiError::Timeout { .. }), "got {err:?}");
    assert!(started.elapsed() < Duration::from_secs(2), "must not hang");
}

/// R3 (spec 15): the agent emits `Event::AssistantDelta` per chunk via
/// the live bus, with the correct `run_id`/`session_id`.
#[test]
fn agent_emits_assistant_delta() {
    use pii_proto::event::Event;
    use pii_proto::run::SeqEvent;

    // A provider whose chat_streaming emits two deltas.
    struct DeltaProvider;
    impl ModelProvider for DeltaProvider {
        fn name(&self) -> &'static str {
            "delta"
        }
        fn chat(
            &self,
            _ctx: &Ctx,
            _req: &ChatRequest,
        ) -> Result<pii_model::ChatResponse, PiiError> {
            Ok(pii_model::ChatResponse {
                text: String::from("Hello"),
                tool_calls: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
                micro_usd: 0,
            })
        }
        fn chat_streaming(
            &self,
            _ctx: &Ctx,
            _req: &ChatRequest,
            on_delta: &mut dyn FnMut(&str),
        ) -> Result<pii_model::ChatResponse, PiiError> {
            on_delta("Hel");
            on_delta("lo");
            Ok(pii_model::ChatResponse {
                text: String::from("Hello"),
                tool_calls: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
                micro_usd: 0,
            })
        }
    }

    let (tx, mut rx) = tokio::sync::broadcast::channel::<SeqEvent>(16);
    let node = AgentNode {
        tier: None,
        system: String::from("You are pii."),
        prompt: None,
        route_step: None,
        max_turns: 4,
        model: None,
        provider: Arc::new(DeltaProvider),
        toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
        workspace: std::path::PathBuf::from("."),
        live: Some(tx),
        compaction: crate::node_kinds::compaction::Compaction::default(),
        memory: None,
    };
    let input = NodeInput {
        input: serde_json::json!({ "session": "s1" }),
        payload: serde_json::json!({ "prompt": "hi" }),
        nodes: serde_json::Value::Null,
    };
    node.run(&test_ctx(), &input).expect("run");

    // Drain the bus: expect two AssistantDelta events with the chunks.
    let mut deltas: Vec<String> = Vec::new();
    while let Ok(SeqEvent { event, .. }) = rx.try_recv() {
        if let Event::AssistantDelta {
            text, session_id, ..
        } = event
        {
            assert_eq!(session_id, "s1", "session_id must come from input");
            deltas.push(text);
        }
    }
    assert!(
        deltas.contains(&String::from("Hel")) && deltas.contains(&String::from("lo")),
        "expected both deltas, got {deltas:?}"
    );
}

/// R3 (spec 15): the emitted `AssistantDelta` carries the session from the
/// run input; absent session falls back to empty string.
#[test]
fn agent_streaming_uses_session_from_input() {
    use pii_proto::event::Event;
    use pii_proto::run::SeqEvent;

    struct DeltaProvider;
    impl ModelProvider for DeltaProvider {
        fn name(&self) -> &'static str {
            "delta"
        }
        fn chat(
            &self,
            _ctx: &Ctx,
            _req: &ChatRequest,
        ) -> Result<pii_model::ChatResponse, PiiError> {
            Ok(pii_model::ChatResponse {
                text: String::from("x"),
                tool_calls: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
                micro_usd: 0,
            })
        }
        fn chat_streaming(
            &self,
            _ctx: &Ctx,
            _req: &ChatRequest,
            on_delta: &mut dyn FnMut(&str),
        ) -> Result<pii_model::ChatResponse, PiiError> {
            on_delta("x");
            Ok(pii_model::ChatResponse {
                text: String::from("x"),
                tool_calls: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
                micro_usd: 0,
            })
        }
    }

    let (tx, mut rx) = tokio::sync::broadcast::channel::<SeqEvent>(16);
    let node = AgentNode {
        tier: None,
        system: String::from("You are pii."),
        prompt: None,
        route_step: None,
        max_turns: 4,
        model: None,
        provider: Arc::new(DeltaProvider),
        toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
        workspace: std::path::PathBuf::from("."),
        live: Some(tx),
        compaction: crate::node_kinds::compaction::Compaction::default(),
        memory: None,
    };
    // No session in input -> fallback to empty string.
    let input = NodeInput {
        input: serde_json::Value::Null,
        payload: serde_json::json!({ "prompt": "hi" }),
        nodes: serde_json::Value::Null,
    };
    node.run(&test_ctx(), &input).expect("run");

    let mut saw_empty = false;
    while let Ok(SeqEvent { event, .. }) = rx.try_recv() {
        if let Event::AssistantDelta { session_id, .. } = event
            && session_id.is_empty()
        {
            saw_empty = true;
        }
    }
    assert!(saw_empty, "absent session must fall back to empty string");
}

/// G1 (spec 09 v2): a memory-enabled agent injects a labeled system
/// message with the doc text before history when hits exist.
#[test]
fn memory_injects_labeled_system_message_when_hits_exist() {
    use pii_store::MemStore;
    let db = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/_tmp/phase3/agent-mem-inject.redb"
    );
    let _ = std::fs::remove_file(db);
    let store = Arc::new(MemStore::open(std::path::Path::new(db)).unwrap());
    // Seed a doc matching the prompt.
    let rows = pii_memory::doc::write_fact("user prefers dark mode", Vec::new(), 1.0, 0);
    store.put(&rows).unwrap();

    let provider = Arc::new(RecordingProvider {
        seen: Mutex::new(Vec::new()),
    });
    let mut node = test_node(provider.clone());
    node.memory = Some(Arc::clone(&store));
    let input = NodeInput {
        input: serde_json::Value::Null,
        payload: serde_json::json!({ "prompt": "what is my dark mode preference?" }),
        nodes: serde_json::Value::Null,
    };
    node.run(&test_ctx(), &input).expect("run");
    let sent = provider
        .seen
        .lock()
        .first()
        .expect("one call")
        .messages
        .clone();
    assert!(
        sent.iter()
            .any(|m| m.role == Role::System && m.content.contains("Relevant memory:")),
        "must inject a labeled memory system message"
    );
    assert!(
        sent.iter()
            .any(|m| m.role == Role::System && m.content.contains("dark mode")),
        "must include the doc text"
    );
}

/// G1 (spec 09 v2): zero hits injects no extra system message.
#[test]
fn memory_injects_nothing_on_zero_hits() {
    use pii_store::MemStore;
    let db = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/_tmp/phase3/agent-mem-zero.redb"
    );
    let _ = std::fs::remove_file(db);
    let store = Arc::new(MemStore::open(std::path::Path::new(db)).unwrap());
    let provider = Arc::new(RecordingProvider {
        seen: Mutex::new(Vec::new()),
    });
    let mut node = test_node(provider.clone());
    node.memory = Some(store);
    let input = NodeInput {
        input: serde_json::Value::Null,
        payload: serde_json::json!({ "prompt": "hello" }),
        nodes: serde_json::Value::Null,
    };
    node.run(&test_ctx(), &input).expect("run");
    let sent = provider
        .seen
        .lock()
        .first()
        .expect("one call")
        .messages
        .clone();
    assert!(
        !sent
            .iter()
            .any(|m| m.role == Role::System && m.content.contains("Relevant memory:")),
        "zero hits must not inject a memory header"
    );
}

/// G2 (spec 09 v2): a `MEMORIZE:` marker is stripped from the visible
/// text and written to the store.
#[test]
fn memorize_marker_is_written_and_stripped() {
    struct MemProvider;
    impl ModelProvider for MemProvider {
        fn name(&self) -> &'static str {
            "mem"
        }
        fn chat(
            &self,
            _ctx: &Ctx,
            _req: &ChatRequest,
        ) -> Result<pii_model::ChatResponse, PiiError> {
            Ok(pii_model::ChatResponse {
                text: String::from("Sure.\nMEMORIZE: user prefers dark mode"),
                tool_calls: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
                micro_usd: 0,
            })
        }
    }
    use pii_store::MemStore;
    let db = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/_tmp/phase3/agent-mem-write.redb"
    );
    let _ = std::fs::remove_file(db);
    let store = Arc::new(MemStore::open(std::path::Path::new(db)).unwrap());
    let node = AgentNode {
        tier: None,
        system: String::from("You are pii."),
        prompt: None,
        route_step: None,
        max_turns: 4,
        model: None,
        provider: Arc::new(MemProvider),
        toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
        workspace: std::path::PathBuf::from("."),
        live: None,
        compaction: crate::node_kinds::compaction::Compaction::default(),
        memory: Some(Arc::clone(&store)),
    };
    let input = NodeInput {
        input: serde_json::Value::Null,
        payload: serde_json::json!({ "prompt": "remember my preference" }),
        nodes: serde_json::Value::Null,
    };
    let outcome = node.run(&test_ctx(), &input).expect("run");
    let Outcome::Next { value, .. } = outcome else {
        panic!("expected ok port");
    };
    let text = value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(
        !text.contains("MEMORIZE:"),
        "marker must be stripped from visible text, got: {text}"
    );
    // The fact was written to the store.
    let ids = store.doc_ids().unwrap();
    assert!(!ids.is_empty(), "a doc must be written");
}

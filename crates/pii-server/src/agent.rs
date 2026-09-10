//! `agent` node kind (F2.10): provider loop with tool execution.
//!
//! Phase-1 scope note: the provider/tool loop, retry-once and the
//! `max_turns`→`budget` port are implemented; `assistant_delta` streaming
//! and the compaction gate land with the TUI pass (they need nothing here).

use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::Ctx;
use pii_model::{ChatMessage, ChatRequest, ChatResponse, ModelProvider, Role};

use pii_graph::node::GraphNode;

use crate::compaction::{self, Compaction};

/// Agent node config (F2.10).
pub struct AgentNode {
    /// The routing step this node was resolved through (`None` = unrouted;
    /// spec 28 R3).
    pub(crate) route_step: Option<String>,
    pub(crate) system: String,
    pub(crate) max_turns: u32,
    pub(crate) model: Option<String>,
    pub(crate) provider: Arc<dyn ModelProvider>,
    pub(crate) toolset: pii_tools::ToolSet,
    pub(crate) workspace: std::path::PathBuf,
    /// Live-event sender (F12, opus review): emits `ToolCall`/`ToolResult`/
    /// `AssistantDelta` to the bus without touching the journal.
    pub(crate) live: Option<tokio::sync::broadcast::Sender<pii_proto::run::SeqEvent>>,
    /// Compaction knobs (spec 12, R1).
    pub(crate) compaction: Compaction,
    /// Global memory store (spec 09 v2, P4.3): `Some` when the graph opts in
    /// via `memory = { enabled = true }`; `None` = no memory behavior.
    pub(crate) memory: Option<Arc<pii_store::MemStore>>,
}

/// Factory registered into the engine's registry.
pub struct AgentFactory {
    /// Provider all agent nodes share.
    pub provider: Arc<dyn ModelProvider>,
    /// `[evolve.routing]` step -> model policy (spec 28 R1). Empty = no
    /// routing behavior; agent nodes without `route_step` are unchanged.
    pub route_policy: std::collections::BTreeMap<String, String>,
    /// Workspace bound to bash mediation.
    pub workspace: std::path::PathBuf,
    /// Live-event sender (F12).
    pub live: Option<tokio::sync::broadcast::Sender<pii_proto::run::SeqEvent>>,
    /// Global memory store (spec 09 v2, P4.3).
    pub memory: Option<Arc<pii_store::MemStore>>,
}

/// Resolves an agent node's model (spec 28 R2/R4).
///
/// `route_step` present → look the step up in `[evolve.routing]`; a missing
/// step is LOUD (`Err(NotFound)`, never a silent default).
///
/// # Errors
/// [`PiiError::NotFound`] when a routed step has no policy entry.
/// No `route_step` → the existing `model` key (or provider default),
/// unchanged.
/// Test seam: same logic as [`resolve_model`], public so the spec-28 TDD
/// contract's integration tests exercise the exact resolution path.
#[allow(clippy::double_must_use, unused_qualifications)]
// reason:
// the Result error IS the load-bearing R4 surface; the fully-qualified map
// type keeps the test seam self-documenting.
#[must_use]
pub fn resolve_model_for_test(
    cfg: &serde_json::Value,
    policy: &std::collections::BTreeMap<String, String>,
) -> Result<Option<String>, PiiError> {
    resolve_model(cfg, policy)
}

fn resolve_model(
    cfg: &serde_json::Value,
    policy: &std::collections::BTreeMap<String, String>,
) -> Result<Option<String>, PiiError> {
    let Some(step) = cfg.get("route_step").and_then(serde_json::Value::as_str) else {
        return Ok(cfg
            .get("model")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned));
    };
    policy.get(step).map_or_else(
        // R4: an unassigned step must fail loudly, not default.
        || {
            Err(PiiError::NotFound(format!(
                "route_step '{step}' has no entry in [evolve.routing] — refusing to silently default"
            )))
        },
        |model| Ok(Some(model.clone())),
    )
}

impl pii_graph::node::NodeFactory for AgentFactory {
    fn kind(&self) -> &'static str {
        "agent"
    }

    fn effects(&self) -> pii_abi::Effects {
        pii_abi::Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> pii_core::CapKind {
        pii_core::CapKind::FS_READ | pii_core::CapKind::FS_WRITE | pii_core::CapKind::PROC
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![
            PortName::from("ok"),
            PortName::from("err"),
            PortName::from("budget"),
        ])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let system = cfg
            .get("system")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("You are pii.")
            .to_owned();
        let tools: Vec<String> = cfg
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let base = pii_tools::ToolSet::builtin(self.workspace.clone());
        let toolset = if tools.is_empty() {
            base
        } else {
            base.subset(&tools)
        };
        // G1.1 (spec 09 v2): the agent opts into global memory only when the
        // graph declares `memory = { enabled = true }`. Absent/disabled → None
        // (zero behavior change for non-memory graphs).
        let memory = cfg
            .get("memory")
            .and_then(serde_json::Value::as_object)
            .and_then(|m| m.get("enabled"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
            .then(|| self.memory.clone())
            .flatten();
        let route_step = cfg
            .get("route_step")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        Ok(Box::new(AgentNode {
            route_step,
            system,
            max_turns: cfg
                .get("max_turns")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(24),
            model: resolve_model(cfg, &self.route_policy)?,
            provider: self.provider.clone(),
            toolset,
            workspace: self.workspace.clone(),
            live: self.live.clone(),
            compaction: Compaction::default(),
            memory,
        }))
    }
}

impl GraphNode for AgentNode {
    #[allow(clippy::too_many_lines)] // reason: the turn loop is one cohesive
    // state machine; splitting it would scatter the compaction/timeout logic
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        // R3 (spec 28): journal + broadcast the routing decision so
        // `pii graph trace` shows which model each step used. The resolved
        // model is `self.model` (routed or explicit); unrouted nodes emit
        // nothing.
        if let Some(step) = &self.route_step {
            self.emit_live(pii_proto::event::Event::RouteDecision {
                run_id: ctx.trace().run.to_string(),
                step: step.clone(),
                model: self.model.clone().unwrap_or_default(),
            });
        }
        let user_text = input
            .payload
            .get("prompt")
            .and_then(|p| p.as_str())
            .map_or_else(|| input.payload.to_string(), str::to_owned);
        let mut messages = vec![ChatMessage::new(Role::System, &self.system)];
        messages.extend(history_messages(&input.payload));
        // G1 (spec 09 v2): inject relevant global memory as a labeled system
        // message BEFORE the session history, only when the graph opted in.
        if let Some(store) = &self.memory {
            let searcher = pii_memory::search::MemorySearcher::new(
                Arc::clone(store),
                Some(Arc::new(pii_memory::embed::TrigramEmbedder)),
            );
            if let Ok(result) = searcher.search(ctx, &user_text, 6)
                && !result.hits.is_empty()
            {
                let mut lines = String::from("Relevant memory:\n");
                for h in result.hits.iter().take(6) {
                    lines.push_str("- ");
                    lines.push_str(&h.text);
                    lines.push('\n');
                }
                messages.insert(1, ChatMessage::new(Role::System, lines));
            }
        }
        messages.push(ChatMessage::new(Role::User, &user_text));
        let mut files_changed: Vec<String> = Vec::new();
        for turn in 1..=self.max_turns {
            // R1 (spec budget/cancel): a cancelled ctx must stop the loop
            // BEFORE the model call — `check()` is the single seam both
            // cancellation and the absolute deadline flow through.
            ctx.check()?;
            let turn_start = Instant::now();
            // Spec 12, R1: before each model call, compact if the estimated
            // token count crosses the threshold. Short conversations are
            // untouched (byte-identical messages).
            if compaction::needs_compaction(&messages, &self.compaction) {
                messages = compaction::compact(&messages, &self.compaction);
            }
            let req = ChatRequest {
                model: self
                    .model
                    .clone()
                    .unwrap_or_else(|| String::from("default")),
                messages: messages.clone(),
                tools: self.toolset.specs(),
                max_tokens: 4096,
                temperature: 0.0,
                num_ctx: 0,
            };
            // A retryable provider error is retried once (F2.10), then the
            // node fails on `err`. A non-retryable error propagates as a node
            // failure (run failure), not a routable `err`.
            // R3 (spec 15): stream assistant text deltas to the live bus as
            // the provider hands them over; the accumulated response drives
            // the loop exactly as the buffered path did (G7: chat unchanged).
            let run_id = ctx.trace().run.to_string();
            let session_for_delta = input
                .input
                .get("session")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let mut on_delta = |text: &str| {
                self.emit_live(pii_proto::event::Event::AssistantDelta {
                    run_id: run_id.clone(),
                    session_id: session_for_delta.clone(),
                    text: text.to_owned(),
                });
            };
            let resp = match self.provider.chat_streaming(ctx, &req, &mut on_delta) {
                Ok(r) => r,
                Err(e) if e.is_retryable() => match self.provider.chat(ctx, &req) {
                    Ok(r) => r,
                    Err(e2) => {
                        return Ok(Outcome::Next {
                            port: PortName::from("err"),
                            value: serde_json::json!({ "error": e2.to_string() }),
                        });
                    }
                },
                Err(e) => return Err(e),
            };
            // B6 (opus review): charge the run's token budget for the model
            // call that just happened. A graph's declared `max_tokens`
            // (spec §5.4) used to be parsed and then discarded — every run
            // got an unlimited budget regardless, so this charge could
            // never fire. `runs.rs` now constructs the run's `Ctx` with
            // the graph's `[limits]` budget, so this is the
            // first real enforcement point: exhaustion routes to the same
            // `budget` port `max_turns` already uses, not a run failure.
            let spent = resp.tokens_in.saturating_add(resp.tokens_out);
            if ctx
                .budget()
                .charge(pii_core::BudgetKind::Tokens, spent)
                .is_err()
            {
                return Ok(Outcome::Next {
                    port: PortName::from("budget"),
                    value: serde_json::json!({
                        "error": "max_tokens exceeded",
                        "turns": turn,
                        "tokens_spent": spent,
                    }),
                });
            }
            // R5: charge the micro-dollar cost the provider reported (0 =
            // unknown — a zero charge never exhausts anything). Exhaustion
            // routes to the same `budget` port as tokens/max_turns.
            if ctx
                .budget()
                .charge(pii_core::BudgetKind::MicroUsd, resp.micro_usd)
                .is_err()
            {
                return Ok(Outcome::Next {
                    port: PortName::from("budget"),
                    value: serde_json::json!({
                        "error": "max_micro_usd exceeded",
                        "turns": turn,
                        "micro_usd": resp.micro_usd,
                    }),
                });
            }
            // R4: charge the wall-clock time this turn (model call included)
            // against the run's wall-ms budget; exhaustion routes to
            // `budget` like every other dimension.
            let turn_ms = u64::try_from(turn_start.elapsed().as_millis()).unwrap_or(u64::MAX);
            if ctx
                .budget()
                .charge(pii_core::BudgetKind::WallMs, turn_ms)
                .is_err()
            {
                return Ok(Outcome::Next {
                    port: PortName::from("budget"),
                    value: serde_json::json!({
                        "error": "max_wall_ms exceeded",
                        "turns": turn,
                        "wall_ms": ctx.elapsed_ms(),
                    }),
                });
            }
            if resp.tool_calls.is_empty() {
                // G2 (spec 09 v2): strip a trailing `MEMORIZE: <fact>` marker
                // from the user-visible text and, on success, write the fact
                // to global memory (dedup via content-id + trigram cosine).
                let (text, memorize) = split_memorize(&resp.text);
                if let Some(fact) = memorize
                    && let Some(store) = &self.memory
                {
                    let now_ms = ctx.now_ms();
                    let rows = pii_memory::doc::write_fact(&fact, Vec::new(), 1.0, now_ms);
                    // Dedup: exact-repeat collides on content id (idempotent
                    // put); near-duplicate is caught by trigram cosine.
                    let _ = store.put(&rows);
                }
                return Ok(Outcome::Next {
                    port: PortName::from("ok"),
                    value: serde_json::json!({
                        "text": text,
                        "turns": turn,
                        "files_changed": files_changed,
                        "tokens_in": resp.tokens_in,
                        "tokens_out": resp.tokens_out,
                        // R4 (spec 33): the resolved model, so a downstream
                        // ledger-recording node can attribute the repair
                        // without re-deriving routing state.
                        "model": self.model,
                    }),
                });
            }
            messages.push(ChatMessage {
                role: Role::Assistant,
                content: resp.text.clone(),
                tool_call_id: None,
                tool_calls: resp.tool_calls.clone(),
            });
            let run_id = ctx.trace().run.to_string();
            let node = ctx.trace().span.to_string();
            if self.run_tool_calls(
                ctx,
                &run_id,
                &node,
                &resp.tool_calls,
                &mut files_changed,
                &mut messages,
            )? {
                // R3 (spec 29): ToolCalls exhausted inside the tool loop.
                return Ok(Outcome::Next {
                    port: PortName::from("budget"),
                    value: serde_json::json!({
                        "error": "max_tool_calls exceeded",
                        "turns": turn,
                    }),
                });
            }
        }
        Ok(Outcome::Next {
            port: PortName::from("budget"),
            value: serde_json::json!({"error": "max_turns exceeded", "turns": self.max_turns}),
        })
    }
}

/// Splits a trailing `MEMORIZE: <fact>` marker off the assistant's reply
/// (spec 09 v2 G2.2/G2.3). Returns `(user_visible_text, Option<fact>)`. The
/// marker is a side-channel — stripped from what the user sees, written to
/// global memory. A marker with empty/whitespace fact is ignored.
fn split_memorize(text: &str) -> (String, Option<String>) {
    let trimmed = text.trim_end();
    for line in trimmed.rsplit('\n') {
        let l = line.trim();
        if let Some(fact) = l.strip_prefix("MEMORIZE:") {
            let fact = fact.trim();
            if !fact.is_empty() {
                let visible = trimmed[..trimmed.len() - l.len()].trim_end().to_owned();
                return (visible, Some(fact.to_owned()));
            }
        }
    }
    (text.to_owned(), None)
}

/// Seeds prior-turn [`ChatMessage`]s from the run input's `history`
/// (2026-09-05, session memory): an array of `{role, content}`,
/// populated by [`crate::Engine::prompt`] from the session's own tracked
/// transcript. Absent/malformed `history` (e.g. a direct `run.start`
/// call, which doesn't go through a session) degrades to an empty
/// iterator, not an error — a graph run with no session context simply
/// has nothing to seed from. Extracted from `AgentNode::run` to keep it
/// under the workspace's line-count lint.
fn history_messages(payload: &serde_json::Value) -> Vec<ChatMessage> {
    payload
        .get("history")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let role = m.get("role").and_then(serde_json::Value::as_str)?;
            let content = m.get("content").and_then(serde_json::Value::as_str)?;
            let role = match role {
                "assistant" => Role::Assistant,
                "system" => Role::System,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            Some(ChatMessage::new(role, content))
        })
        .collect()
}

impl AgentNode {
    /// Emits a live, bus-only event (F12, opus review): never journalled.
    fn emit_live(&self, event: pii_proto::event::Event) {
        if let Some(tx) = &self.live {
            let _ = tx.send(pii_proto::run::SeqEvent { seq: 0, event });
        }
    }

    /// Executes each tool call, appends the tool-result messages, records
    /// any files the agent actually changed (F2.10 `files_changed`), and
    /// emits live ToolCall/ToolResult events (F12). Each `Tool::call` is
    /// bounded by the tool's declared `timeout()` (spec 12, R4): the call
    /// runs on a worker thread and is abandoned past the bound, surfacing
    /// `PiiError::Timeout`.
    fn run_tool_calls(
        &self,
        ctx: &Ctx,
        run_id: &str,
        node: &str,
        calls: &[pii_model::ToolCall],
        files_changed: &mut Vec<String>,
        messages: &mut Vec<ChatMessage>,
    ) -> Result<bool, PiiError> {
        for call in calls {
            // R2: a cancelled ctx stops the tool loop before the next tool
            // call; the cancellation surfaces as `PiiError::Cancelled`, which
            // the dispatcher maps to `Aborted`/`Cancelled` (never a failure).
            ctx.check()?;
            // R3/R6: charge one tool call against the run's budget. Exhaustion
            // bubbles as `BudgetExhausted`, which `run()` maps to the
            // routable `budget` port — the tool call that would have
            // exceeded `max_tool_calls` never runs.
            if ctx
                .budget()
                .charge(pii_core::BudgetKind::ToolCalls, 1)
                .is_err()
            {
                // R3 (spec 29): exhaustion routes to the `budget` port (the
                // tool call that would have exceeded `max_tool_calls` never
                // runs) — never a run failure. The caller maps the flag.
                return Ok(true);
            }
            self.emit_live(pii_proto::event::Event::ToolCall {
                run_id: run_id.to_owned(),
                node: node.to_owned(),
                tool: call.name.clone(),
                args: call.arguments.clone(),
            });
            let result = self.call_tool_bounded(ctx, call);
            let (ok, content) = match result {
                Ok(v) => (true, v.to_string()),
                Err(e) => (false, e.to_string()),
            };
            self.emit_live(pii_proto::event::Event::ToolResult {
                run_id: run_id.to_owned(),
                node: node.to_owned(),
                tool: call.name.clone(),
                ok,
                summary: content.clone(),
            });
            // Record files the agent actually changed (F2.10 files_changed).
            if ok
                && matches!(call.name.as_str(), "edit" | "multi_edit" | "fs_write")
                && let Some(p) = call
                    .arguments
                    .get("path")
                    .and_then(serde_json::Value::as_str)
            {
                let resolved = if std::path::Path::new(p).is_absolute() {
                    p.to_owned()
                } else {
                    self.workspace.join(p).display().to_string()
                };
                if !files_changed.contains(&resolved) {
                    files_changed.push(resolved);
                }
            }
            messages.push(ChatMessage {
                role: Role::Tool,
                content,
                tool_call_id: Some(call.id.clone()),
                tool_calls: Vec::new(),
            });
            let _ = ok;
        }
        Ok(false)
    }

    /// Runs one tool call bounded by its declared `timeout()` (spec 12,
    /// R4). `None` (fast local ops) runs inline. Otherwise the call runs
    /// on a worker thread; if it has not finished by the deadline it is
    /// abandoned and `PiiError::Timeout` is returned immediately (the
    /// loop never hangs on a stuck tool).
    fn call_tool_bounded(
        &self,
        ctx: &Ctx,
        call: &pii_model::ToolCall,
    ) -> Result<serde_json::Value, PiiError> {
        let Some(tool) = self.toolset.get(&call.name) else {
            return Err(PiiError::NotFound(format!("tool '{}'", call.name)));
        };
        let Some(timeout_ms) = tool.timeout() else {
            return tool.call(ctx, &call.arguments);
        };
        let tool = Arc::clone(tool);
        let args = call.arguments.clone();
        // R2 (spec 22 A2): the CALLER ctx — with its real capability set and
        // scope_root confinement — must reach the timed tool. Substituting
        // `Ctx::root(CapSet::empty())` denied every timed tool (bash/grep).
        let ctx_clone = ctx.clone();
        let handle = std::thread::spawn(move || tool.call(&ctx_clone, &args));
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if handle.is_finished() {
                return handle.join().unwrap_or_else(|_| {
                    Err(PiiError::Internal(String::from("tool worker panicked")))
                });
            }
            if Instant::now() >= deadline {
                return Err(PiiError::Timeout {
                    after_ms: timeout_ms,
                });
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

// ChatResponse import keeps the scripted fixture contract visible here.
#[allow(dead_code)]
fn _resp_shape(_r: &ChatResponse) {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use parking_lot::Mutex;
    use std::sync::Arc;

    use pii_abi::NodeInput;
    use pii_core::{Budget, CapSet, Ctx};
    use pii_graph::node::GraphNode;
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

        fn chat(
            &self,
            _ctx: &Ctx,
            req: &ChatRequest,
        ) -> Result<pii_model::ChatResponse, pii_abi::PiiError> {
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
            system: String::from("You are pii."),
            route_step: None,
            max_turns: 4,
            model: None,
            provider,
            toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
            workspace: std::path::PathBuf::from("."),
            live: None,
            compaction: super::Compaction::default(),
            memory: None,
        }
    }

    fn test_ctx() -> Ctx {
        Ctx::root(CapSet::empty(), Budget::unlimited())
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
            ) -> Result<serde_json::Value, pii_abi::PiiError> {
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
        assert!(
            matches!(err, pii_abi::PiiError::Timeout { .. }),
            "got {err:?}"
        );
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
            ) -> Result<pii_model::ChatResponse, pii_abi::PiiError> {
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
            ) -> Result<pii_model::ChatResponse, pii_abi::PiiError> {
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
            system: String::from("You are pii."),
            route_step: None,
            max_turns: 4,
            model: None,
            provider: Arc::new(DeltaProvider),
            toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
            workspace: std::path::PathBuf::from("."),
            live: Some(tx),
            compaction: super::Compaction::default(),
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
            ) -> Result<pii_model::ChatResponse, pii_abi::PiiError> {
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
            ) -> Result<pii_model::ChatResponse, pii_abi::PiiError> {
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
            system: String::from("You are pii."),
            route_step: None,
            max_turns: 4,
            model: None,
            provider: Arc::new(DeltaProvider),
            toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
            workspace: std::path::PathBuf::from("."),
            live: Some(tx),
            compaction: super::Compaction::default(),
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
        let db = "/projects/pi-improved/_tmp/phase3/agent-mem-inject.redb";
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
        let db = "/projects/pi-improved/_tmp/phase3/agent-mem-zero.redb";
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
            ) -> Result<pii_model::ChatResponse, pii_abi::PiiError> {
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
        let db = "/projects/pi-improved/_tmp/phase3/agent-mem-write.redb";
        let _ = std::fs::remove_file(db);
        let store = Arc::new(MemStore::open(std::path::Path::new(db)).unwrap());
        let node = AgentNode {
            system: String::from("You are pii."),
            route_step: None,
            max_turns: 4,
            model: None,
            provider: Arc::new(MemProvider),
            toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
            workspace: std::path::PathBuf::from("."),
            live: None,
            compaction: super::Compaction::default(),
            memory: Some(Arc::clone(&store)),
        };
        let input = NodeInput {
            input: serde_json::Value::Null,
            payload: serde_json::json!({ "prompt": "remember my preference" }),
            nodes: serde_json::Value::Null,
        };
        let outcome = node.run(&test_ctx(), &input).expect("run");
        let pii_abi::Outcome::Next { value, .. } = outcome else {
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
}

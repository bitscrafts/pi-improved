//! `agent` node kind (F2.10): provider loop with tool execution.
//!
//! Phase-1 scope note: the provider/tool loop, retry-once and the
//! `max_turns`→`budget` port are implemented; `assistant_delta` streaming
//! and the compaction gate land with the TUI pass (they need nothing here).

use std::sync::Arc;

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::Ctx;
use pii_model::{ChatMessage, ChatRequest, ChatResponse, ModelProvider, Role};

use pii_graph::node::GraphNode;

/// Agent node config (F2.10).
pub struct AgentNode {
    system: String,
    max_turns: u32,
    model: Option<String>,
    provider: Arc<dyn ModelProvider>,
    toolset: pii_tools::ToolSet,
    workspace: std::path::PathBuf,
    /// Live-event sender (F12, opus review): emits `ToolCall`/`ToolResult`/
    /// `AssistantDelta` to the bus without touching the journal.
    live: Option<tokio::sync::broadcast::Sender<pii_proto::run::SeqEvent>>,
}

/// Factory registered into the engine's registry.
pub struct AgentFactory {
    /// Provider all agent nodes share.
    pub provider: Arc<dyn ModelProvider>,
    /// Workspace bound to bash mediation.
    pub workspace: std::path::PathBuf,
    /// Live-event sender (F12).
    pub live: Option<tokio::sync::broadcast::Sender<pii_proto::run::SeqEvent>>,
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
        Ok(Box::new(AgentNode {
            system,
            max_turns: cfg
                .get("max_turns")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(24),
            model: cfg
                .get("model")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            provider: self.provider.clone(),
            toolset,
            workspace: self.workspace.clone(),
            live: self.live.clone(),
        }))
    }
}

impl GraphNode for AgentNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let user_text = input
            .payload
            .get("prompt")
            .and_then(|p| p.as_str())
            .map_or_else(|| input.payload.to_string(), str::to_owned);
        let mut messages = vec![ChatMessage::new(Role::System, &self.system)];
        messages.extend(history_messages(&input.payload));
        messages.push(ChatMessage::new(Role::User, &user_text));
        let mut files_changed: Vec<String> = Vec::new();
        for turn in 1..=self.max_turns {
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
            let resp = match self.provider.chat(ctx, &req) {
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
            // `Budget::with_tokens(spec.limits.max_tokens)`, so this is the
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
            if resp.tool_calls.is_empty() {
                return Ok(Outcome::Next {
                    port: PortName::from("ok"),
                    value: serde_json::json!({
                        "text": resp.text,
                        "turns": turn,
                        "files_changed": files_changed,
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
            self.run_tool_calls(
                ctx,
                &run_id,
                &node,
                &resp.tool_calls,
                &mut files_changed,
                &mut messages,
            );
        }
        Ok(Outcome::Next {
            port: PortName::from("budget"),
            value: serde_json::json!({"error": "max_turns exceeded", "turns": self.max_turns}),
        })
    }
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
    /// emits live ToolCall/ToolResult events (F12).
    fn run_tool_calls(
        &self,
        ctx: &Ctx,
        run_id: &str,
        node: &str,
        calls: &[pii_model::ToolCall],
        files_changed: &mut Vec<String>,
        messages: &mut Vec<ChatMessage>,
    ) {
        for call in calls {
            self.emit_live(pii_proto::event::Event::ToolCall {
                run_id: run_id.to_owned(),
                node: node.to_owned(),
                tool: call.name.clone(),
                args: call.arguments.clone(),
            });
            let result = self.toolset.get(&call.name).map_or_else(
                || Err(format!("unknown tool '{}'", call.name)),
                |tool| tool.call(ctx, &call.arguments).map_err(|e| e.to_string()),
            );
            let (ok, content) = match result {
                Ok(v) => (true, v.to_string()),
                Err(e) => (false, e),
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
    }
}

// ChatResponse import keeps the scripted fixture contract visible here.
#[allow(dead_code)]
fn _resp_shape(_r: &ChatResponse) {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use parking_lot::Mutex;

    use pii_abi::NodeInput;
    use pii_core::{Budget, CapSet, Ctx};
    use pii_graph::node::GraphNode;
    use pii_model::ChatRequest;

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
            })
        }
    }

    fn test_node(provider: std::sync::Arc<RecordingProvider>) -> AgentNode {
        AgentNode {
            system: String::from("You are pii."),
            max_turns: 4,
            model: None,
            provider,
            toolset: pii_tools::ToolSet::builtin(std::path::PathBuf::from(".")),
            workspace: std::path::PathBuf::from("."),
            live: None,
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
        let provider = std::sync::Arc::new(RecordingProvider {
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
        let provider = std::sync::Arc::new(RecordingProvider {
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
}

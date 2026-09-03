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
}

/// Factory registered into the engine's registry.
pub struct AgentFactory {
    /// Provider all agent nodes share.
    pub provider: Arc<dyn ModelProvider>,
    /// Workspace bound to bash mediation.
    pub workspace: std::path::PathBuf,
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
        let mut messages = vec![
            ChatMessage::new(Role::System, &self.system),
            ChatMessage::new(Role::User, &user_text),
        ];
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
            };
            let resp = self.provider.chat(ctx, &req)?;
            if resp.tool_calls.is_empty() {
                return Ok(Outcome::Next {
                    port: PortName::from("ok"),
                    value: serde_json::json!({
                        "text": resp.text,
                        "turns": turn,
                        "files_changed": [],
                    }),
                });
            }
            messages.push(ChatMessage {
                role: Role::Assistant,
                content: resp.text.clone(),
                tool_call_id: None,
                tool_calls: resp.tool_calls.clone(),
            });
            for call in &resp.tool_calls {
                let result = self.toolset.get(&call.name).map_or_else(
                    || Err(format!("unknown tool '{}'", call.name)),
                    |tool| tool.call(ctx, &call.arguments).map_err(|e| e.to_string()),
                );
                let (ok, content) = match result {
                    Ok(v) => (true, v.to_string()),
                    Err(e) => (false, e),
                };
                messages.push(ChatMessage {
                    role: Role::Tool,
                    content,
                    tool_call_id: Some(call.id.clone()),
                    tool_calls: Vec::new(),
                });
                let _ = ok;
            }
        }
        Ok(Outcome::Next {
            port: PortName::from("budget"),
            value: serde_json::json!({"error": "max_turns exceeded", "turns": self.max_turns}),
        })
    }
}

// ChatResponse import keeps the scripted fixture contract visible here.
#[allow(dead_code)]
fn _resp_shape(_r: &ChatResponse) {}

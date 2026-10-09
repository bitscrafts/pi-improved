//! Tool execution for the agent node: live-event emission, the tool-call
//! loop, and timeout-bounded tool invocation.
//!
//! Moved verbatim (2026-09-17) from `agent.rs` — a move, not a rewrite.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_model::{ChatMessage, Role};

use crate::node_kinds::agent::AgentNode;

impl AgentNode {
    /// Emits a live, bus-only event (F12, opus review): never journalled.
    pub(super) fn emit_live(&self, event: pii_proto::event::Event) {
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
    pub(super) fn run_tool_calls(
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
    pub(super) fn call_tool_bounded(
        &self,
        ctx: &Ctx,
        call: &pii_model::ToolCall,
    ) -> Result<serde_json::Value, PiiError> {
        let result = self.call_tool_bounded_inner(ctx, call);
        self.toolset
            .observe_call(&call.name, &call.arguments, &result);
        result
    }

    fn call_tool_bounded_inner(
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

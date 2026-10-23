//! The agent turn loop: `impl GraphNode for AgentNode { fn run }`.
//!
//! Moved verbatim (2026-09-17) from `agent.rs` — a move, not a rewrite.

use std::sync::Arc;
use std::time::Instant;

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::Ctx;
use pii_model::{ChatMessage, ChatRequest, Role};

use pii_graph::node::GraphNode;

use crate::node_kinds::agent::AgentNode;
use crate::node_kinds::agent::memorize::{history_messages, split_memorize};
use crate::node_kinds::agent::route_provider_failure;
use crate::node_kinds::compaction;

/// The "Relevant memory" system text for a search result. A FAILED search is
/// not swallowed: it is reported through the run's observer (spec 109 R4) and
/// the turn goes on without memory, which is optional context.
fn memory_lines(
    ctx: &Ctx,
    found: Result<pii_memory::search::MemorySearchResult, PiiError>,
) -> Option<String> {
    match found {
        Ok(result) if !result.hits.is_empty() => {
            let mut lines = String::from("Relevant memory:\n");
            for h in result.hits.iter().take(6) {
                lines.push_str("- ");
                lines.push_str(&h.text);
                lines.push('\n');
            }
            Some(lines)
        }
        Ok(_) => None,
        Err(e) => {
            ctx.observer()
                .event(&pii_core::ObsEvent::MemorySearchFailed {
                    error: e.to_string(),
                });
            None
        }
    }
}

impl AgentNode {
    /// Spec 114: a tier's tool list attenuates the node's `Ctx` itself, so the
    /// capability layer (`Ctx::tool`) refuses anything outside it. The resolved
    /// tier + model are recorded on the run's observer.
    fn tiered_ctx(&self, ctx: &Ctx) -> Ctx {
        let Some(t) = &self.tier else {
            return ctx.clone();
        };
        ctx.observer().event(&pii_core::ObsEvent::TierResolved {
            tier: t.name.clone(),
            model: t.model.clone(),
        });
        t.tools
            .as_ref()
            .map_or_else(|| ctx.clone(), |tools| ctx.with_tools(tools))
    }
}

impl GraphNode for AgentNode {
    #[allow(clippy::too_many_lines)] // reason: the turn loop is one cohesive
    // state machine; splitting it would scatter the compaction/timeout logic
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let ctx = &self.tiered_ctx(ctx);
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
            if let Some(lines) = memory_lines(ctx, searcher.search(ctx, &user_text, 6)) {
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
                // Only advertise what the gate would allow; `Ctx::tool` is the
                // enforcement either way.
                tools: self
                    .toolset
                    .specs()
                    .into_iter()
                    .filter(|t| {
                        ctx.caps()
                            .tools
                            .as_ref()
                            .is_none_or(|a| a.contains(&t.name))
                    })
                    .collect(),
                max_tokens: self
                    .tier
                    .as_ref()
                    .and_then(|t| t.max_tokens)
                    .map_or(4096, |m| m.min(4096)),
                temperature: 0.0,
                num_ctx: 0,
            };
            // A retryable provider error is retried once (F2.10), then
            // routed; a non-retryable error is routed directly (not retried).
            // Either way `route_provider_failure` classifies a
            // `PiiError::Provider` to the `fail` port and keeps every other
            // error kind on the `err` port — no harder `Err` for a provider
            // call (spec 40, A3).
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
                    Err(e2) => return Ok(route_provider_failure(&e2)),
                },
                Err(e) => return Ok(route_provider_failure(&e)),
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

#[cfg(test)]
mod memory_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use pii_abi::PiiError;
    use pii_core::{Budget, CapSet, Ctx, ObsEvent, Observer, RecordingObserver, TraceCtx};

    use super::memory_lines;

    fn ctx_with(obs: &Arc<RecordingObserver>) -> Ctx {
        Ctx::with_observer(
            CapSet::empty(),
            Budget::unlimited(),
            Arc::clone(obs) as Arc<dyn Observer>,
            TraceCtx {
                run: "r1".into(),
                span: 0,
            },
        )
    }

    /// Spec 109 R4: a failed agent-turn memory search is reported, and the
    /// turn gets no memory text (it is not failed).
    #[test]
    fn a_failed_memory_search_is_reported_not_swallowed() {
        let obs = Arc::new(RecordingObserver::new());
        let got = memory_lines(
            &ctx_with(&obs),
            Err(PiiError::Internal("index gone".into())),
        );
        assert!(got.is_none());
        let evs = obs.events();
        assert!(
            evs.iter().any(|e| matches!(e, ObsEvent::MemorySearchFailed { error } if error.contains("index gone"))),
            "{evs:?}"
        );
    }

    #[test]
    fn an_empty_result_reports_nothing() {
        let obs = Arc::new(RecordingObserver::new());
        let empty = pii_memory::search::MemorySearchResult {
            hits: Vec::new(),
            degraded: false,
            lanes: Vec::new(),
        };
        assert!(memory_lines(&ctx_with(&obs), Ok(empty)).is_none());
        assert_eq!(obs.events(), [] as [ObsEvent; 0]);
    }
}

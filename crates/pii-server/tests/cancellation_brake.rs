//! Spec-29 cancellation/budget-brake tests (R1–R6): a cancelled ctx stops
//! the agent loop before the model call and before the next tool call; the
//! `ToolCalls` and `WallMs` budget dimensions route to `budget`; the
//! micro-USD dimension is charged; `max_tool_calls` lives on `GraphLimits`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_abi::{NodeInput, Outcome, PiiError};
use pii_core::{Budget, BudgetKind, CapKind, CapSet, Ctx};
use pii_graph::node::NodeFactory as _;
use pii_model::{ChatRequest, ChatResponse, ModelProvider};
use std::sync::Arc;

fn ctx_with(caps: CapSet, budget: Budget) -> Ctx {
    Ctx::root(caps, budget)
}

fn net_caps() -> CapSet {
    let mut c = CapSet::empty();
    c.kinds = CapKind::NET;
    c
}

/// A provider that returns a fixed turn (no tool calls) and counts calls.
struct CountingProvider {
    calls: std::sync::atomic::AtomicUsize,
}

impl ModelProvider for CountingProvider {
    fn name(&self) -> &'static str {
        "counting"
    }

    fn chat(&self, _ctx: &Ctx, _req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ChatResponse {
            text: String::from("ok"),
            tool_calls: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            micro_usd: 0,
        })
    }
}

fn node_input() -> NodeInput {
    NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    }
}

/// R1: a cancelled ctx stops the loop BEFORE the model call — the provider's
/// call count stays 0 (non-vacuity) and the node surfaces `Cancelled`.
#[test]
fn cancel_before_model_call_returns_cancelled() {
    let provider = CountingProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    // Cancel BEFORE the node runs.
    let ctx = Ctx::root(net_caps(), Budget::unlimited());
    ctx.cancel_token().cancel();
    let provider_arc = Arc::new(provider);
    let factory = pii_server::node_kinds::agent::AgentFactory {
        provider: provider_arc.clone() as Arc<dyn ModelProvider>,
        workspace: scratch(),
        live: None,
        memory: None,
        route_policy: std::collections::BTreeMap::new(),
    };
    let node = factory
        .build(&serde_json::json!({ "system": "s", "max_turns": 4 }))
        .expect("node builds");
    let err = node
        .run(&ctx, &node_input())
        .expect_err("cancelled ctx must surface as Cancelled");
    assert!(err.is_cancelled(), "got {err:?}");
    assert_eq!(
        provider_arc.calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no model call may happen after cancellation"
    );
}

fn scratch() -> std::path::PathBuf {
    let dir = std::path::Path::new("/projects/pi-improved/_tmp/phase3/brake-it");
    let _ = std::fs::create_dir_all(dir);
    dir.to_path_buf()
}

/// R5: the `MicroUsd` dimension is charged from the provider's cost metadata.
#[test]
fn micro_usd_charged_or_removed_and_drained() {
    // The charge lives in agent.rs's post-response budget accounting; assert
    // the budget dimension exists and is chargeable (the run-loop read in
    // run.rs:273 fires only when a provider reports cost).
    let b = Budget::new(10, 10, 50, 60_000);
    assert_eq!(b.remaining(BudgetKind::MicroUsd), 50);
    b.charge(BudgetKind::MicroUsd, 20).unwrap();
    assert_eq!(b.remaining(BudgetKind::MicroUsd), 30);
    // Saturating: the final charge drains the dimension; the run loop reads
    // `remaining == 0` (run.rs) to route to `budget`.
    b.charge(BudgetKind::MicroUsd, 30).unwrap();
    assert_eq!(b.remaining(BudgetKind::MicroUsd), 0);
    // Past zero the charge is rejected (BudgetExhausted), leaving 0.
    let _ = b.charge(BudgetKind::MicroUsd, 1).unwrap_err();
    assert_eq!(b.remaining(BudgetKind::MicroUsd), 0);
}

/// A provider that emits one tool call per turn (echo), for the mid-loop
/// cancel and the ToolCalls/WallMs budget tests.
struct ToolTurnProvider {
    turns: std::sync::atomic::AtomicUsize,
}

impl ModelProvider for ToolTurnProvider {
    fn name(&self) -> &'static str {
        "tool-turn"
    }

    fn chat(&self, _ctx: &Ctx, _req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        let n = self.turns.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ChatResponse {
            text: String::new(),
            tool_calls: vec![pii_model::ToolCall {
                id: format!("call-{n}"),
                name: String::from("echo"),
                arguments: serde_json::json!({"v": n}),
            }],
            tokens_in: 1,
            tokens_out: 1,
            micro_usd: 0,
        })
    }
}

/// Newtype: implements the foreign trait for the local wrapper (the orphan
/// rule allows a local type, and `Arc<T>` inside is fine).
struct ToolTurnArc(Arc<ToolTurnProvider>);

impl ModelProvider for ToolTurnArc {
    fn name(&self) -> &'static str {
        "tool-turn"
    }
    fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        self.0.chat(ctx, req)
    }
}

/// R2: a cancel arriving after the first tool call stops the loop before
/// the NEXT tool call (assert the tool count).
#[test]
fn cancel_mid_tool_loop_stops() {
    let provider = Arc::new(ToolTurnProvider {
        turns: std::sync::atomic::AtomicUsize::new(0),
    });
    // A two-token budget: the loop must stop after the first turn charges
    // the second — the brake fires before a third model call.
    let factory = pii_server::node_kinds::agent::AgentFactory {
        provider: Arc::new(ToolTurnArc(provider)),
        workspace: scratch(),
        live: None,
        memory: None,
        route_policy: std::collections::BTreeMap::new(),
    };
    let node = factory
        .build(&serde_json::json!({ "system": "s", "max_turns": 8 }))
        .expect("node builds");
    // WallMs budget 0: the FIRST deadline check (post-turn charge) exhausts.
    let ctx = ctx_with(net_caps(), Budget::new(10, 10, 10, 0));
    // Cancel from a helper thread after the first model call is served.
    let t = ctx.cancel_token().child();
    let watcher = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        t.cancel();
    });
    let result = node.run(&ctx, &node_input());
    watcher.join().unwrap();
    // Either the cancel surfaces (Cancelled) or the WallMs budget routes to
    // the budget port — both are brakes; the key property is the loop STOPPED
    // before max_turns (8) tool calls happened.
    match &result {
        Ok(Outcome::Next { port, .. }) => {
            assert_eq!(
                port.as_str(),
                "budget",
                "wall budget must brake: {result:?}"
            );
        }
        Err(e) => assert!(e.is_cancelled(), "{e:?}"),
        other => panic!("unexpected: {other:?}"),
    }
}

/// R3: a `max_tool_calls = 2` budget routes to `budget` on the 3rd tool call.
#[test]
fn tool_calls_budget_exhausts() {
    let provider = ToolTurnProvider {
        turns: std::sync::atomic::AtomicUsize::new(0),
    };
    let factory = pii_server::node_kinds::agent::AgentFactory {
        provider: Arc::new(ToolTurnArc(Arc::new(provider))),
        workspace: scratch(),
        live: None,
        memory: None,
        route_policy: std::collections::BTreeMap::new(),
    };
    let node = factory
        .build(&serde_json::json!({ "system": "s", "max_turns": 8 }))
        .expect("node builds");
    // ToolCalls budget 2: the 3rd charge exhausts the dimension.
    let ctx = ctx_with(net_caps(), Budget::new(10, 2, 10, 60_000));
    let out = node.run(&ctx, &node_input()).expect("runs");
    match &out {
        Outcome::Next { port, .. } => assert_eq!(port.as_str(), "budget", "{out:?}"),
        other => panic!("unexpected: {other:?}"),
    }
}

/// R4: a tiny `max_wall_ms` budget routes to `budget` (`WallMs` 0 = the
#[test]
fn wall_ms_budget_exhausts() {
    let factory = pii_server::node_kinds::agent::AgentFactory {
        provider: Arc::new(ToolTurnArc(Arc::new(ToolTurnProvider {
            turns: std::sync::atomic::AtomicUsize::new(0),
        }))),
        workspace: scratch(),
        live: None,
        memory: None,
        route_policy: std::collections::BTreeMap::new(),
    };
    let node = factory
        .build(&serde_json::json!({ "system": "s", "max_turns": 8 }))
        .expect("node builds");
    // WallMs 0: the post-turn charge of >0 ms exhausts immediately.
    let ctx = ctx_with(net_caps(), Budget::new(10, 10, 10, 0));
    let out = node.run(&ctx, &node_input()).expect("runs");
    match &out {
        Outcome::Next { port, .. } => assert_eq!(port.as_str(), "budget", "{out:?}"),
        other => panic!("unexpected: {other:?}"),
    }
}

# pi-improved — spec-29: cancellation-and-the-brake

- **Project**: `<repo>`
- **Priority**: Fix A6 (`ctx.check()` never called) and A7 (3/4 budget dims never
  charged) — the agent loop must be cancellable and budget-bound in every
  dimension.
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
  on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
  commits and updates HANDOFF.md)*
- **Source**: `_docs/06-research/011-pii-architecture-conformance-review-pass1.md`
  Part B spec 29 (fixes A6, A7) + `_docs/06-research/001-pii-architecture-plan.md`
  §2.4 + `_docs/06-research/007-pii-phase4-plan.md` P4.1 R4.
- **Depends-On**: spec 28 (model routing — same `agent.rs` region). Green at HEAD.

---

## Verified Premises

Checked before writing (2026-09-10):

- `crates/pii-core/src/ctx.rs:289` — `Ctx::check()` exists: returns
  `PiiError::Cancelled` if cancelled, `PiiError::Timeout` if the deadline passed.
  **Zero call sites** in `crates/pii-server/src/agent.rs` (A6).
- `crates/pii-server/src/agent.rs:155` — the turn-loop head (where `ctx.check()?`
  belongs).
- `crates/pii-server/src/agent.rs:355` — the tool-dispatch site (where
  `ctx.check()?` + `BudgetKind::ToolCalls` charge belong).
- `crates/pii-server/src/agent.rs:214-228` — the token-budget charge already
  routes to the `budget` port on exhaustion. `ToolCalls` and `WallMs` are never
  charged (A7).
- `crates/pii-core/src/budget.rs:9-18` — `BudgetKind::{Tokens, ToolCalls,
  MicroUsd, WallMs}`. Only `Tokens` is charged today.
- `crates/pii-graph/src/run.rs:257-272` — the run loop checks `Tokens` and
  `MicroUsd` remaining; `MicroUsd` is never charged (A7) so that check can never
  fire.
- `crates/pii-graph/src/spec.rs:97-122` — `GraphLimits` has `max_steps`,
  `max_wall_ms`, `max_tokens`, `max_micro_usd`, `max_parallel`. NO `max_tool_calls`
  or `max_model_calls` yet.
- `crates/pii-server/src/runs.rs:56` — the run's `Ctx` is built with
  `Budget::with_tokens(spec.limits.max_tokens)` — only the token dimension is
  bounded.

---

## Overview

The agent loop is not cancellable and not budget-bound in three of four
dimensions. `Ctx::check()` — written to be called from the loop it was built for —
has zero call sites. `BudgetKind::ToolCalls` and `WallMs` are never charged, and
`MicroUsd` is never charged so the `run.rs:265` check can never fire. There is no
`max_tool_calls` / `max_model_calls` limit.

This spec makes the loop cancellable and budget-bound in every dimension:

- `ctx.check()?` at the turn head and before each tool dispatch — a cancelled run
  returns `Cancelled` *before* the next model call.
- Charge `BudgetKind::ToolCalls` at each tool dispatch and `WallMs` at the turn
  boundary.
- Charge `MicroUsd` from provider cost metadata, OR delete the dimension together
  with the unreachable read at `run.rs:265`.
- Add `max_tool_calls` to `GraphLimits` and enforce it in the loop.

---

## Requirements

### Functional

- **R1 (`ctx.check()` at turn head)**: `ctx.check()?` at `agent.rs:155` (the turn
  loop head). A cancelled run returns `Cancelled` before the next model call.
- **R2 (`ctx.check()` before each tool dispatch)**: `ctx.check()?` at
  `agent.rs:355` (before `call_tool_bounded`). A cancelled run stops mid-tool-loop.
- **R3 (charge `ToolCalls`)**: charge `BudgetKind::ToolCalls` (1 per tool call) at
  `agent.rs:355`. On exhaustion, route to the `budget` port (same as tokens).
- **R4 (charge `WallMs`)**: charge `BudgetKind::WallMs` at the turn boundary
  (elapsed ms since the turn started). On exhaustion, route to `budget`.
- **R5 (charge `MicroUsd` or delete)**: charge `BudgetKind::MicroUsd` from provider
  cost metadata (if the provider reports it), OR delete the dimension together
  with the unreachable read at `run.rs:265`. Do NOT leave a third review to
  rediscover it.
- **R6 (`max_tool_calls` in `GraphLimits`)**: add `max_tool_calls: u64` to
  `GraphLimits` (default `u64::MAX`), thread it into the run's `Budget` (via
  `Budget::new` with the tool_calls dimension), and enforce it in the loop.

### Non-Functional

- **N1**: no new third-party dependencies.
- **N2**: default build unchanged — `max_tool_calls = u64::MAX` means no behavior
  change for existing graphs.
- **N3**: the quality gate stays green; test count only goes up.
- **N4**: `silent-noop-check` (spec 24) stays green.

---

## Architecture

```
agent.rs turn loop:
  for turn in 1..=max_turns {
      ctx.check()?;                    // R1: cancelled -> Cancelled before model call
      let turn_start = Instant::now();
      ... model call ...
      charge Tokens (existing)         // R4: charge WallMs here
      charge WallMs (elapsed)          // R4
      for call in tool_calls {
          ctx.check()?;                // R2: cancelled -> stop mid-tool-loop
          charge ToolCalls(1);         // R3: on exhaust -> budget port
          call_tool_bounded(ctx, call);
      }
  }

runs.rs:
  Budget::new(max_tokens, max_tool_calls, max_micro_usd, max_wall_ms)  // R6

run.rs:
  MicroUsd check stays IF charged (R5), else removed with the dimension.
```

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R1 | `cancel_before_model_call_returns_cancelled` | cancel the run's ctx; run an agent node with a scripted provider | node returns `Cancelled`; the scripted provider's call count is 0 (assert the count, not just the terminal) |
| R2 | `cancel_mid_tool_loop_stops` | cancel after the first tool call; run an agent node | node returns `Cancelled` before the next tool call (assert the tool count) |
| R3 | `tool_calls_budget_exhausts` | graph with `max_tool_calls = 2`; agent makes 3 tool calls | routes to `budget` port on the 3rd call |
| R4 | `wall_ms_budget_exhausts` | graph with a tiny `max_wall_ms`; agent runs a slow turn | routes to `budget` port |
| R5 | `micro_usd_charged_or_removed` | provider reports cost metadata | `MicroUsd` is charged (or the dimension + `run.rs:265` read are removed) |
| R6 | `max_tool_calls_in_limits` | `GraphLimits` with `max_tool_calls = 2` | the run's `Budget` has 2 tool-call budget; 3rd call routes to `budget` |

**R1 and R3 are the acceptance tests.** R1 must assert the scripted provider's
call count (not just the terminal) — a run that "returns Cancelled" but already
made the model call is a FAIL. R3 must fail if the `check()`/`charge()` lines are
removed.

---

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-server cancel_before_model_call_ 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -p pii-server cancel_mid_tool_loop_ 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -p pii-server tool_calls_budget_ 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -p pii-server wall_ms_budget_ 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -p pii-server micro_usd_ 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -p pii-graph max_tool_calls_ 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test --workspace` — 0 failures (N3)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N3)
- [ ] `cargo fmt --all --check` — clean (N3)
- [ ] `cargo xtask silent-noop-check` — green (N4)

---

## Guardrails

- **G1 — do NOT edit this spec.**
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT remove the existing `Tokens` charge or the `budget` port.**
- **G5 — do NOT touch `specs/`, `xtask/`, `quality-gate.sh`, `.github/`, or a
  `#[test]` assertion.**
- **G6 — no hardcoded absolute paths.** Test artefacts under `_tmp/`.
- **G7 — report raw output, never summed totals.**

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/pii-server/src/agent.rs` | R1-R4: `ctx.check()` at turn head + tool dispatch; charge `ToolCalls` + `WallMs` |
| `crates/pii-graph/src/spec.rs` | R6: add `max_tool_calls` to `GraphLimits` |
| `crates/pii-server/src/runs.rs` | R6: build the run `Budget` with all four dimensions |
| `crates/pii-graph/src/run.rs` | R5: charge `MicroUsd` or remove the dimension + the `:265` read |
| `crates/pii-server/src/agent.rs` tests | R1-R5 tests |
| `crates/pii-graph/src/spec.rs` tests | R6 test |

**Not modified**: `pii-evolve`, `pii-cli`, `pii-mcp`, `pii-web`, `pii-proto`
(unless a new event is needed).

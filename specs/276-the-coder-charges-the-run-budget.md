# Spec 276: the coder charges the run's budget with the cost its agent reports, and will not start on an empty one

## Overview
**Project**: <repo>
**Phase**: production — bounded spend. Prerequisite for any unattended fix loop and for the first real run of the composed `spec` graph (specs 255-275): `code` and `docs__docs` are two coder nodes of ONE run, and today nothing stops the second from spending after the first has spent.

The run already has a budget. `crates/pii-server/src/engine/runs.rs:59` builds the run's `Ctx` with `Budget::new(spec.limits.max_tokens, max_tool_calls, max_micro_usd, max_wall_ms)`, and the `agent` node charges it (`agent/turn.rs:182,199,217`). The `coder` node never does: it parses claude's `total_cost_usd` (`agent_claude.rs:50`) and only journals it as `claude_cost_usd` (`coder/report.rs:150`). A graph that declares `[limits] max_micro_usd` is therefore enforced for `agent` nodes and silently ignored for the node that spends the real money.

An inline `call` shares the run's `Ctx`, so a coder inside a called graph charges the SAME budget as the host's coder. No composition work is needed; the charge is the whole feature.

## Verified Premises
- **CONFIRMED** (read 2026-10-25) `Budget` has a `MicroUsd` dimension; `charge(kind, amount)` returns `Err(BudgetExhausted(kind))` when `amount` exceeds what remains and leaves the counter at 0 (`pii-core/src/budget.rs:115-135`); `remaining(kind)` reads it. A graph without `[limits]` has `max_micro_usd = u64::MAX` (`pii-graph/src/spec.rs:122`): unlimited, so nothing changes for it. Neither tracked graph (`graphs/spec.toml`, `graphs/docs.toml`) declares `[limits]` today.
- **CONFIRMED** (read) the coder is built from `coder.rs` (404/450 lines: new logic goes in a NEW file `coder/budget.rs`), refuses through `self.refuse_with(ctx, agent, model, reason, ...)` (`coder.rs:180-200`), journals `coder.exited` through `emit_exited` (`coder.rs:286`), and parses claude's facts with `run_facts(agent_output)` (cost is `Option<f64>`, absent for pi and for a run that printed no result line).
- **CONFIRMED** (read) the claude argv takes `--max-budget-usd <usd>` from `WorkerSpec.max_budget_usd` (`agent_claude.rs:99`); `input_cfg.rs:11` lets the run input supply it.
- **MEASURE FIRST (do not assume).** (1) The exact `result` line shape `run_facts` accepts: read `agent_claude.rs:40-60` and build the fake agent's line from THAT code, not from memory. (2) Whether a refused coder's `err` port carries a message the existing `refused` node of `graphs/spec.toml` journals; read `refuse_with` and one existing test that asserts a coder refusal. (3) That `ctx.budget()` is the run's own `Arc` for a coder inside an expanded `call`: read `Ctx::child_with_token` for how a node's ctx is derived. If any of these is false, STOP and say so; do not work around it.
- **NOT CLAIMED**: that pi reports cost. It does not, so a pi coder charges nothing and the budget does not bound it. This spec says so in a module comment and a test pins it as the documented limit (R3 d). NOT CLAIMED that the budget is exact: claude's number is a self-report, charged after the run ends.

## Requirements
### Functional
- **R1. Refuse before spawning on an empty budget.** In `coder/budget.rs`, `fn start_gate(ctx: &Ctx) -> Option<String>`: `Some(reason)` when `ctx.budget().remaining(BudgetKind::MicroUsd) == 0`, else `None`; the coder calls it before it spawns anything and, on `Some`, returns `self.refuse(ctx, agent, model, &reason)` with the reason text `run budget exhausted: max_micro_usd`. No agent process is started, no session file is written.
- **R2. Cap a claude run by what is left.** `fn cap_usd(ctx: &Ctx, node_cap: Option<f64>) -> Option<f64>`: when the run's remaining micro-dollars are finite (`< u64::MAX`), the cap is `min(node_cap, remaining as f64 / 1e6)`; when unlimited, the cap is `node_cap` unchanged (including `None`). The coder passes the result as `max_budget_usd` to the worker spec, so claude itself stops at what is left.
- **R3. Charge after the run.** After a claude run ends with a reported `total_cost_usd`, the coder charges `(cost * 1e6).round()` micro-dollars (never negative, `NaN` and infinities charge nothing) with `ctx.budget().charge(BudgetKind::MicroUsd, ...)`. When the charge exhausts the budget, the node still returns its normal outcome for THIS run (the work is done and its criteria are judged), and journals one extra field on `coder.exited`: `budget_exhausted = "max_micro_usd"`. The NEXT coder's `start_gate` then refuses (R1). A run with no reported cost, and every pi run, charges nothing.
- **R4. Tests, with controls.** New `crates/pii-server/tests/coder_charges_run_budget.rs` (copy the fake-agent helpers of `coder_session_per_node.rs`; the fake claude prints the one `result` line R-measure-1 found). Unit tests for the two pure functions plus graph-level tests with a `[limits] max_micro_usd` graph of two coder nodes in sequence: (a) first claude run reports 0.4 USD, budget 1.0 USD: it charges 400000, the second still starts and its argv carries `--max-budget-usd 0.6` (the remainder, not the node's own cap); (b) first run reports 1.5 USD, budget 1.0: the first run's outcome is unchanged and its `coder.exited` carries `budget_exhausted`; the second coder refuses with the reason in R1 and its agent is NEVER spawned (assert no second `coder.spawned`); (c) CONTROL: the same two-node graph with NO `[limits]` runs both and the second argv carries NO `--max-budget-usd` (unlimited stays unlimited: this is the near miss that must keep the normal path); (d) the pi limit: a pi coder under a finite budget charges nothing and `remaining` is unchanged afterwards; (e) `cap_usd` with a node cap smaller than the remainder keeps the node cap; (f) cost `NaN`/negative/absent charges nothing; (g) CONTROL: a claude run that reports cost `0.0` charges 0 and does not trip `budget_exhausted`.

### Non-Functional
- Every file under the 450-line gate; `coder.rs` must not grow past 450 (it is at 404). No new dependency. No change to `Budget`, the engine, any other node kind, `graphs/*.toml`, the lane scripts.
- The journal must carry NO argv value and no credential: only the field name and the dimension.

## TDD Contract
| Test | Expected |
|---|---|
| `a_partly_spent_budget_caps_the_next_claude_run` | argv carries the remainder |
| `an_exhausted_budget_refuses_the_next_coder_and_spawns_nothing` | refused; no second spawn |
| `CONTROL_no_limits_means_no_cap` | no `--max-budget-usd` on the second argv |
| `a_pi_coder_is_not_bounded` | remaining unchanged |
| `the_node_cap_wins_when_smaller` | node cap kept |
| `an_unusable_cost_charges_nothing` | NaN, negative, absent |
| `CONTROL_zero_cost_is_not_exhaustion` | no `budget_exhausted` |

**The requirement a plausible implementation satisfies only in appearance:** charging after the run and nothing else. Test (b) passes the first coder, the budget reads 0, and the second coder spends again. Only R1's start gate stops it, so (b) asserts the second agent is never spawned.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test coder_charges_run_budget 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test -f crates/pii-server/src/node_kinds/nodes/coder/budget.rs`
- [ ] `grep -q "start_gate" crates/pii-server/src/node_kinds/nodes/coder.rs`
- [ ] `test "$(wc -l < crates/pii-server/src/node_kinds/nodes/coder.rs)" -le 450`
- [ ] `cargo test -q -p pii-server --test coder_session_per_node 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test spec_graph_documents_after_verify 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test agent_claude_arm 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `git diff --quiet -- crates/pii-core crates/pii-graph graphs`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT change `Budget`, `Ctx`, `runs.rs`, any graph file, or any test other than the new one. Do NOT put a cost, argv or credential value in the journal. Do NOT make a pi run charge a guess. Do NOT commit.
- If a MEASURE FIRST premise is false, stop and report it.

## Follow-ups (not this spec)
1. A `[limits] max_micro_usd` in `graphs/spec.toml` itself (a policy number the operator chooses; this spec only makes any such number real).
2. Pi cost: pi has no cost report; a wall-clock or token bound for pi is a separate measurement.
3. The `net-check` allowlist entry the new test will need if its fake runtime opens a socket (specs 241/252/255/263 each needed one): add `crates/pii-server/tests/coder_charges_run_budget.rs` ONLY if `net-check` names it.

## Closure

**Verified by the orchestrator, independently of the worker's report — 2026-10-25, tree `b8358f7`, `git status --porcelain` empty before and after.**

- **All ten Exit Criteria rc=0 on the committed tree** (`_tmp/autonomy/verify-276.sh`, log `_tmp/autonomy/logs/verify-276-run.log`): C1 `coder_charges_run_budget` **9 passed**, C5 `coder_session_per_node`, C6 `spec_graph_documents_after_verify`, C7 `agent_claude_arm` green, C9 `clippy -p pii-server --all-targets -D warnings` clean, C10 `lint-files` green. A criterion naming `--lib` on a binary crate was not used: every test criterion names the target that holds the test and asserts `[1-9] passed`.
- **The claim was reddened with the orchestrator's own mutation, not with the worker's.** `start_gate`'s `remaining(...) == 0` clause was replaced by `let _ = ctx.budget().remaining(...); None` (a gate that can never refuse): `an_exhausted_budget_refuses_the_next_coder_and_spawns_nothing` and `start_gate_refuses_only_an_empty_budget` went red — `7 passed; 2 failed`, rc 101 — while the three control tests stayed green. Restored, `touch`ed (never `cp -p`), then re-run: **9 passed, rc 0**, and `sha256 e9f4525fa578c0d8ac8ecec0b46d85120b54918701f53ed6c28b599d56eb4034` identical before and after. Log `_tmp/autonomy/logs/mutate-276.log`; the tree was clean after the restore.
- **The gate reading is the item's own.** The queue committed this item only after `quality-gate.sh --routine` passed with coverage measured (row A141); this closure does not run a second `fresh-gate` on the identical HEAD, because a repeat gate on the same tree is the stale-binary class of spec 236, not a second verdict.
- **Documented limits, carried as limits rather than defects:** a pi run reports no cost, so it charges nothing and `max_micro_usd` does not bound it (pinned by `a_pi_coder_is_not_bounded`); the charge is claude's own `total_cost_usd` self-report, applied after the run ends, so the budget is a bound and not an accounting.
- **Not this spec, and still open:** nothing in `graphs/spec.toml` sets `[limits] max_micro_usd`, so the shipped composed graph is still unbounded — a policy number only the operator chooses (Follow-up 1). `selftest.sh` at this tree: **`RESULT: 40 passed, 0 failed` / `VERDICT: ALL MECHANISMS PASS`**.

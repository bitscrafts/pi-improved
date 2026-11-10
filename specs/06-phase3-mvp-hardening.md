# Spec: Phase 3.5 — MVP hardening (fix the six review BLOCKERs)

**Project**: <repo>
**Phase**: 3.5 — MVP hardening (post-Phase-3 review fixes)
**Spec of record**: `_tmp/phase3-opus-review.md` (opus deep-analysis
report, 2026-09-05, committed reasoning in `HANDOFF.md` §"OPUS DEEP
ANALYSIS"). Also consult `_docs/06-research/002-pii-architecture-plan-pass2.md`
§4 (net), §5.4 (budget), §8.7/§8.8 (daemon) for original intent.

## Overview
The Phase-3 opus review (read-only, 2026-09-05) found the substrate
(scheduler, journal, resume, validator, store, client/server split) is
solid, but the system has **never executed a real LLM turn** and has
five other structural gaps that block any real use. This spec fixes
the six BLOCKERs opus identified, in the order opus recommended as
"M0: the coding agent" — the smallest MVP that lets a user actually
run `starter/verify-fix` against a live local model.

Do NOT weaken any existing test, lint threshold, or gate to make this
pass. Every fix must be a real fix, verified by a new or existing test.

## Requirements

### B1 — Wire a live model provider (Ollama-only for now)
- `pii-model/src/lib.rs::provider_from_spec` must dispatch `"ollama:<model>"`
  to `crate::ollama::provider_from_env` (which already exists and is
  correct — it just has zero callers). Keep `"scripted:"` for tests.
- `pii-server::registry()` / wherever `Engine::new` picks a provider
  currently hardcodes `ScriptedProvider` — it must read `PII_MODEL`
  (via `pii_core::env_var`, already the sanctioned single entry point
  for env reads) and call `provider_from_spec`, falling back to the
  scripted fixture only if `PII_MODEL` is unset (keeps existing tests
  green).
- New integration test: with `PII_MODEL=ollama:<any>` unset (or set to
  an unreachable port), verify a clear, non-panic error surfaces
  through the run's `Terminal::Failure`, not a process exit or panic.
- Do not add TLS, do not add an Anthropic provider (out of scope,
  cut per parent spec).

### B2 — Fix the runtime node-capability default
- `pii-graph/src/toml_load.rs` computes each node's default caps from
  `NodeRegistry::builtin()` (4-5 built-in kinds only). But the actual
  runtime registry (`pii-server::registry()`) additionally registers
  `agent`, `tool`, `gate`, `memory`, `crash` — so at runtime those
  kinds get `CapKind::empty()` and every capability check silently
  denies.
- Fix: `load_toml` (and any caller of it, e.g. `pii graph lint`, the
  server's graph-load path) must accept an optional `&NodeRegistry`
  parameter (default `NodeRegistry::builtin()` for callers that don't
  have the full registry, e.g. lint-only paths — but the SERVER path
  that actually runs graphs MUST pass its real, full registry with
  agent/tool/gate/memory/crash all registered). Do not change the
  registry's `required_caps_of` semantics — only make sure the caller
  passes the registry that will actually execute the graph.
- New integration test: load `graphs/starter/verify-fix.toml` (or
  whichever starter graph has an `agent`/`tool` node) through the
  SAME path the daemon uses, assert the loaded node's caps are
  non-empty and match the kind's `required_caps()` — not
  `CapKind::empty()`.
- Also fix the false negative in `pii-server/src/lib.rs` where a
  failed `redb` open for `lint-approval.redb`/`lint-memory.redb`
  silently skips registering `gate`/`memory` (produces a wrong `V0:
  unknown kind 'memory'` on a fresh machine) — surface that failure
  as a real error instead of silent registry gap, OR make the paths
  configurable/relative instead of hardcoded (see B3 below — this
  overlaps).

### B3 — De-hardcode `<repo>` paths
- `crates/pii-server/src/daemon/router.rs` (~line 46-71) scopes every
  channel's fs capability to the literal string
  `"_tmp/phase3"`. Replace with a value derived
  from the daemon's configured workspace/runtime dir (already exists
  as `EngineConfig::workspace` or equivalent — reuse it, don't invent
  a new config key unless none exists).
- `pii-server/src/lib.rs` opens `lint-approval.redb`/`lint-memory.redb`
  at a hardcoded absolute path — make these relative to the runtime
  dir or an explicit configured path, consistent with the fix above.
- New test: run a lint/registry-construction path from a different
  CWD (e.g. `/tmp`) and confirm it does not depend on
  `<repo>` being the literal string anywhere in the
  produced registry/paths.

### B5 — Fix the leaked UDS connection (this closes the real rss-gate EC)
- `pii-proto/src/client.rs::Client` has no `Drop` impl and no
  `shutdown()` method; `reader_task` (the split-read half) parks in
  `next_line()` forever once the writer half is dropped, because
  `tokio::io::split` only closes the underlying fd when BOTH halves
  drop. Add `Client::shutdown(&self)` that aborts/joins both the
  reader and writer tasks and drops both halves, and call it from
  every caller that currently just drops a `Client` (`pii soak`'s
  driver, `cmd_soak.rs`, and any other one-shot client use).
- Consider adding `impl Drop for Client` as a backstop that aborts the
  reader task if `shutdown()` was never called (best-effort, not a
  substitute for explicit shutdown in the hot path).
- Server side: `cmd_daemon.rs`'s accept loop and `conn.rs`'s
  per-connection state have no idle timeout and no connection cap.
  Add a bounded idle timeout (e.g. drop a connection with no traffic
  for N seconds — pick something that does not break existing tests,
  document the constant) so a client that vanishes without a clean
  close does not pin server-side resources forever.
- Verification (this is the actual EC — do NOT weaken the gate):
  `cargo xtask rss-gate --secs 300 --interval 10 --max-mb 50 --runs 3000`
  must pass. For fast iteration use `--secs 60 --interval 5 --runs 2000`
  (~1 min) and only run the full 300s form for the final check. Also
  spot-check `ls /proc/<daemon-pid>/fd | wc -l` stays roughly flat
  across a soak run, not growing with run count.

### B6 — Wire the budget system
- `pii-server/src/runs.rs` currently constructs every run with
  `Budget::unlimited()`. Add a `[limits]` / `max_model_calls` (already
  parsed from TOML per spec `05-phase3-assistant-gateway-part3.md` if
  present — check `pii-graph/src/spec.rs`/`toml_load.rs`; if
  `max_model_calls` is parsed but discarded, wire it through to a real
  `Budget` charged per model call) so a graph's declared
  `max_model_calls` actually limits execution.
- `pii_core::Budget::charge` already exists (`pii-core/src/budget.rs`)
  — use it, do not reinvent. The `agent` node factory
  (`pii-server/src/nodes/agent.rs` or wherever chat calls happen) must
  call `ctx.budget().charge(BudgetKind::ModelCalls, 1)` (or whatever
  the existing `BudgetKind` variant is named — check the enum) before
  each model call, propagating a breach as a real, distinct
  `Terminal::BudgetBreach` variant (add one to `pii_abi::Terminal` if
  it does not already exist) with a dedicated exit code (use exit 3,
  per the parent spec's existing convention — check `main.rs` for
  the existing exit-code table and do not collide with an existing
  code; note the review flagged exit 3 is currently shared with lint's
  own error path — resolve that collision, pick a genuinely free code
  if needed and document the choice).
- New test: a graph with `max_model_calls = 1` and an agent loop that
  would call the model twice must terminate with `BudgetBreach` on the
  second call, not run unbounded.

### M1 — Fix `event.unsubscribe`
- `pii-server`'s `dispatch.rs` `event.unsubscribe` handler is currently
  a no-op; the subscription task it's supposed to cancel only exits
  when it has something to send, so unsubscribing (or a client that
  disconnects) leaks a task + its mpsc channel forever.
- Fix: give each subscription a cancellation handle (e.g. a
  `CancellationToken` or a oneshot the subscriber task selects on)
  that `event.unsubscribe` actually triggers, and that a connection's
  teardown path also triggers for all of that connection's live
  subscriptions.
- Enforce the advertised `max_subscriptions: 32` cap (currently
  advertised, never counted) — reject a 33rd `event.subscribe` on a
  single connection with a clear protocol error rather than silently
  accepting it.
- New test: subscribe, unsubscribe, assert (via a task-count probe or
  timing) the subscriber task actually exits; subscribe 33 times on
  one connection, assert the 33rd is rejected.

## Non-Functional
- No file exceeds 450 effective lines (`cargo xtask lint-files`).
- `unsafe_code = "forbid"`; existing lint policy unchanged.
- No new third-party crates without discussion — these are all fixes
  within existing crates using existing dependencies.
- Do not touch Discord (B4/TLS is explicitly cut per the parent
  Phase-3 spec and stays cut here — out of scope for this pass).
- Do not touch `subgraph`/`map`/`llm`/`net`/`nano` node kinds — out of
  scope (M2 "composition" in the MVP roadmap, not this pass).
- Do not touch `pii-obs`/gateway/workers-beyond-tests/memory-embedder
  — all explicitly out of scope for this MVP-hardening pass (later
  milestones per `HANDOFF.md`'s "After M0" section).

## Exit Criteria
- [ ] `cargo build --workspace --all-targets`
- [ ] `./quality-gate.sh` (fmt + clippy + test, all PASS)
- [ ] `cargo xtask lint-files`
- [ ] `cargo xtask deps-check`
- [ ] `PII_MODEL=ollama:test-model cargo test -p pii-model provider_from_spec_dispatches_ollama -- --exact` (or equivalent new test name; must exist and pass)
- [ ] `cargo test -p pii-graph load_toml_uses_full_registry_caps -- --exact` (or equivalent new test name proving B2 is fixed)
- [ ] `cargo test -p pii-server budget_breach_stops_run -- --exact` (or equivalent new test name proving B6 is fixed)
- [ ] `cargo test -p pii-server unsubscribe_actually_cancels -- --exact` (or equivalent new test name proving M1 is fixed)
- [ ] `cargo xtask rss-gate --secs 300 --interval 10 --max-mb 50 --runs 3000`
- [ ] `grep -rL "/projects/pi-improved" crates/pii-server/src/daemon/router.rs` (i.e. the hardcoded path string is gone from that file; adjust the exact check to whatever the fix's replacement looks like, the point is: no more literal `/projects/pi-improved` in daemon/router.rs)

## Guardrails
- Do NOT weaken the rss-gate thresholds, the lint-files line cap, or
  any existing test to make an exit criterion pass.
- Do NOT touch Discord/TLS/gateway/subgraph/obs — explicitly cut from
  this pass, see Non-Functional.
- Do NOT edit this spec file or any file under `specs/` — you are the
  workhorse, not the planner.
- Checkpoint-commit clean-building work between exit criteria if you
  hit a tool-call/iteration budget mid-task — do not lose work.

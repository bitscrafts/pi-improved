# Spec 288: a declared `needs` key that the run input cannot supply refuses the run at start, before any node runs

## Overview
**Project**: <repo>
**Phase**: production — composability, closing a gap that spec 284's daemon test measured. Spec 267 made a called graph's declared `needs` refuse the run when a key resolves to nothing. It does so with a `require` node at the **head of the called graph**, so the refusal arrives only when the scheduler reaches the call. In the shipped `spec` graph that is **after `code` has run** — a full coder pass (measured 2026-10-26: 50 turns, $0.245 on haiku) is spent before a missing `docs_brief` is noticed. The A148 worker found it and reported it instead of bending the test (spec 284's R3 item 2 was amended to pin the late behaviour).

A required input that is absent from the run input is known at **run start**: the loader recorded each call's `needs` and its mapped `in` keys (`CallRecord`, spec 269). This spec adds a preflight to `Engine::start_run` that refuses such a run before anything runs, with the same error text the `require` node would give, and leaves the `require` node as the second line of defence for keys that only resolve at run time (a mapped expression over a node output).

## Verified Premises
- **CONFIRMED** (read 2026-10-26) `Engine::start_run` (`crates/pii-server/src/lib.rs:443`) calls `load_spec(graph)` then starts the run; `load_spec` weaves calls via the loader and the weave keeps the `calls: Vec<CallRecord>` (spec 269) with `needs: Option<Vec<String>>` and `input_keys: Vec<String>` (the keys the call maps). The refusal text of the run-time node is `graph.call.need: call '<id>' graph '<g>' needs '<key>'` (`nodes_require.rs`).
- **CONFIRMED** the A148 worker's measurement: with `docs_brief` absent, exactly one coder (`code`) spawned before `graph.call.need` ended the run.
- **MEASURE FIRST** how `start_run` can reach the call records (the loaded spec may not carry them; `load_host_with_graphs` returns them — read how `load_spec` calls it) and how the engine reports a refusal that happens before a run exists (the existing refusal tests: `run_start_refuses_bad_input.rs`, `crates/pii-cli/tests`). The preflight returns the same `PiiError` variant those use.
- **MEASURE FIRST** which keys the preflight can decide: a `needs` key that the call's `in` table maps is decided by the **mapped expression**; only the case `in` does not map it (the key reads the host run input, spec 261) is decidable from the input alone. A mapped key is left to the run-time node. The preflight must not refuse a run the run-time node would accept.
- **NOT CLAIMED** that every missing input is caught early: optional inputs the graph reads outside `needs` (for example `max_budget_usd` for a pi run) must stay optional, and a key mapped from a node output is only knowable at run time.

## Requirements
### Functional
- **R1. Preflight in `start_run`.** After the graph loads and before any run exists: for each call record whose `needs` is `Some`, every key that is **not** in `input_keys` must be present and non-null in the run input (a missing, `null`, or absent key refuses). The error is `graph.call.need: call '<id>' graph '<g>' needs '<key>'` (identical text to the run-time node), names the **first** missing key in `needs` order, and **no run is created** (no header, no journal, no coder spawn). A nested call's needs are covered (the records already list them with their `parent`).
- **R2. Near-miss behaviours, each a test:** a key present and non-null → the run starts as before; a key mapped by the call's `in` → not refused by the preflight (decided at run time as today); a graph with no calls, and a call whose graph declares no `needs` → unchanged, byte-for-byte; an **empty string** is accepted (spec 267: empty string is a value) .
- **R3. Through the engine, on the shipped graph.** `Engine::start_run("spec", input-without-docs_brief)` returns the refusal **and the store holds no run for it**; the same input with `docs_brief` present starts. This replaces the late behaviour spec 284 pinned: the worker for this spec updates the one assertion in `spec_graph_runs_through_the_daemon.rs` that pins "one coder spawned first" to pin "refused at start, nothing spawned" — and **only** that assertion, and says so in the commit message.
- **R4. The run-time node stays.** The existing run-time refusal tests (`call_requires_needs`, `daemon_journals_calls`) pass unchanged, and a new test shows the run-time node still refuses a key the preflight cannot decide (a mapped key that resolves to null).

### Non-Functional
- The preflight lives in a new file (`crates/pii-server/src/engine/preflight.rs` or the module the MEASURE FIRST finds natural); `lib.rs` and `fragment/call.rs` stay under the 450-line gate. No new dependency; no change to `graphs/`.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test start_refuses_missing_needs 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test "$(cargo test -q -p pii-server --test start_refuses_missing_needs -- --list 2>/dev/null | grep -c ': test$')" -ge 6`
- [ ] `cargo test -q -p pii-server --test spec_graph_runs_through_the_daemon 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-graph --test call_requires_needs 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test daemon_journals_calls 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test -f crates/pii-server/src/engine/preflight.rs`
- [ ] `git diff --quiet -- graphs crates/pii-graph/src`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `crates/pii-server/src/engine/preflight.rs`
- `crates/pii-server/src/engine/mod.rs`
- `crates/pii-server/src/lib.rs`
- `crates/pii-server/tests/start_refuses_missing_needs.rs`
- `crates/pii-server/tests/spec_graph_runs_through_the_daemon.rs`

## Guardrails
- Do NOT change a `graphs/` file or the loader. Do NOT weaken or delete an assertion except the ONE named in R3. Do NOT make the preflight refuse a key the run-time node would accept. Do NOT commit.

## Follow-ups
Optional-input declaration (`needs` vs `wants`) so `max_budget_usd` can be declared; a `pii graph check --input` that runs the same preflight offline.

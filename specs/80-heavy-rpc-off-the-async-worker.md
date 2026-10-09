# Spec: 80 — heavy RPC methods stop blocking the async worker (A1)

**Project**: /projects/pi-improved
**Phase**: batch D — 043 §10 item 10 / §7 A1.

## Context
`crates/pii-server/src/engine/conn.rs:172` calls
`dispatch::dispatch(&engine, &req, peer, &wtx, &subs)` SYNCHRONOUSLY inside the
connection's async loop. A slow handler (memory search/eval, worker reload, run trace,
session prompt) therefore blocks the tokio worker thread — on a `current_thread`
runtime, the whole daemon; on multi-thread, one worker per blocked request.

## Requirements
R1. The four heavy methods run OFF the async worker: `memory.*`, `worker.reload`,
    `run.trace`, `session.prompt` — i.e. inside `tokio::task::spawn_blocking` (or an
    equivalent off-worker mechanism the codebase already uses). Light methods may stay
    inline; if you route everything through the off-worker path, prove it with R3 too.
R2. No semantic change: response ordering per connection, subscription ownership
    (`&subs`), and the `hello` gate all behave exactly as today. If the shared
    `SubscriptionRegistry` must be shared across the blocking call, put it behind an
    `Arc` — do not clone the registry per call.
R3 (TDD, RED first). A `current_thread`-runtime heartbeat test per heavy method: a
    background task ticks every 10 ms while the method runs against a deliberately slow
    stub; the test asserts the tick is never delayed by more than 50 ms. It must FAIL
    before the fix (record the observed delay) and pass after. Add a CONTROL: the same
    test for a light method must pass both before and after (proving the test measures
    the off-worker property, not something else).

## Exit Criteria
- [x] `[ "$(grep -c 'spawn_blocking' crates/pii-server/src/engine/conn.rs)" -ge 1 ]` (verified: `is_heavy` + `run_handler` route the 6 heavy methods)
- [x] `cargo test -p pii-server heartbeat` → **7 passed / 0 failed** (6 heavy + the light-method control)
- [ ] `cargo test -p pii-server 2>&1 | grep -c 'test result: FAILED' | grep -q '^0$'`
- [ ] `cargo clippy -p pii-server --workspace --all-targets -- -D warnings` exits 0 (run workspace-wide: the change touches shared types)
- [ ] `cargo fmt --all --check` exits 0
- [ ] the new tests plus their light-method control are listed in the report with their before/after measurements

## Guardrails
- Do NOT change the RPC protocol, method names, error codes, or the response framing.
- Do NOT introduce `block_on` inside an async context, and do NOT spawn detached tasks
  that outlive the connection (teardown must still cancel subscriptions — see the M1
  comment in that file).
- Do NOT use unwrap/expect/panic (workspace lints deny them in production paths).
- Do NOT commit; leave the tree for the orchestrator. Scratch under `_tmp/` only.
- If the four-method scope cannot be done without a protocol change, STOP and report
  the blocker instead of widening scope.

## Orchestrator verification (2026-10-09)
- Shape: `run_handler` awaits the blocking call before the loop reads the next frame, so
  response ordering, the `hello` gate and the M1 `cancel_all` teardown are unchanged;
  `SubscriptionRegistry` is behind an `Arc` and the Arc (not the registry) is cloned per call.
  The test seam is `pub(crate) serve_connection_with(.., handler)`; production
  `serve_connection` passes the real `dispatch::dispatch`, and the tests substitute ONLY the
  work (a 300 ms stub) — the routing logic under test (`is_heavy` + spawn_blocking + await) is
  the real one.
- **Mutation control (orchestrator)**: forcing `is_heavy` to `false` (the pre-fix inline shape)
  makes all 6 heavy tests FAIL — "blocked the async worker: tick late by 299.4ms" — while the
  CONTROL `heartbeat_control_light_method_never_late` still PASSES. The tests measure the
  off-worker property and are not decoration.
- Worker-reported honesty: its first ticker version was flaky (a pending tick when stopped went
  unrecorded, so some heavy methods passed by accident); it fixed the ticker and re-ran 4/4.
  Measured worst-case tick delay before/after: ~299 ms → ~1.3-1.5 ms.
- Full gate: `quality-gate.sh` ALL PASS.

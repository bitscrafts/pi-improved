# Spec 308: runs have their own thread pool, so the control plane is never queued behind a run, and a queued run can be cancelled

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composability, daemon control plane (found by the composition simulations 2026-10-28, ranked and shaped by an Opus review: `_tmp/opus-composability-limits-20261028/opus-review-plan-20261028T103657Z.md`)
Every run holds one thread of tokio's blocking pool for its whole life (`spawn_blocking`, `crates/pii-server/src/engine/runs.rs:92` and `:481`), and the daemon caps that pool at 8 (`PII_DAEMON_MAX_BLOCKING`, `crates/pii-cli/src/cmd_daemon.rs:171`). The same pool serves the heavy RPCs (`conn.rs:212-228`: `RunTrace`, `EventSubscribe`, `SessionPrompt`, memory and skill calls), channel polls (`daemon/router.rs:44`) and the learn timers. With more than 8 runs in flight those calls queue behind whole runs: measured 12 runs in flight, slowest `pii run get` 19.2 s (the follow-up `RunTrace`), 20 runs with a 45 s docs step, every call blocked 94 s (`/tmp/runget3.out`). Two more defects sit beside it: a run that is queued (registered, header and `Started` written, no node open) cannot be cancelled cleanly, and `run cancel --kill` sleeps up to 30 s on one of only 4 async workers (`runs_cancel.rs:124-134`).
The cap exists for a real reason (RSS climbed with thread churn, 279 threads under a 3000-run soak: the comment at `cmd_daemon.rs:153-165`), so it is not lifted. Runs move to a fixed-size pool of long-lived threads; the shared pool then serves only short work.

## Requirements
### Functional
- R1: a new module `crates/pii-server/src/engine/run_pool.rs` holds a fixed-size pool of N named OS threads (`pii-run-0` ...). N is `PII_DAEMON_RUN_THREADS` (default **8**, a value below 1 or unparsable falls back to 8). Each thread enters the daemon's tokio runtime once (`Handle::enter`, because `require_runtime` and the cron re-arm need a handle) and pulls boxed jobs from a queue. A job that panics is caught and logged and does not kill its thread.
- R2: `spawn_run` and `spawn_resume` in `runs.rs` submit their closure to this pool instead of calling `spawn_blocking`. Nothing else in the daemon is moved onto it. The pool is owned by `Engine` and created with it.
- R3: a run that is queued in the pool (its closure has not started) is reported `running` as today and **can be cancelled**: `run cancel <id>` without `--kill` closes it (the executing node is empty, so there is nothing to refuse); the pool job checks the run's cancel token first and, when tripped, does not call `execute` and writes nothing a second time. A run cancelled while queued ends with the same journal shape as a run cancelled between nodes today.
- R4: `run cancel --kill` no longer sleeps on an async worker. `is_heavy` in `conn.rs` takes the request, not only the method, and treats `RunCancel` with `kill` true as heavy, so the 30 s wait runs on the shared blocking pool. `RunCancel` without `kill` stays inline.
- R5: nothing about the other pool changes: `PII_DAEMON_MAX_BLOCKING` keeps its default 8 and meaning, and `RunTrace`, `EventSubscribe` and the others stay heavy.
- R6: the daemon's shutdown stays bounded with jobs still queued (the existing `shutdown_is_bounded_by_a_stuck_blocking_task` test stays green and a queued job at shutdown is dropped, not awaited).

### Non-Functional
- Rust, clippy and fmt at zero; every file under 450 effective lines (`runs.rs` is already 781 lines in total, so the new code goes in `run_pool.rs` and `runs_cancel.rs`, not in `runs.rs`); the observability lint budget for raw prints is not raised.
- No hardcoded paths. No new dependency: `std::thread` and `std::sync::mpsc` with a shared receiver are enough.

## Verified Premises
- **CONFIRMED** `crates/pii-server/src/engine/conn.rs:212-228` `is_heavy` lists `RunTrace`, `EventSubscribe`, `SessionPrompt` and the memory, learn and skill methods, and does not list `RunCancel` or `RunGet`.
- **CONFIRMED** `crates/pii-cli/src/cmd_daemon.rs:166-175` `worker_threads` default 4 and `max_blocking_threads` default 8.
- **CONFIRMED** `crates/pii-server/src/engine/runs_cancel.rs:124-134` `stop_executing` loops with `std::thread::sleep(50 ms)` for up to `wait` (30 s) and is reached from the inline `RunCancel` handler.
- **CONFIRMED** `crates/pii-server/src/engine/runs_cancel.rs:61-77` a cancel without `--kill` on an in-flight run is refused with `executing node '<n>'`; `executing_node` returns the empty string when no node is open.
- **CONFIRMED (measured)** `/tmp/runget3.out`: 20 runs in flight, docs step 45 s, every `run get` timed out at 8 s for 94 s, then answered at once; at 20 s the stall was 41 s; `health` and `run list` were never slow.
- **INFERRED (Opus, read the code, not run)** a run queued behind a full pool is registered and journalled `Started` with no node open, and today a kill-cancel of it waits 30 s and fails. The first TDD test measures it before any change.
- **CORRECTED** an earlier note said `run get` itself is the heavy call. It is not: the status comes from the light `RunGet` and the coder lines come from `RunTrace` (`cmd_trace/run.rs:49`), which is heavy.

## TDD Contract
| Test (crates/pii-cli/tests/control_plane_is_not_behind_runs.rs) | Input | Expected |
|---|---|---|
| `trace_subscribe_and_cancel_answer_while_runs_fill_the_pool` | private daemon, `PII_DAEMON_MAX_BLOCKING=2`, `PII_DAEMON_RUN_THREADS=2`, 4 runs whose fake agent sleeps 20 s; then on run 1: `run trace`, `event subscribe`, and `run cancel --reason x` on run 4 (queued) | each answers in under 2 s; run 4 closes `cancelled` |
| `CONTROL_at_most_run_threads_agents_are_alive` | the same start, count live fake agents every 200 ms | never more than 2 at once (the fix did not lift the cap) |
| `CONTROL_threads_stay_bounded_after_300_short_runs` | 300 sequential short runs, read `Threads:` from `/proc/<pid>/status` before and after | the count after is at most the count before plus 4 (the RSS-gate property) |
| `a_kill_cancel_does_not_freeze_the_async_workers` | 4 concurrent `run cancel --kill` on agents that ignore SIGTERM, time `health` meanwhile | `health` answers in under 2 s throughout |
| `a_cancel_without_kill_of_a_running_node_is_still_refused` | cancel a run whose node is executing, no `--kill` | refused with `executing node` (the old behaviour) |
| `a_queued_run_is_cancelled_and_never_executes` | 3 runs on 1 run thread, cancel the queued third | it never spawns an agent (spawn count 2) and its journal has no second terminal |
| existing `shutdown_is_bounded_by_a_stuck_blocking_task` and `run_cancel_kill` | unchanged | still pass |

## Exit Criteria
- [ ] `cargo test -q -p pii-cli --test control_plane_is_not_behind_runs 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `test "$(grep -c '^#\[test\]\|^#\[tokio::test' crates/pii-cli/tests/control_plane_is_not_behind_runs.rs)" -ge 6`
- [ ] `cargo test -q -p pii-cli --test run_cancel_kill 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `cargo test -q -p pii-cli --test run_cancel 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `cargo test -q -p pii-cli shutdown_is_bounded 2>&1 | grep -q "test result: ok. [1-9]"`
- [ ] `grep -q 'PII_DAEMON_RUN_THREADS' crates/pii-server/src/engine/run_pool.rs`
- [ ] `! grep -q 'spawn_blocking' crates/pii-server/src/engine/runs.rs`
- [ ] `cargo clippy -q -p pii-server -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `cargo run -q -p xtask -- lint-observability 2>&1 | tail -1 | grep -q " 0 findings"`
- [ ] `git diff --quiet -- graphs scripts`

## Files
- `crates/pii-server/src/engine/run_pool.rs`
- `crates/pii-server/src/engine/runs.rs`
- `crates/pii-server/src/engine/runs_cancel.rs`
- `crates/pii-server/src/engine/conn.rs`
- `crates/pii-server/src/engine/mod.rs`
- `crates/pii-server/src/lib.rs`
- `crates/pii-cli/tests/control_plane_is_not_behind_runs.rs`

## Guardrails
- Do NOT edit `graphs/`, `scripts/`, or an existing assertion. Do NOT raise `PII_DAEMON_MAX_BLOCKING`'s default and do NOT move any heavy RPC onto the run pool. Scratch under `_tmp/` inside the repository.
- The appearance-only implementation: a larger blocking cap (the pool-thread control and the 300-run thread count fail it), `RunTrace` dropped from `is_heavy` alone (`event subscribe` still waits), or a queue-cancel that marks the header cancelled but still lets the job run (the spawn-count test fails it).
- Mutations the tests must kill: the run pool size ignored; a job executed after its cancel token tripped; `RunCancel --kill` left inline; the pool thread not entering the runtime; a panicking job killing its thread.
- Do not edit `runs.rs` beyond submitting to the pool; it is over the file-size line already, so new logic goes in the new module.

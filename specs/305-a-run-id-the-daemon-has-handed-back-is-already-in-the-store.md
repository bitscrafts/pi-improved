# Spec 305: a run id the daemon has handed back is already in the store

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composability, durability (found by simulation 2026-10-28, `composition-sim.py ackloss`, probe `probe-sim.py durable`)
`Engine::start_run` returns the run id "BEFORE execution". The run's header and its `Started` event are written later, inside `spawn_blocking`, by `pii_graph::run::run`, after graph validation, node build and two `put_graph` calls, and flushed by `jr.flush()`.
A caller that has been given an id and then loses the daemon (SIGKILL, OOM, power) in that window finds the id **unknown to the restarted daemon** ("run 'rNNN' is unknown to this daemon (not in flight, not stored)"). MEASURED on a private daemon with stand-in agents: killing at 0 s after the last acknowledgement lost 1 of 6 ids; at 1, 2, 4 and 9 s, 0 of 6. The queue lane trusts the id it receives (it polls it, and cancels it on give-up), so an id that can vanish is a hole in "always".

## Requirements
### Functional
- R1: when `Engine::start_run` returns `Ok(run_id)`, the store already holds that run: `store.get_run(&run_id)` is `Some` with status `Running`, and the journal holds its `Started` event. Write the header and `Started` synchronously inside `start_run` (before `spawn_run` returns), then let the spawned task continue from there. `run()` must not write a second `Started`: pass the already-started journaler/state through, or have `run()` accept an already-created run. Keep the existing refusal path (spec 115 R1): a start refused before the first journal write still journals a terminal `Failed` for its id.
- R2: the same holds for the two other callers of `spawn_run` that hand out ids (`crates/pii-server/src/engine/runs.rs:546`, `crates/pii-server/src/daemon/mod.rs:693`): either they go through the same early write or the spec states, in the PR text, why their id is never handed to a caller before the write.
- R3: `start_run` stays fast: the added synchronous write is one transaction. Measure and state its cost; N4's bound is 50 ms.

- R4 (found by the gate, MEASURED 2026-10-28: `webhook_post_makes_a_run_and_the_reply_is_posted_to_the_sink` fails 5 of 5 with the gate's own command on a tree with R1, and passes on HEAD): `crates/pii-server/src/daemon/router.rs` registers a channel run in `channel_runs` AFTER `engine.start_run` returns. With R1 the validation, build and header write happen before the return, so a graph that finishes at once (the `echo` graph) can reach its terminal event before the registration, and the reply lookup finds no run: nothing is posted. The caller must be able to register the run BEFORE it can finish: add `Engine::start_run_with(graph, input, on_id)` where `on_id: impl FnOnce(&RunId)` is called after the id is allocated and before the run is spawned; `start_run` calls it with a no-op; the router registers through it. Every other caller of `start_run` whose result is looked up by run id from an event handler (cron, spawn node) is checked for the same ordering and the finding recorded in the PR text.

### Non-Functional
- Rust, clippy and fmt at zero. No file over the 450 effective-line limit. No change to `graphs/`, `pii-store` table layout, or the journal format.

## Verified Premises
- **CONFIRMED** `crates/pii-server/src/lib.rs:443` `start_run` calls `store.next_run_id()` then `engine::runs::spawn_run`, which returns `Ok(())` right after `spawn_blocking`.
- **CONFIRMED** `crates/pii-graph/src/run/mod.rs:69-130` `run_with_validate` writes `Started` and calls `jr.flush()` only after `validate_for_exec`, `build_nodes`, `put_graph` x2.
- **CONFIRMED** (probe 2026-10-28 01:47Z, private daemon, stand-ins): ack-to-kill 0 s -> 1 of 6 ids unknown; 1/2/4/9 s -> 0 of 6.
- **CONFIRMED** (2026-10-28, gate run and a bisect with the gate's command): the webhook test fails with R1's sources and passes on HEAD; `daemon/router.rs:155-170` registers after `start_run` returns.
- **CONFIRMED** run ids are reserved in blocks (`next_run_id`), so a reused id is not the cause.

## TDD Contract
| Test (crates/pii-server/tests/acked_run_survives_kill.rs) | Input | Expected |
|---|---|---|
| `an_acknowledged_run_is_in_the_store` | 200 x `start_run` of a graph whose first node blocks, each checked IMMEDIATELY after it returns | every id has `get_run == Some`, status Running, and a `Started` event |
| `CONTROL_a_refused_start_still_resolves` | a graph that fails validation | the id resolves to a `Failed` run with the reason (spec 115 R1 unchanged) |
| `a_second_started_is_never_written` | one run to completion | exactly one `Started` event in its journal |
| `crates/pii-server/tests/channel_adapters.rs` `webhook_post_makes_a_run_and_the_reply_is_posted_to_the_sink` (existing, unchanged) | the gate's command | passes |
| `a_run_is_registered_before_it_can_finish` (acked_run_survives_kill.rs) | `start_run_with` on a graph that finishes at once, the callback records the id | the callback ran before the run's terminal event, 50 times |
| `the_start_stays_fast` | 50 sequential `start_run` calls | the slowest is under 50 ms (print the median and max) |

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test acked_run_survives_kill 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `test "$(grep -c '^#\[\(tokio::\)\?test' crates/pii-server/tests/acked_run_survives_kill.rs)" -ge 4`
- [ ] `cargo test -q -p pii-graph 2>&1 | grep -c "test result: ok" | grep -qv '^0$'`
- [ ] `cargo test -q -p pii-server --test run_get_names_the_failure 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `cargo test -q -j 2 --workspace --lib --bins --tests -- webhook_post_makes_a_run 2>&1 | grep -q "test result: ok. 1 passed"`
- [ ] `grep -q 'start_run_with' crates/pii-server/src/daemon/router.rs`
- [ ] `cargo clippy -q -p pii-server -p pii-graph --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `git diff --quiet -- graphs scripts crates/pii-store`

## Files
- `crates/pii-server/src/lib.rs`
- `crates/pii-server/src/engine/runs.rs`
- `crates/pii-graph/src/run/mod.rs`
- `crates/pii-server/tests/acked_run_survives_kill.rs`
- `crates/pii-server/src/daemon/router.rs`

## Guardrails
- Do NOT change an existing assertion, `graphs/`, `scripts/` or `crates/pii-store`. Scratch under `_tmp/` inside the repository.
- The appearance-only implementation: writing the header early but leaving `Started` to the task (the id resolves, the journal is empty, a restart still reconciles it with no `Started`). The test that catches it asserts the `Started` event, not only the header.
- Mutations the tests must kill: the synchronous write removed (id unknown right after return); the refusal path dropped; `Started` written twice; the write moved after `spawn_blocking`.

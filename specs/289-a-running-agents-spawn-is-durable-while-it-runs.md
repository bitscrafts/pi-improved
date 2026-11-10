# Spec 289: a running agent's `coder.spawned` is durable while it runs, so a restart can reap it

## Overview
**Project**: <repo>
**Phase**: production — a defect that spec 285's restart case measured (not a composition defect: it reproduces in a one-node graph). `crates/pii-graph/src/sched/dispatch.rs:276` journals a node's agent-call observations (`coder.spawned`, `coder.exited`) only **after** `node.run` returns (`for observation in recorder.take()`). While the coder runs, its `coder.spawned` — the only record of the agent's pid and argv0 — lives in `CallRecorder.seen`, a memory buffer. Spec 242's orphan reaper (`engine/orphans.rs::reap_orphans`) searches the **store** for a `coder.spawned` without a `coder.exited`; a daemon that dies mid-coder never wrote the spawn, so the reaper finds nothing and the agent keeps running.

Measured 2026-10-26 (the A149 worker's `m2_restart_inside_a_call`, then the orchestrator's controls): the live store journal holds 26 events with `graph.call` and the host's finished `code` coder, and **no** `coder.spawned` for the running docs coder — for 5 s of polling, in the composed graph and in the `docs` graph run alone. Spec 242's own test (`orphans_are_reaped.rs`) builds the `coder.spawned` event by hand and stores it, so it cannot see this: its input came from the test, not from the live path.

## Verified Premises
- **CONFIRMED** (read 2026-10-26) `dispatch.rs:270-282`: the recorder wraps the context; `run_with_barrier` runs the node on the dispatch thread with a scoped watcher thread that, after `START_DURABLE_WITHIN` (2 s), calls `env.jr.flush_if_stale`; the observations are appended only after the node returns.
- **CONFIRMED** `Journaler::append` (`journal.rs:200`) assigns the seq eagerly and buffers; `flush_if_stale(window)` flushes only when events are pending and the last flush is at least `window` old.
- **CONFIRMED** `CallRecorder::take` drains `seen`; `Observer::event` pushes to it from the node's thread.
- **MEASURE FIRST** the ordering contract: spec 217 says the observations are journalled **before the node's own `NodeFinished`**, in the order the node made them. Appending them early must keep that order, and must not **duplicate** them when the node returns (drain-once). Read `agent_calls.rs` (`unmatched_spawns`, `pid_of`) and `run_trace` for anything that assumes spawn precedes exit **adjacently**.
- **MEASURE FIRST** whether `env.jr` (the `Journaler`) is safe to append to from the watcher thread while the dispatch thread is blocked in `node.run` (it holds `state: Mutex`; `append` locks it briefly). It is used by the barrier's watcher already (`flush_if_stale`), so a second thread appending is expected to be safe; confirm there is no deadlock with the engine's own `append` after `node.run`.
- **MEASURE FIRST** the cost: a coder run that spawns once and runs for minutes must cost **one** extra append, not a poll loop's worth. Prefer an event-driven design (the recorder signals a channel when it records a `coder.*` event) over polling; if polling is chosen, the interval is ≥ 250 ms and the test counts transactions.
- **NOT CLAIMED** that a coder **killed with the daemon** can be adopted later (spec 242 rejected adoption); the goal is that the reaper can *find* it.

## Requirements
### Functional
- **R1. Early journalling.** A `coder.spawned` (and any `coder.*`/`worker.start.refused` observation) a running node reports reaches `env.jr` **and is flushed** within `START_DURABLE_WITHIN` of being reported, while the node is still running. It is appended **once**: the post-run drain appends only what the early path has not.
- **R2. Order preserved.** For every run, the journal order of a node's observations is the order the node made them, and they precede that node's `NodeFinished`. A run that finishes normally has a journal **byte-identical in content and order** to today's for a node whose observations all arrive before it returns (a fast node), including the seq numbers — no extra events, no reordering.
- **R3. The reaper finds a live coder.** With a hanging stand-in agent, the **live** store (read from the same engine, mid-run) holds the `coder.spawned` with its pid and argv0 within the window; and `recover_at_boot` on a copy of that store kills the agent and closes the run (the A149 worker's `m2_restart_inside_a_call` and `m7_resume_of_a_run_interrupted_inside_a_call_is_refused` — written, failing — turn green **unchanged**).
- **R4. A fast node costs nothing.** A node that finishes before the window adds no transaction and no event beyond today's (the `deadline_flush::a_fast_node_costs_no_barrier_transaction` test passes unchanged).
- **R5. A control that fails when the fix is absent.** A new test in `pii-graph` (library level, no server): a node that records a `coder.spawned`-shaped observation through its context and then blocks for longer than the window; while it blocks the store's journal holds that observation; after it returns, the journal holds it **exactly once**, before `NodeFinished`.

### Non-Functional
- The change is in `crates/pii-graph/src/sched/` (and `agent_calls.rs` if needed); `dispatch.rs` stays under the 450-line gate (it is 300+ lines: put the new logic in a new file `sched/early_calls.rs`). No change to `journal.rs`'s public API unless the MEASURE FIRST shows it is required — say so in the commit. No new dependency.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test early_agent_calls 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test "$(cargo test -q -p pii-graph --test early_agent_calls -- --list 2>/dev/null | grep -c ': test$')" -ge 4`
- [ ] `cargo test -q -p pii-graph --test deadline_flush 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test composition_failure_matrix 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test orphans_are_reaped 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test -f crates/pii-graph/src/sched/early_calls.rs`
- [ ] `git diff --quiet -- graphs crates/pii-server/src`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `crates/pii-graph/src/sched/early_calls.rs`
- `crates/pii-graph/src/sched/dispatch.rs`
- `crates/pii-graph/src/sched/mod.rs`
- `crates/pii-graph/tests/early_agent_calls.rs`
- `crates/pii-server/tests/composition_failure_matrix.rs`

**Acceptance tests already written (A149, spec 285).** `composition_failure_matrix.rs` (14 cases, 12 green) is committed WITH this spec: its `m2_restart_inside_a_call` and `m7_resume_of_a_run_interrupted_inside_a_call_is_refused` are red today for exactly this defect and turn green unchanged when the fix lands. Do not edit that file or `spec_graph_daemon/mod.rs`.

## Guardrails
- Do NOT weaken, skip or delete `m2`, `m7`, `deadline_flush`, or `orphans_are_reaped`. Do NOT reorder or duplicate a journal event. Do NOT add a poll loop shorter than 250 ms. Do NOT change `graphs/` or `crates/pii-server/src`. Do NOT commit.

## Follow-ups
Spec 242's `orphans_are_reaped.rs` gains a test whose `coder.spawned` comes from a real run (the live path), so the reaper can no longer be certified on a hand-built event.

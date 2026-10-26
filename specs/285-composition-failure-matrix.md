# Spec 285: a composed run survives cancel, crash, concurrency and reuse — through the engine

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composability, level 2 of 4. Measured 2026-10-26: searching every composability test for cancel, resume, restart, concurrency and reuse found **nothing**. A composed run is one graph, one run id, one journal, one cancel token (the design decision for inline expansion), so these should work — but "should" is what the daemon-level gap just disproved. This spec makes each a test, through `Engine`, with a control.

Spec 284 proves the happy run and four refusals. This one covers what happens **to** a run that is inside a called graph.

## Verified Premises
- **CONFIRMED** (read 2026-10-26) the engine surface: `Engine::start_run` (`lib.rs:443`), `Engine::cancel` (`lib.rs:524`), `runs_cancel::cancel_run` / `cancel_run_with` (`engine/runs_cancel.rs:25,38`), `orphans::recover_at_boot` (`engine/orphans.rs:222`), `engine::bus::reattach`.
- **CONFIRMED** existing non-composed coverage to copy the shape from: `cancel_with_kill_stops_a_hung_agent_and_closes_the_run`, `cancel_kill_stops_an_executing_run_and_the_refusal_stays_without_the_flag` (`crates/pii-cli/tests/run_cancel_kill.rs`), `reattach.rs`, and the orphan tests in `engine/orphans.rs`.
- **CONFIRMED** `unsafe_code = "forbid"`: a fake agent reaches the engine through a **child process** whose `PATH` starts with the fake bin dir (see spec 284, "MEASURE FIRST"); reuse whatever helper spec 284 landed rather than writing a second one. If spec 284 has not landed, stop and report.
- **MEASURE FIRST (1)** whether `cancel_run_with(.., kill = true)` reaches a coder that is running **inside an inlined call** — the node ids carry the `docs__` prefix; the kill path may look nodes up by the host's ids only.
- **MEASURE FIRST (2)** what `recover_at_boot` does to a run left `Running` in the store whose agent process is dead: which terminal status it writes and whether `graph.call` stays readable afterwards. Find how an existing test simulates a restart (a second `Engine::new` on the same `store.redb` after dropping the first, or a copied store) and use that; if no in-process simulation is possible, record exactly why in the test file's header and cover it in spec 287 (the real daemon).
- **MEASURE FIRST (3)** whether the engine can resume a run at all. If no resume exists for graph runs, the test asserts the **refusal** (the verb names the run and says it cannot resume) and the spec records it as a documented limit — not a gap to paper over.
- **MEASURE FIRST (4)** whether the loader accepts a `call` inside a `loop` body; the test pins whichever it is (works, or a clean load error naming the call), never a panic.

## Requirements
### Functional
One new test file `crates/pii-server/tests/composition_failure_matrix.rs`, each case a host that calls a small sub-graph whose coder is a fake agent that sleeps or records, started with `Engine::start_run`:

- **M1 cancel inside a call.** Start; wait until the called graph's coder has spawned (journal `coder.spawned` for a `docs__…` node); cancel with kill. Assert: terminal `Cancelled`; the agent's pid (the fake writes it to a file) is **no longer alive**; the journal has exactly one `graph.call`; the node after the call never started. CONTROL: cancelling **without** kill is refused while a node runs, as for any graph (spec `run_cancel_kill.rs`).
- **M2 restart inside a call.** Start a run whose called coder hangs; take the engine down without cancelling; open a new engine on the same store; run `recover_at_boot`. Assert: the run is closed with a terminal status (not left `Running`), the dead agent's process is gone, and the journal read back through the **new** engine still holds its `graph.call` with the same digest. CONTROL: a run that **finished** before the restart is left untouched (same terminal status, same journal length).
- **M3 two concurrent runs of one graph.** Start two runs of the same host with different run inputs while both agents are alive. Assert: two run ids, two journals, each with its **own** `graph.call` and its own input-derived brief in the agent's argv; neither journal contains the other's node events; the two sessions differ. CONTROL: one run cancelled does not change the other's terminal status.
- **M4 one graph called twice.** A host with two `call` nodes naming the same pinned graph, in sequence. Assert: two `graph.call` observations with **different** call ids and the same digest; both sub-runs executed (two coder spawns, two sessions). CONTROL: the host's journal node ids are unique (no collision from the second inlining).
- **M5 budget exhausted inside a called graph.** `[limits]` of 100 000 micro-USD, first coder charges 0.40: the run ends `Failed`, the called graph's coder **never spawns**, and the journal carries the reason (`budget` in the failure text). CONTROL: the same host with the default (no limit) completes.
- **M6 a `call` inside a loop** — pin whichever MEASURE FIRST (4) found.
- **M7 resume** — pin whichever MEASURE FIRST (3) found.
- **M8 needs refused through the engine.** A host whose called graph declares `needs = ["x"]` started **without** `x` → no run is started or the run fails before any node, with `graph.call.need` naming the call, graph and key (spec 267 through `Engine`). CONTROL: with `x` present it runs.

### Non-Functional
- Test-only. Under 450 lines per file (split by case group if needed). Nothing spends. A case that cannot be written (the engine offers no way in) is **reported with the reason in the test file header and the worker's final message**, never skipped silently.
- A failing case is a **finding**: the worker stops, leaves the failing test in the tree, and reports which of M1–M8 failed and what the journal showed. It changes no source.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test composition_failure_matrix 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test "$(cargo test -q -p pii-server --test composition_failure_matrix -- --list 2>/dev/null | grep -c ': test$')" -ge 12`
- [ ] `test "$(grep -ciE "^(async )?fn m[1-8]_" crates/pii-server/tests/composition_failure_matrix.rs)" -ge 8`
- [ ] `grep -q 'recover_at_boot' crates/pii-server/tests/composition_failure_matrix.rs`
- [ ] `grep -qE 'cancel_run_with|\.cancel\(' crates/pii-server/tests/composition_failure_matrix.rs`
- [ ] `cargo test -q -p pii-server --test spec_graph_runs_through_the_daemon 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `git diff --quiet -- crates/pii-server/src crates/pii-graph/src graphs`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit source, graphs or existing tests; do NOT `#[ignore]`, loosen or delete a case to get green; do NOT call `set_var`; do NOT commit.
- A fake agent that sleeps must be killed by the test's cleanup even when an assertion fails (a leaked sleeper holds the build lock and the disk).

## Follow-ups
Spec 286 (mutation sweep over the composition code), spec 287 (the real devbox daemon process).

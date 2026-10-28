# Spec 306: every `call` has an entry counter, so no subgraph can be entered without bound

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composability, loop safety (user request 2026-10-28: "add a counter to every subgraph to avoid infinite loops")
A `call` is inlined at load (`<call>__in` head plus the sub-graph's nodes). Today a call inside a cycle is bounded only by the cycle's own `loop_budget` (rule V3, at least one member must carry one) and by the whole run's `max_steps` (1000) and `max_wall_ms`. Every pass through a call can spawn a real agent and spend money, so "1000 steps" is not a bound anyone wants. MEASURED: `composition_failure_matrix.rs` m-looped runs a call inside a loop that ends on its budget, so the loader accepts a call in a loop, and only the loop's own node limits it.

## Requirements
### Functional
- R1: a `call` node accepts an optional integer `max_entries` (>= 1). The default, when absent, is **8** (Opus review: no shipped graph enters a call more than once, and 8 covers a 2 x 3 nested retry while halving the worst case). A call is *entered* each time its head `<call>__in` executes.
- R2: the (`max_entries`+1)-th entry ends the run `Failed` with a reason that names the call and the limit: `call 'docs' entered more than max_entries (3) times`. The N-th entry still runs. A call that is entered once is unaffected at `max_entries = 1`.
- R3: the counter is per EXPANDED call instance (ids are namespaced `<call>__<n>`), cumulative for the run, never reset by an enclosing loop (resetting per parent entry would multiply the limits, 16^4). It counts COMPLETIONS of the head `<call>__in`, in `apply_completion`, so `replay` rebuilds it on resume with no new journal field. The check is made at DISPATCH of the (N+1)-th entry; the refused entry journals nothing. Do NOT reuse the `loop_budget` `if it == b` arm (`sched/state.rs:233`): it fails the node on its LAST allowed pass.
- R3b: the refusal names the FULL path (`call 'outer__inner'`). Document that an inner retry loop inside a looped outer call reaches its own limit early; the author raises the inner limit explicitly.
- R4: `CALL_KEYS` in `crates/pii-graph/src/fragment/call.rs` gains `max_entries`; a non-integer, zero or negative value is refused at load with `graph.call.max_entries: ...`.
- R5: no shipped graph changes: `graphs/spec.toml` calls `docs` once and must run unchanged. The `spec` graph's digest pin and `lane-input-matches-graph` stay green.

### Non-Functional
- Rust, clippy and fmt at zero; files under 450 effective lines; reuse the existing `loop_budget` machinery (`sched/state.rs`) rather than a second counter if it can express R2 exactly, and say in the PR text which you chose and why.

## Verified Premises
- **CONFIRMED** `crates/pii-graph/src/fragment/call.rs:28` `MAX_DEPTH = 4`, `:31` `CALL_KEYS` has six keys and no counter.
- **CONFIRMED** `crates/pii-graph/src/sched/state.rs:195-240` `loop_budget` semantics: the budget-th execution completes and the run then fails (budget=3 means exactly 3 runs); a cycle with no budgeted member is refused by V3 (`validate.rs:274`).
- **CONFIRMED** a call inside a loop loads and runs (`composition_failure_matrix.rs`, looped host with `loop_budget = 2` on `again`).
- **INFERRED** reusing `loop_budget` on the head would fail a call on its LAST allowed entry (budget-th execution fails); R2 says the N-th entry runs and the next fails, so a plain reuse is wrong unless adjusted. The first TDD step settles this with a test before any change.

## TDD Contract
| Test (crates/pii-graph/tests/call_max_entries.rs) | Input | Expected |
|---|---|---|
| `a_call_in_a_loop_stops_after_max_entries` | host loops a call 10 times, `max_entries = 3` | exactly 3 entries ran, the run is Failed, the reason names the call and `3` |
| `CONTROL_a_loop_that_needs_two_entries_passes` | the same host, loop ends after 2, `max_entries = 3` | the run Succeeds |
| `the_default_is_eight` | a call in a V3-valid loop whose own `loop_budget` is ABOVE 8, no `max_entries` | 8 entries, then Failed naming 8 (without the higher loop budget the loop budget fires first) |
| `a_single_call_is_fine_at_one` | `max_entries = 1`, no loop | Succeeds |
| `a_bad_value_is_refused_at_load` | `max_entries = 0`, `-1`, `"x"`, `1.5` | each refused with `graph.call.max_entries` |
| `BOUNDARY_exactly_max_entries_succeeds` | a loop that exits after EXACTLY 3 entries, `max_entries = 3` | the run SUCCEEDS (kills the off-by-one that `== N` semantics would introduce; the other rows do not) |
| `the_count_survives_a_resume` | enter twice, stop the run by TRUNCATING the journal and replaying it (not a timer), resume | the third entry is counted as the third |
| `a_nested_call_names_its_full_path` | an inner call exceeds its limit inside an outer call | the reason contains `outer__inner` |
| `an_enclosing_loop_does_not_reset_the_count` | an outer loop re-enters a call, each time once | the count is cumulative and the limit is reached |
| `two_calls_count_apart` | two call nodes of the same graph in one loop | each has its own count |
| `crates/pii-server/tests/composition_failure_matrix.rs` (existing looped case) | unchanged | still passes |

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test call_max_entries 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `test "$(grep -c '^#\[test\]' crates/pii-graph/tests/call_max_entries.rs)" -ge 10`
- [ ] `cargo test -q -p pii-server --test composition_failure_matrix 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `cargo test -q -p pii-graph 2>&1 | grep -c "test result: ok" | grep -qv '^0$'`
- [ ] `grep -q 'max_entries' crates/pii-graph/src/fragment/call.rs`
- [ ] `bash scripts/autonomy/tests/lane-input-matches-graph.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `cargo clippy -q -p pii-graph -p pii-server --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `git diff --quiet -- graphs scripts`

## Files
- `crates/pii-graph/src/fragment/call.rs`
- `crates/pii-graph/src/fragment/call_needs.rs`
- `crates/pii-graph/src/sched/state.rs`
- `crates/pii-graph/src/run/engine.rs` — the dispatch-time refusal R3 requires (added by the orchestrator after the first
  worker run: the original list omitted it, so a correct change to it read as `unlisted-changes`).
- `crates/pii-graph/tests/call_max_entries.rs`

## Guardrails
- Do NOT edit `graphs/`, `scripts/`, an existing assertion, or the digest of `graphs/docs.toml`. Scratch under `_tmp/` inside the repository.
- The appearance-only implementation: a lint that refuses loops around calls (nothing runs, nothing is counted) or a counter that counts parses instead of executions. The tests above run real loops and count real entries.
- Mutations the tests must kill: the default removed (unbounded); the limit off by one in either direction (the BOUNDARY row is what kills the `== N` form); the count reset on resume; one counter shared by two calls; the check applied at load instead of at execution; a refusal that does not name the call.

## Closure
Reviewed by the orchestrator on the tree the worker left (session `1b9282e2`), not on a report:

- **9 of 9 exit criteria green**, run from the row's own `verify` column (the column equals `## Exit Criteria` command for
  command, 9 of 9 counted programmatically); `call_max_entries.rs` holds the required 10 tests.
- **The refusal is load-bearing where R3 says it must be.** Mutation: `crates/pii-graph/src/run/engine.rs` restored to
  HEAD (the dispatch-time `entry_refusal` check removed, `state.rs` untouched) reds 5 of the 10 tests —
  `a_call_in_a_loop_stops_after_max_entries`, `the_default_is_eight`, `an_enclosing_loop_does_not_reset_the_count`,
  `a_nested_call_names_its_full_path`, `the_count_survives_a_resume` — each failing on the enclosing `loop_budget`
  because the entry limit no longer fires first. Restored from the saved copy and `touch`ed:
  `sha256 ca63260f96e5b02cd430f90b2f710834db6d49a13e97c9094704412f84a2ca91` before and after, then `10 passed; 0 failed`.
- **`fresh-gate --worktree`: `quality-gate: ALL PASS`** over HEAD plus the reviewed files.
- **Spec defect found and repaired here, not worked around by the worker.** The first run stopped with
  `unlisted-changes: crates/pii-graph/src/run/engine.rs`: R3 requires the check at DISPATCH of the (N+1)-th entry, and
  the dispatch loop lives in `run/engine.rs`, which the `## Files` list omitted. The change was correct; the list was
  short. It is now named in `## Files` and in the row's commit-path column (`run/engine.rs` added, `call_needs.rs`
  kept as declared).
- Not done here: nothing in `graphs/` changed (R5 holds — criterion 9 and `lane-input-matches-graph` both green).


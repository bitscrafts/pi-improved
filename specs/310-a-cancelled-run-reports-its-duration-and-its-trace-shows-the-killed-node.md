# Spec 310: a cancelled run reports how long it ran, and its trace shows the node that was killed

## Overview
**Project**: /projects/pi-improved
**Phase**: production — observability of composed runs (user request 2026-10-28: "when using it with other graphs it will be easy to debug, take into consideration the observability")

MEASURED 2026-10-28 on run `r000000000580`, a three-level call chain cancelled while its real leaf agent worked (leaf pid 971276, alive before the cancel, gone after):
- `pii run explain` says `failed in 0ms: cancelled: chain-test (node 'via2__via1__docs__docs' abandoned)`, and `pii run get` says `"dur_ms":0`. The cause is named; the duration is wrong by the whole time the agent ran.
- `pii run trace` lists the call levels and the nodes before the leaf, then `trace: 7 nodes · 1 agent calls · 0 orphans`. The leaf node, the one the operator wants, has no line, although `pii run get` shows it (`spawned=971276 exited=-1`).

CAUSE (read in the code): `close_failed` (`crates/pii-server/src/engine/runs.rs`) closes a run that is not in flight by appending a `Finished` event to the header as read from the store; `RunHeader.dur_ms` is only ever set by the run's own `finish` (`crates/pii-graph/src/run/engine.rs`), so a run closed from outside keeps `dur_ms: 0`. `node_lines` (`crates/pii-cli/src/cmd_trace/analysis.rs`) emits a line only on `NodeFinished` or `NodeFailed`, so a node that has `NodeStarted` and was then abandoned never gets one.

## Requirements
### Functional
- R1: a run closed by `close_failed` (cancel, abandon, boot recovery) records `dur_ms` as the milliseconds from the run's creation (`created_ms`) to the close, taken from the engine's clock. A header whose `dur_ms` is already non-zero is left as it is.
- R2: `node_lines` emits a line for every node that has a `NodeStarted` event and no `NodeFinished` or `NodeFailed` by the end of the journal, in journal order, with `failure = Some("abandoned: started and never finished")` and the agent calls observed on it. The summary counts these nodes.
- R3: the closed-run line of `pii run explain` prints the real duration. No change to its wording otherwise.
- R4: no event kind, header field or wire name changes; the journal of a finished run is unchanged.

### Non-Functional
- Rust, clippy and fmt at zero; files under 450 effective lines; the existing `run_trace_names_the_call` and `cancel_kill` suites stay green and untouched.

## Verified Premises
- **CONFIRMED** `RunHeader` has `dur_ms`, `created_ms`, `process_start_ms` (`crates/pii-abi/src/journal.rs`); `Store::stamp_origin` sets `created_ms` at acceptance (`crates/pii-store/src/lib.rs:198`).
- **CONFIRMED** `finish` sets `dur_ms` from `self.root.elapsed_ms()` (`run/engine.rs:375`); `close_failed` does not touch it (`runs.rs:307-340`).
- **CONFIRMED** `node_lines` pushes only on `NodeFinished` and `NodeFailed` (`analysis.rs:170-200`).
- **MEASURED** the live symptoms above on `r000000000580`.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| `crates/pii-server/tests/cancelled_run_has_a_duration.rs` `a_cancelled_run_reports_its_duration` | a run registered and left running ~300 ms, then `cancel_run` | the stored header's `dur_ms` is at least 250 |
| same file `CONTROL_a_finished_run_keeps_its_duration` | a run that finishes on its own after ~100 ms | `dur_ms` equals what `finish` recorded (not overwritten by the close path) |
| same file `a_header_that_already_has_a_duration_is_left_alone` | `close_failed` on a header with `dur_ms = 7` | still 7 |
| `crates/pii-cli/tests/trace_shows_the_abandoned_node.rs` `an_unfinished_node_has_a_trace_line` | a journal with `NodeStarted` and `coder.spawned` for `leaf`, then `Finished` (run closed), no `NodeFinished` | a line for `leaf` with its pid and the text `abandoned` |
| same file `CONTROL_a_finished_node_has_one_line` | the existing finished-node journal | exactly one line for that node, no `abandoned` text |
| same file `the_summary_counts_the_abandoned_node` | the abandoned journal | the summary's node count includes `leaf` |

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test cancelled_run_has_a_duration 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `cargo test -q -p pii-cli --test trace_shows_the_abandoned_node 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `test "$(grep -c '^#\[test\]\|^#\[tokio::test\]' crates/pii-server/tests/cancelled_run_has_a_duration.rs)" -ge 3`
- [ ] `test "$(grep -c '^#\[test\]' crates/pii-cli/tests/trace_shows_the_abandoned_node.rs)" -ge 3`
- [ ] `cargo test -q -p pii-cli --test run_trace_names_the_call 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `cargo test -q -p pii-server --test cancel_kill 2>&1 | tail -3 | grep -q "test result: ok"`
- [ ] `cargo clippy -q -p pii-server -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `git diff --quiet -- graphs`

## Files
- `crates/pii-server/src/engine/runs.rs`
- `crates/pii-cli/src/cmd_trace/analysis.rs`
- `crates/pii-server/tests/cancelled_run_has_a_duration.rs`
- `crates/pii-cli/tests/trace_shows_the_abandoned_node.rs`

## Guardrails
- Do NOT edit `graphs/`, `scripts/`, an existing assertion, or any journal event or header field definition. Scratch under `_tmp/` inside the repository.
- The appearance-only implementation: a trace line printed from the header instead of the journal, or `dur_ms` computed in `explain` only (the stored header would still say 0). The tests read the stored header and the journal.
- Mutations the tests must kill: the duration set only when the run was cancelled (not abandoned); the duration overwritten on a finished run; the abandoned line emitted for a node that DID finish; the abandoned node left out of the node count.

# Spec 293: a run that fails on a limit says which limit

## Overview
**Project**: /projects/pi-improved
**Phase**: production — observability of terminal failures. Measured 2026-10-27 on the production devbox daemon: run `r000000000386`
(the shipped `spec` graph with its `max_wall_ms` removed, two real claude coders, both exited 0, 888 774 ms) ended `failed`. The engine
journalled the reason (`crates/pii-graph/src/run/engine.rs:72-78`, `Terminal::Failure { error: "max_wall_ms exceeded" }`), but `pii run get`,
`pii run explain` and `pii run trace` print only `failed in 888774ms` and the two agents' successful exits. An operator cannot tell a wall-limit
stop from a crash, and the same silence covers `max_steps exceeded` and `budget exhausted: Tokens` (engine.rs:64-90, 90+). The unattended loop
reads these commands, so a wall stop looks like a worker defect.

## Requirements
### Functional
- R1: for a run whose terminal is `Terminal::Failure`, `pii run get <id>` adds a `reason` field with the error text, and `pii run explain <id>` ends its
  sentence with it (`... failed in Nms: max_wall_ms exceeded.`). A run that succeeded or was cancelled prints no `reason`.
- R2: the three engine limit stops (`max_steps exceeded`, `max_wall_ms exceeded`, `budget exhausted: <kind>`) each reach the output verbatim.
- R2b: a judge node's result is shown too: `run trace` prints `crit=<passed>/<total>` and the failing criteria's text on its row (today it prints `crit=-` for a judge node, so an operator cannot see that 4 of 5 passed). Measured on r000000000388.
- R3: no field is added to a succeeded run; the existing output of every other command is unchanged byte for byte.

### Non-Functional
- Read the reason from the journal's terminal event; do not recompute it. No new dependency. Each file stays under 450 effective lines.

## Verified Premises
- **CONFIRMED** (read 2026-10-27) `engine.rs:72-78` finishes with `Terminal::Failure { error: "max_wall_ms exceeded" }` and `RunStatus::Failed`.
- **CONFIRMED** on r000000000386: `run get` printed `{"dur_ms":888774,...,"status":"failed"}` and `run explain` printed no reason.
- **TO BE READ BY THE WORKER** where `run get` and `run explain` are built, and which event carries the terminal.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| `a_wall_stop_names_the_wall` | a graph with `max_wall_ms = 1` and a node that sleeps past it | `run get` has `"reason":"max_wall_ms exceeded"`, `explain` ends with it |
| `a_step_stop_names_the_steps` | `max_steps = 1`, two nodes | reason `max_steps exceeded` |
| `a_budget_stop_names_the_budget` | the spec 276 exhausted-budget graph | reason starts `budget exhausted` |
| `CONTROL_a_succeeded_run_has_no_reason` | a passing run | no `reason` key, explain unchanged |
| `CONTROL_a_cancelled_run_has_no_reason` | a cancelled run | no `reason` key |

## Exit Criteria
- [ ] `cargo test -q -p pii-cli --test run_failure_reason 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test "$(cargo test -q -p pii-cli --test run_failure_reason -- --list 2>&1 | grep -c ': test')" -ge 5`
- [ ] `cargo test -q -p pii-cli --test graph_explain_prints_calls 2>&1 | grep -qE "^test result: ok\."`
- [ ] `cargo test -q -p pii-cli --test trace_prints_calls 2>&1 | grep -qE "^test result: ok\."`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- crates/pii-cli/tests/run_failure_reason.rs

## Guardrails
- Do NOT change what a succeeded or cancelled run prints. Do NOT weaken or edit an existing assertion.
- Mutations to kill: reason always empty, reason printed for a succeeded run, reason printed only for wall stops.

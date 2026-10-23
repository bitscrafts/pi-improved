# Spec: 239 — a node whose agent changed nothing fails only when its criterion already passed before the agent ran

**Project**: /projects/pi-improved
**Phase**: filed 2026-10-23T10:30Z by the orchestrator (defects 7 and 11 in `_docs/05-operations/agent-management-findings-2026-10-23.md`).
Premises read, not assumed:

- `crates/pii-server/src/node_kinds/nodes/coder/graph_node.rs` (202 lines) runs the agent, then the criteria
  (`for (command, argv) in checks`, lines ~122-134), then takes `tree_after = tree::digest(...)` (line ~135), builds
  `RunReport`, emits `coder.exited`, and decides with `decide(code, "", &runs)`. `tree_before` is taken at line ~85,
  before the agent.
- `crates/pii-server/src/node_kinds/nodes/coder/tree.rs` has `digest(&Path) -> TreeDigest`; `RunReport.tree_fields`
  journals `tree_changed` as `yes`/`no`/`unknown`.
- `agent_worker::decide(worker_rc, report, criteria) -> Verdict` returns `Failed { reason, failing }` or `Passed`.
- `criteria` run through `ctx.proc()?.run(&argv, &workdir, CRITERION_TIMEOUT)`.

## Overview

A coder node succeeds today when the agent exits 0 and every criterion passes, even when the agent changed nothing
(measured: stand-in mode `noop` succeeded; `tree_changed: no` is journalled but nothing reads it). That is right when
the spec was already satisfied and wrong when the criterion could not tell: the agent did nothing and the node
certified work that never happened.

The two cases differ only in what was true BEFORE the agent ran. If the criteria already passed on the untouched tree,
a no-change run proves nothing; if they failed before and pass now, something changed, so `tree_changed` cannot be `no`.
So the refusal is: the tree is unchanged, the agent exited 0, and the criteria passed on the tree as the agent found it.

## Requirements

- **R1** Before the agent is spawned, when the node has criteria and `tree_before` is a real digest, run the criteria once
  (same argv, same workdir, same `CRITERION_TIMEOUT`) and record whether ALL passed as `pre_passed`.
- **R2** After the agent and the criteria, when `decide` would pass AND `tree_changed` is `no` AND `pre_passed`, the node
  returns `Outcome::Fail` with reason `no change: the agent changed nothing and the criteria already passed before it ran`.
- **R3** Every other combination keeps today's verdict: tree changed; tree `unknown`; criteria failed before the agent;
  agent failed; node has no criteria (the deferred verdict).
- **R4** `coder.exited` gains `pre_criteria_passed` (`yes`/`no`/`skipped`). `skipped` when no criteria or no digest.
  Existing fields keep their names and meaning.
- **R5** The pre-run costs one extra criterion execution per node; it is not run when the node has no criteria or when
  `tree_before` is `unknown`.

## Architecture

A pure decision function, so the rule is testable without spawning anything:
`pub fn no_change_refusal(pre_passed: Option<bool>, tree_changed: &str, verdict_passed: bool) -> Option<String>` in a new
file `crates/pii-server/src/node_kinds/nodes/coder/no_change.rs`, registered as `mod no_change;` (or `pub mod`, matching
how `tree`/`redact` are registered in `coder.rs`). `graph_node.rs` calls it between `decide` and the return.

Rejected: failing on every `tree_changed = no` (wrong for an already-satisfied spec, which is a legitimate no-op); leaving
it to a `judge` node (the judge reads criteria, which cannot see this).

## TDD Contract

New test file `crates/pii-server/tests/coder_no_change.rs`. The decision table is tested on the pure function; one
integration test drives the real node with the stand-in style fake `pi` the neighbouring tests already use (read
`coder_spawns_a_process.rs` and follow its construction; the fake must be a script under `_tmp/`).

| test | given | expects |
|---|---|---|
| `no_change_with_criteria_already_passing_is_refused` | pre `Some(true)`, tree `no`, verdict passed | `Some(reason)` containing `no change` |
| `a_changed_tree_is_not_refused` | pre `Some(true)`, tree `yes`, verdict passed | `None` (CONTROL) |
| `criteria_that_failed_before_the_agent_are_not_refused` | pre `Some(false)`, tree `no`, verdict passed | `None` (CONTROL) |
| `an_unknown_tree_is_not_refused` | pre `Some(true)`, tree `unknown`, verdict passed | `None` (CONTROL) |
| `no_pre_run_means_no_refusal` | pre `None`, tree `no`, verdict passed | `None` (CONTROL) |
| `a_failed_verdict_is_left_alone` | pre `Some(true)`, tree `no`, verdict failed | `None` (CONTROL) |
| `the_node_fails_when_the_agent_writes_nothing_and_the_criterion_already_passes` | fake agent that exits 0 and writes nothing, criterion `true` | node outcome is `Fail`, reason names `no change` |
| `the_node_passes_when_the_agent_writes_what_the_criterion_needs` | fake agent that creates a file the criterion tests, criterion `test -f <file>` | node outcome is `Next` on `ok` (CONTROL) |
| `the_event_says_whether_the_criteria_passed_before` | the passing node above | `coder.exited` has `pre_criteria_passed` = `no` |

The requirement a plausible implementation gets wrong is R2's condition: refusing on `tree_changed = no` alone fails an
already-satisfied spec, and running the pre-check AFTER the agent (instead of before) always sees the criterion pass. The
CONTROL rows and the last two integration rows catch both.

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test coder_no_change 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `grep -q "pre_criteria_passed" crates/pii-server/src/node_kinds/nodes/coder/report.rs`
- [ ] `grep -q "no_change_refusal" crates/pii-server/src/node_kinds/nodes/coder/graph_node.rs`
- [ ] `cargo test -q -p pii-server --test coder_spawns_a_process 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-server --test coder_tree_digest 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`

## Guardrails

- `specs/` are READ-ONLY for the implementer. If `graph_node.rs` does not have the shape described (no `tree_before`, no
  `decide` call, no `RunReport`), STOP and report the mismatch instead of forcing the change.
- Never weaken an existing test. `coder_spawns_a_process` and `coder_tree_digest` must stay green unchanged. If an existing
  test fails because the pre-run now executes its criterion, report it: the test's criterion is observable and that is a
  finding, not something to loosen.
- The pre-run uses the SAME argv and `CRITERION_TIMEOUT` as the post-run; do not add a second timeout or a different
  working directory.
- Do not change `decide`, `deferred_verdict`, `stop_reason`, or any field already on `coder.exited`.
- Tests write under `_tmp/`. 450-effective-line file gate and `#![forbid(unsafe_code)]` apply; `graph_node.rs` is 202 lines.
- Do NOT commit; the orchestrator commits.

## Closure

_(empty — the implementer's readings go here)_

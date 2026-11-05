# Spec: the pool and cancellation tests finish, and a test that hangs is not a test

## Overview
Five tests in `crates/pii-cli/tests/control_plane_is_not_behind_runs.rs` hang: they sit past their own
60-second warning and past the 90-second `until` budget, so the gate reports them as failures after
holding the whole run for many minutes. They hang **alone**, and they hung on a clean HEAD before any of
this session's work — so this is the test family, not a regression.

Measured 2026-11-05: `CONTROL_at_most_run_threads_agents_are_alive` alone, in a fresh export, still
running at 60 s; the same five names red in the gate; the same names red at HEAD in a separate worktree.

Why it matters beyond the five: the gate cannot go green while they hang, so every row is blocked behind
them. A hanging test is worse than a failing one: it spends the wall clock and reports nothing.

**Project**: /projects/pi-improved
**Phase**: operations (test infrastructure)

## Requirements
### Functional
- R1 each of the five reaches a verdict (pass or fail) inside its own budget, alone and in the suite.
- R2 the daemon each one spawns is owned by a `DaemonGuard` and cannot outlive the test (spec 331).
- R3 the heavy-serialising lock checks its OWNER, not just its age: a killed run must not make the next
  one wait fifteen minutes (the lock writes a pid and never reads it).
- R4 a test that exceeds its budget FAILS with the condition it was waiting for named, instead of
  blocking the suite.

### Non-Functional
- R5 the five together finish in under two minutes on this host.

## TDD Contract
| Test name | Input | Expected |
|---|---|---|
| `a_dead_lock_holder_is_reaped_at_once` | lock file naming a dead pid | taken in < 1 s |
| `a_live_lock_holder_makes_the_waiter_report_it` | lock filed by a live pid | the waiter names the pid and its age |
| `every_pool_test_reaches_a_verdict_alone` | each of the five, alone | pass or fail inside its budget |

## Exit Criteria
- [ ] `timeout 240 cargo test -q -p pii-cli --test control_plane_is_not_behind_runs 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `grep -q '/proc/' crates/pii-cli/tests/control_plane_is_not_behind_runs.rs`
- [ ] `timeout 240 cargo test -q -p pii-cli --test control_plane_is_not_behind_runs >/dev/null 2>&1`
- [ ] `timeout 240 cargo test -q -p pii-cli --test heavy_lock_checks_its_owner 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`

## Guardrails
- Every criterion is wrapped in `timeout 240`. A criterion that hangs must FAIL in four minutes, not sit
  for the judge's ten-minute ceiling: this row's own contract phase spent twenty minutes hanging on the
  very tests it exists to unstick (measured 2026-11-05, run `r000000001474` inside `sdd__contract`).
- Do NOT raise the 90-second budget to make them pass: the budget is not the defect.
- Do NOT delete a test because it is slow; the property it asserts (the control plane is not queued
  behind runs) is a product promise.
- Do NOT make the suite skip them: a skipped test reads as a pass in every summary line.

## Files
- `crates/pii-cli/tests/control_plane_is_not_behind_runs.rs`

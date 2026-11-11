# Spec: a lock checks its owner, not its age

## Overview
Tonight's blocker, measured. `crates/pii-cli/tests/control_plane_is_not_behind_runs.rs` takes a lock at
`_tmp/control-plane-heavy.lock` and writes its pid into it, then **never reads the pid**. A killed run
leaves the file behind, and the next run waits its full fifteen-minute budget before asserting. That is
not slowness: it is a lock with no owner check, and it is indistinguishable from a hang.

Consequence measured on 2026-11-04/05: the gate's test step "hung", the cycle's gate never completed, the
row was parked after three attempts, and parking QUARANTINES THE TREE — three batches of work were swept
mid-flight. The defect is in the test harness, and it cost a night.

**Project**: /projects/pi-improved
**Phase**: operations (test harness)

## Requirements
### Functional
- R1 the lock records `pid` and reads it back: a holder whose process is gone is reaped AT ONCE, not
  after 900 s of mtime.
- R2 the mtime rule stays as a fallback for a pid that exists but is not this test's (a recycled pid).
- R3 a wait that gives up says which pid held the lock and for how long, so the next person reads a
  diagnosis instead of a hang.

### Non-Functional
- R4 the same defect is swept from every test that takes a lock: `grep -rn 'control-plane-heavy\\|_tmp/.*\\.lock' crates/` finds them all in this row, not in a later one.

## TDD Contract
| Test name | Input | Expected |
|---|---|---|
| `a_dead_holder_is_reaped_immediately` | a lock file naming a pid that does not exist | taken in < 1 s |
| `a_live_holder_makes_the_test_wait` | a lock file naming this test's own pid | waits, then reports the pid |
| `CONTROL_no_lock_file_still_works` | no file | taken |

## Exit Criteria
- [ ] `cargo test -q -p pii-cli --test control_plane_is_not_behind_runs a_dead_holder_is_reaped_immediately 2>&1 | grep -qE 'test result: ok'`
- [ ] `grep -q '/proc/' crates/pii-cli/tests/control_plane_is_not_behind_runs.rs`
- [ ] `cargo test -q -p pii-cli --test control_plane_is_not_behind_runs 2>&1 | grep -qE 'test result: ok'`

## Guardrails
- Do NOT raise the fifteen-minute budget: the budget is not the defect, the missing owner check is.
- Do NOT delete a lock file blindly on start: a live holder must still be respected.

## Files
- `crates/pii-cli/tests/control_plane_is_not_behind_runs.rs`

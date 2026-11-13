# Spec: the heavy-test lock waits for a holder that keeps its heartbeat, and fails only one that goes silent
#: depends=415 status=pending block=-

## Overview
Measured 2026-11-13 in the gate for row 415 (`_tmp/autonomy/logs/cycle-20261113T021943Z.log`): `test (lib/bins/tests)` failed with **5 of the 6 tests in
`crates/pii-cli/tests/control_plane_is_not_behind_runs.rs` failing, none of them having run** —
`another heavy test has held …/control-plane-heavy.lock for 120s (holder pid Some(3705409), File exists (os error 17))`, from
`crates/pii-cli/tests/heavy_lock/mod.rs:64`. The same file passes 3 of 3 alone in 28–34 s at load 0.8, and the first landing of row 415 passed with the
identical tree once the load was lower. The six tests take turns with `heavy_lock`, whose own doc says the pid is the check and age is only the fallback
for a pid `/proc` cannot see — but `take` still fails a **live** holder once a flat `give_up` (2 min at the one call site) has elapsed.

A live holder is the one case a wall clock cannot judge: it may be slow because the gate runs every other test binary beside it, or it may be stuck.
Its own progress is what distinguishes them. This row makes the holder prove it is progressing, and keeps `give_up` as the budget for *silence*.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 415

## Requirements
- R1 A `HeavyGuard` proves it is progressing while it is held: a background thread refreshes the lock file's modification time every `HEARTBEAT`
  (2 s) using `File::set_modified`, which changes no content and cannot race a reader. It stops when the file no longer names this process, and when
  the guard is dropped. A heartbeat that cannot touch the file is ignored: the lock is no longer ours. `Drop` joins the thread, then removes the file.
- R2 `take` uses the file's age as the **silence** measure, not as the total wait: a holder whose mtime is younger than `give_up` is waited for however
  long it has held the lock, and one whose mtime is older is not. The panic message keeps the holder's pid, the budget and the age it waited.
- R3 Everything else about the lock is unchanged: the pid is written before the guard is returned; a dead holder is reaped immediately; a file with no
  readable pid is reaped only when it is older than 900 s; the file is removed on `Drop`; no new dependency and no `unsafe`.
- R4 The one call site (`only_one_heavy_test_at_a_time`, `Duration::from_mins(2)`) keeps its budget and gains a comment saying what the budget now
  measures, so no test is weakened or skipped to make this pass.
- R5 A holder that goes silent is still failed inside `give_up`, and the message names the pid: the near-miss of R1 must not turn a stuck holder into
  an eternal wait.

## Architecture
```
take(path, give_up)                       HeavyGuard::new(path)
  loop:                                    spawn: every 2 s
    create_new -> write pid -> guard         if the file still names self: set_modified(now)
    holder gone?      -> remove, retry       else: stop
    age(mtime) > give_up? -> panic           on Drop: stop the thread, wait for it, remove the file
    else sleep 50 ms
```
Key decision: progress, not elapsed time, is what a waiter can judge. Rejected: raising `give_up` (moves the cliff; the next busy gate hits it again).
Rejected: dropping the deadline for live holders (a genuinely stuck holder would then hang the whole test step with no name). Rejected: serialising the
suite through `--test-threads` (a global change to every test, to fix one file).

## TDD Contract
Added to `crates/pii-cli/tests/heavy_lock_checks_its_owner.rs` (91 lines; both files stay under 450).
| Test Name | Input | Expected Output |
|---|---|---|
| `a_holder_that_keeps_its_heartbeat_is_waited_for_past_the_budget` | one guard held for 3× `give_up` in a thread, second caller with `give_up` = 1 s | the second caller acquires the lock without panicking, after the first is dropped |
| `a_heartbeat_stops_when_the_lock_no_longer_names_this_process` | the file is rewritten with another pid while a guard is held | the guard's heartbeat stops touching the file: its mtime stops advancing (asserted over 3 heartbeats) |

Acceptance: the four existing tests in that file pass **unedited**, and each of these mutations makes at least one fail (the mutation script is the
orchestrator's): the heartbeat removed (the long-held case fails at `give_up`); the age check made age-blind (the silent holder is waited for forever);
the heartbeat thread not joined before the file is removed (the guard leaves a file behind, which the next test sees).

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-cli --test heavy_lock_checks_its_owner 2>&1 | grep -qE 'test result: ok. [5-9] passed'`

## Guardrails
- Keep green, unedited: the four tests already in `heavy_lock_checks_its_owner.rs`, and
  `cargo test -q -p pii-cli --test control_plane_is_not_behind_runs` alone (6 passed in ~30 s on an idle machine).
- Do NOT change `give_up` at the call site, do NOT skip a test, and do NOT lower a budget anywhere else.
- No new dependency, no `unsafe`, no `std::sync::Mutex` (banned in this crate by `clippy.toml`; the guard's stop flag is an `AtomicBool`).
- Both Rust files stay under 450 lines. Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-cli/tests/heavy_lock/mod.rs`
- `crates/pii-cli/tests/heavy_lock_checks_its_owner.rs`
- `crates/pii-cli/tests/control_plane_is_not_behind_runs.rs`

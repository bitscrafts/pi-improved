# Spec: 196 — a cancelled run's timer does not fire it

**Project**: /projects/pi-improved · **Phase**: production blocker found by the 09:00Z evidence, 2026-10-19
**Filed**: 2026-10-19 by the orchestrator, from the production log, not from a theory.

## Overview

MEASURED 2026-10-19 09:00:01.923–09:00:02.537Z on the deployed daemon (`121fb6de67e1`): **four digest runs
that had been CANCELLED the day before executed and each posted a 94-byte digest to the Discord forum.**

| fact | evidence |
|---|---|
| the four runs were cancelled on 2026-10-18 (`--reason "stale duplicate"`) | `run.cancelled` ×4 in `pii.log`; `EXERCISE-RESULTS-2026-10-19.md` (digest-timing correction) |
| at 09:00:01–02Z today all four posted | `channel.posted` ×4, fields `run=r000000004129/4161/4193/4225`, `bytes=94`, `destination=1558589728671146105` |
| all four are now `succeeded`, `dur_ms` 1.9–2.5 s, `seq` 25 | `pii run ls` (measured 09:05Z) |
| the only suspended run is r000000004353 (12:00Z) | `pii run ls --status suspended` |
| their wheel keys were in the wheel until they fired | `strings /run/user/0/pii/timer.redb` before 09:00Z: `timer:r000000004129:tick:1792400400000` … ×4 |

Three facts in the tree explain it:

- `crates/pii-server/src/engine/runs.rs:306-347` `cancel_run` closes the run (`close_failed`, status
  `Failed`, `cancelled: <reason>`) and emits `run.cancelled`. It never touches the timer wheel.
- `crates/pii-store/src/timer.rs` exposes `put_timer`, `next_timer`, `take_due` — and **no removal**. There
  is no way for the cancel path to disarm a schedule, so the entry survives.
- `crates/pii-server/src/daemon/scheduler.rs:87-91` takes due entries and calls
  `spawn_resume(...)` with `let _ =` — a refused resume is silent, and nothing checks that the run's header
  is still resumable before the wheel's own transaction has already consumed the entry.

Consequence: **`pii run cancel` does not cancel the schedule.** A cancelled run's timer fires it anyway, and
the resume path re-runs a terminal run. This morning it produced four duplicate digest posts in the user's
forum — the same class as the 09:00 multiplication (an armed copy nobody can see), one layer down.

**Second, smaller measurement (recorded, not fixed here):** the four posts are invisible to `since_deploy` —
those runs were created before spec 188 and carry an empty revision, so they land in `unattributed` (1850)
and `posted` reads 0 while four posts went out. 070 §5's reader is keyed by revision by design; an
*unattributed* run's post is nobody's deploy. Not this spec's fix (it cannot attribute history), but the
operator must know a zero `posted` does not mean "no post left the host" when `unattributed > 0`.

## Requirements

- **R1 — the wheel can disarm.** `pii_store::timer::TimerWheel` gains `remove_run(run: &str) -> Result<u64,
  PiiError>`: delete every entry whose run id is `run`, in ONE write transaction, returning the number
  removed. It is the missing verb the cancel path needs; it never touches other runs' entries.
- **R2 — cancel disarms.** `cancel_run` calls `remove_run(run_id)` after `close_failed` succeeds and before
  the `run.cancelled` event, and reports the count in that event's fields (`disarmed`). A run with no timer
  reports `disarmed=0` (not an error: most runs have none).
- **R3 — a resume of a terminal run is refused, loudly.** `spawn_resume_node` reads the run header first and
  refuses when its status is `Succeeded` or `Failed` — a named event `run.resume.refused` (`run`, `status`)
  from `crates/pii-observe/src/names.rs`, and no execution. The scheduler's `let _ =` at
  `scheduler.rs:91` becomes a match that logs the refusal with the run and the key (refusals are loud,
  spec 177's rule) — a due timer whose run is gone must not vanish in silence.
- **R4 — controls (each an assertion).**
  1. A suspended timer run that is NOT cancelled still fires at its instant (the r4353 shape).
  2. `remove_run` for one run leaves another run's entry intact (two runs in the wheel).
  3. A manual `pii run resume` of a suspended run is unchanged (R3 refuses only terminal statuses).
  4. `take_due` past an instant with no entries returns empty (the wheel is not corrupted by removal).

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `a_removed_timer_does_not_fire` (pii-store lib) | `put_timer(t, "r1", "k")`, `remove_run("r1")`, `take_due(t+1)` | removal returns 1; due list empty |
| `another_run_s_timer_survives_the_removal` (control) | two runs' entries; remove one | the other still fires |
| `a_cancelled_run_s_timer_does_not_resurrect_it` (pii-server `tests/cancel_disarms_the_timer.rs`) | a suspended run with a wheel entry; `cancel_run`; `take_due` past the instant | no resume, no execution; `run.cancelled` carries `disarmed=1` |
| `a_suspended_run_still_fires_when_not_cancelled` (control) | the same shape, no cancel | the run resumes (the digest's normal path) |
| `a_due_timer_for_a_finished_run_is_refused_loudly` | a wheel entry naming a `Failed` run; fire the scheduler | no execution; `run.resume.refused` names the run |

## Exit Criteria

- [ ] `cargo test -p pii-store --lib timer 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-server --test cancel_disarms_the_timer 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `grep -q 'fn remove_run' crates/pii-store/src/timer.rs`
- [ ] `cargo test -p pii-server --lib 2>&1 | tail -2 | grep -q 'test result: ok'`

## Guardrails

- **Do NOT delete a run record or rewrite a header.** The run stays `Failed` with `cancelled: <reason>`;
  this spec only disarms the schedule and refuses the resurrection.
- **Do NOT widen `take_due`.** Its single write transaction is the cross-process safety property (spec 30
  D1); removal is a second, separate transaction on the same table.
- **Do NOT make `remove_run` fail when the run has no entries** — `disarmed=0` is a normal answer, and a
  cancel must not fail because most runs were never timers.
- **Do NOT weaken spec 189's criteria**; they stay as they are and this adds the missing half (189 proved
  the verb; it did not prove the schedule died with it — that gap is exactly this spec's premise).
- **A green `cargo test` is not a green gate**: `bash quality-gate.sh --routine` must pass on the tree this
  leaves, and `cargo test -p pii-server --lib` stays green.
- **Mutation controls to record in the Closure**: (a) make `cancel_run` skip `remove_run` → the
  resurrection test reddens with the run executing; (b) make `remove_run` delete by fire time instead of run
  → the two-run control reddens; (c) drop the terminal-status check → the loud-refusal test reddens.

## Closure

(To be filled by the orchestrator after independent verification: the criteria's own output, the mutation's
red output, the byte-identical restore, and what this does and does not close in 070 §1.)
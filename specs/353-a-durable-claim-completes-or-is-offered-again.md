# Spec: A durable claim completes or is offered again

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening
**Findings**: Opus optimisation #3 (timer spin + lost timer) and #4 (a claimed queue row blocks the queue)
**Why it is first among equals**: the night run puts work in this queue. If a restart wedges the queue, the lane
stops with its own fix half-applied.

## Verified premises (measured 2026-11-03)

- `daemon/scheduler.rs:61` — `self.wheel.next_timer().ok().flatten()`: a read error is swallowed; the loop
  `continue`s and re-reads immediately, so a persistent error busy-loops a tokio worker without an await.
- `daemon/scheduler.rs:65-68` — when `fire_at <= now` it calls `fire_due_sync` and `continue`s: no await on that
  path either.
- `take_due` deletes the timer row **before** `spawn_resume`; if the process exits or `spawn_resume` returns `Err`,
  the run stays `Suspended` with no timer and nothing ever re-offers it.
- `daemon/lag.rs:35` `recover_suspended` exists but is called from `daemon/mod.rs:328`, i.e. only after a bus
  `Lagged` event — never at boot.
- `pii-store/src/queue.rs` — `decide` (`:356`) returns `AlreadyRunning` while any item is `Running`, so one stuck
  row blocks `claim_next` for the whole queue; `grep -c requeued_after_restart crates/pii-store/src/queue.rs` → **0**.
  Boot reconcile touches `store.redb` only, not the queue store.
- `graphs/night.toml` calls the queue graph by pin, so this is the lane the night uses.

## Requirements

**R1. No spin.** On a timer-wheel read error the scheduler emits one event (`timer.wheel.error`, carrying the
error) and awaits a backoff of at least 100 ms before retrying. On the `fire_at <= now` path it awaits
`task::yield_now()` (or the injected clock's sleep with 0 ms) so the loop cannot starve the runtime.

**R2. No lost timer.** The timer row is deleted only **after** the resume is spawned successfully. If
`spawn_resume` returns `Err`, the row is put back (or the run is moved to `Failed` with `run.resume.failed`) and an
event records which happened. A run must never end up `Suspended` with no timer row and no event.

**R3. Boot re-offers.** `Daemon::build` calls the existing `lag::recover_suspended` once the scheduler is up, so a
run left `Suspended` with no timer is resumed or failed at start, without waiting for a bus `Lagged`.

**R4. Queue lease.** `claim_next` records `claimed_by` and `claimed_at_ms` on the item. A row is requeued to
`pending` with a `queue_event` kind `requeued_after_restart` when its claiming run is terminal (`Succeeded`,
`Failed`, `Cancelled`) or no longer exists, and it is **not** requeued while its run is alive or `Suspended`.

**R5. `decide` stops blocking.** `AlreadyRunning` is returned only while some item's run is genuinely alive. Once
R4 requeues the stuck row, `claim_next` returns `Claimed` again.

## The requirement a plausible implementation satisfies in appearance only

A worker can make the scheduler `sleep` on error and call `recover_suspended` at boot, and both tests about the
scheduler pass — while the queue still wedges, because the queue's defect is a **state** defect, not a timing one.
The queue test below claims a row, marks its run terminal, builds a fresh daemon, and requires a **new claim**. A
worker who only adds a sleep passes the first two tests and fails this one.

## TDD contract

| file | test | assertion |
|---|---|---|
| `crates/pii-server/tests/scheduler_does_not_spin.rs` | `an_error_does_not_busy_loop` | with a wheel whose `next_timer` fails, at most 2 polls occur in 1 s of paused time, and `timer.wheel.error` appears once |
| same | `CONTROL_a_normal_timer_still_fires` | a 1 ms timer fires exactly once (unchanged behaviour) |
| `crates/pii-server/tests/scheduler_keeps_the_timer.rs` | `a_failed_resume_does_not_lose_the_run` | `spawn_resume` returns `Err` → the run is `Failed` with `run.resume.failed`, or the timer row is back; **never** Suspended-and-forgotten |
| `crates/pii-server/tests/boot_recovers_suspended.rs` | `a_suspended_run_with_no_timer_is_recovered_at_boot` | after `Daemon::build`, the run is resumed or `Failed` with an event, with no `Lagged` delivered |
| `crates/pii-server/tests/queue_requeue_after_restart.rs` | `a_claim_whose_run_died_is_offered_again` | claim; mark the run `Failed`; build a fresh daemon; `claim_next` → `Claimed`, and a `requeued_after_restart` queue event exists |
| same | `CONTROL_a_live_claim_is_not_requeued` | claim with a live run → a second `claim_next` returns `Blocked`/`Empty`, **not** the same item |

All six must be RED before the change; paste the failing output in the Closure.

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test queue_requeue_after_restart 2>&1 | grep -q 'test result: ok\. 2 passed'`
- [ ] `cargo test -q -p pii-server --test scheduler_does_not_spin 2>&1 | grep -q 'test result: ok\. 2 passed'`
- [ ] `cargo test -q -p pii-server --test boot_recovers_suspended 2>&1 | grep -q 'test result: ok\. 1 passed'`
- [ ] `grep -q 'requeued_after_restart' crates/pii-store/src/queue.rs`
- [ ] `grep -q 'timer.wheel.error' crates/pii-observe/src/names.rs`
- [ ] `cargo xtask fresh-gate`

## Guardrails

- Do NOT change `decide`'s meaning for a live run: a `Suspended` run still blocks its row (that is correct — the
  timer will resume it).
- Do NOT delete a run to "fix" the queue; requeue the row, keep the run and its journal.
- Do NOT weaken `queue_blocked_is_not_empty` or `timer_no_refire`; their counts must not drop.
- Do NOT add a second scheduler or a second queue.
- The worker does not commit.

## Files

- `crates/pii-server/src/daemon/scheduler.rs` (R1, R2)
- `crates/pii-server/src/daemon/mod.rs` (R3 — call `recover_suspended` at build)
- `crates/pii-store/src/queue.rs` (R4, R5 — lease fields, requeue, `decide`)
- `crates/pii-observe/src/names.rs` (`timer.wheel.error`, `run.resume.failed`, `requeued_after_restart`)
- `crates/pii-server/src/daemon/mod.rs` (R4 boot reconcile)
- the six test files named above

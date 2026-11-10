# Spec: Silent paths speak

**Project**: <repo> · **Phase**: 0.2.1 hardening · **Source**: Opus optimisation review 2026-11-03,
workstream **W4** — closes the observability gaps behind findings 3, 4, 6, 8 and 15.

## Verified premises

- The following write **no** journal or observe event today: a queue claim, a timer firing, a timer error, a
  resume start and failure, a failed cursor advance, a poll panic, a worker respawn failure (read in
  `daemon/scheduler.rs`, `daemon/router.rs`, `engine/run_pool.rs`, `daemon/supervisor.rs`).
- `crates/pii-observe/src/names.rs` has no `queue.item.claimed`, `timer.fired`, `run.resume.failed` or
  `worker.respawn.failed` constants (`grep -c` → 0 for each).
- `lint-observability` enforces raw-print budgets per file, so new event code must use the observe seam.

## Requirements

**R1.** Each path above emits exactly one named event, with the identifiers a person needs (run id, item id, timer
key, channel name) and no secret: `queue.item.claimed`, `timer.fired`, `timer.error`, `run.resume.failed`,
`cursor.advance.failed`, `channel.poll.panicked`, `worker.respawn.failed`.

**R2.** The names are constants in `pii-observe/src/names.rs`; no string literal is written at the call site.

**R3.** An event is emitted on the failure path only when the failure happened: a control test asserts the event is
absent on the success path.

## The requirement a plausible implementation satisfies in appearance only

A worker can add the constants and one call each, and the tests can assert the constant exists — while the live path
still emits nothing because the call was placed in a branch that never runs. Every test below drives the real path
(the failing scheduler, the panicking poll) and asserts the event appears in the journal.

## TDD contract

`crates/pii-server/tests/silent_paths_speak.rs`:

| test | assertion |
|---|---|
| `a_timer_error_is_recorded` | a wheel whose read fails produces exactly one `timer.error` |
| `a_claimed_item_is_recorded` | a claim produces one `queue.item.claimed` carrying the item id |
| `CONTROL_a_successful_timer_records_fired_and_no_error` | one `timer.fired`, zero `timer.error` |
| `a_panicking_poll_is_recorded` | a poll closure that panics produces `channel.poll.panicked` and the channel degrades |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test silent_paths_speak 2>&1 | grep -qE 'test result: ok. 4 passed'`
- [ ] `grep -q 'queue.item.claimed' crates/pii-observe/src/names.rs`
- [ ] `grep -q 'timer.error' crates/pii-observe/src/names.rs`
- [ ] `cargo run -q -p xtask -- lint-observability`

## Guardrails

- Do NOT log a secret, a token, or a whole payload: ids and names only.
- Do NOT emit on the success path what the failure path emits (that would make the control meaningless).
- Do NOT add raw prints; use the observe seam so `lint-observability` stays green.
- The worker does not commit; the orchestrator reviews and commits.

## Files

- `crates/pii-observe/src/names.rs`
- `crates/pii-server/src/daemon/scheduler.rs`
- `crates/pii-server/src/daemon/router.rs`
- `crates/pii-server/src/engine/run_pool.rs`
- `crates/pii-server/src/daemon/supervisor.rs`
- `crates/pii-server/tests/silent_paths_speak.rs` (new)

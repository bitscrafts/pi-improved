# Spec: 243 — an operator can stop a running coder node explicitly, and the agent dies with it

**Project**: <repo>
**Phase**: filed 2026-10-23T11:40Z by the orchestrator (defect 18 in `_docs/05-operations/agent-management-findings-2026-10-23.md`).
Premises read, not assumed:

- `crates/pii-server/src/engine/runs.rs` `cancel_run` (line ~309): a run that is `engine.is_in_flight(run_id)` is REFUSED with
  `run '<id>' is executing node '<node>'; cancel refused` and the event `run.cancel_refused`; a run that is not in flight is
  closed `Failed` with `cancelled: <reason>` (`close_failed`) and its timer disarmed. The doc says "cancellation never abandons
  work half-done". `crates/pii-cli/tests/run_cancel.rs:126` asserts the refusal text.
- `crates/pii-server/src/lib.rs:638` `Engine::is_in_flight`; `Engine::cancel(run_id)` (line ~644) trips the run's
  `CancelToken` and is idempotent. The token is what `Ctx::check()` reads.
- A hung coder agent is stopped by `wait_child`, which calls `ctx.check()` every `POLL` and, on a cancel, kills the child's
  PROCESS GROUP (`ChildProc::kill` is `killpg`, `childproc.rs`); `PodmanApi::start_and_wait` passes `ctx.check()` into every
  call and force-deletes the container on every path. So the machinery that stops the agent exists; only the operator verb is
  missing.
- Measured 2026-10-23: a hung agent could not be stopped by any `pii` verb (the run stayed `running` at 40 s).

## Overview

The refusal is a deliberate rule and stays the DEFAULT. What is missing is an explicit way to say "I accept that this node's
work is abandoned": `pii run cancel <run> --kill`. It trips the run's token, waits for the executing node to end, and closes the
run as cancelled. Without `--kill` nothing changes.

## Requirements

- **R1** `run.cancel` (the daemon method behind `pii run cancel`) accepts an optional boolean `kill` (default false). With
  `kill` false, behaviour is byte-for-byte today's, including the refusal text and `run.cancel_refused`.
- **R2** With `kill` true and the run in flight: journal `run.cancel_killing` (fields `run`, `node`, `reason`), call
  `Engine::cancel`, and wait up to 30 s for the run to leave flight (`is_in_flight` false). When it does, close the run as
  `Failed` with `cancelled: <reason> (node '<node>' abandoned)`, emit `run.cancelled`, disarm its timer — exactly as the
  non-flight path does after its refusal check.
- **R3** If the run is still in flight after 30 s, return `PiiError::Invalid` saying `run '<id>' did not stop within 30 s after
  kill`, journal `run.cancel_kill_timeout`, and leave the run as it is. Never mark a run closed while its agent may still run.
- **R4** With `kill` true and the run NOT in flight, or already finished, behaviour is today's (closed, or `already finished`);
  `kill` changes nothing there.
- **R5** `pii run cancel <run> --kill [--reason <text>]` is the CLI form. `--kill` without a run id is a usage error.
- **R6** The killed agent leaves no process and no container: the test proves it, it is not assumed from the code reading.

## Architecture

`cancel_run` gains a `kill: bool`. The wait is a bounded poll on `is_in_flight` at 50 ms. The CLI passes the flag through the
existing control request; the request type gains an optional field that defaults to false, so an older client keeps working.
Rejected: making `cancel` kill by default (it would break `run_cancel.rs` and the half-done rule); a separate `run kill` verb (a
second path to maintain, with its own refusal rules).

## TDD Contract

New test file `crates/pii-server/tests/cancel_kill.rs` (engine level, using the stand-in agent technique the neighbouring coder
tests use: a fake `pi` script under `_tmp/` that sleeps) and a CLI test `crates/pii-cli/tests/run_cancel_kill.rs` following
`run_cancel.rs`.

| test | given | expects |
|---|---|---|
| `cancel_without_kill_is_still_refused_on_a_running_node` | run in flight, `kill=false` | `Invalid` containing `cancel refused` (CONTROL: today's behaviour) |
| `cancel_with_kill_stops_a_hung_agent_and_closes_the_run` | hung fake agent (and a `sleep` child), `kill=true` | returns Ok; run status `Failed`; error starts `cancelled:` and contains `abandoned` |
| `the_killed_agent_and_its_children_are_gone` | same | the agent pid and the child `sleep` pid no longer exist within 2 s |
| `a_run_that_will_not_stop_is_not_closed` | an in-flight run whose node ignores the token for longer than the wait (test shortens the wait to 300 ms through a parameter) | `Invalid` containing `did not stop`; run status still `Running` |
| `kill_on_a_run_that_is_not_in_flight_is_the_old_path` | suspended run, `kill=true` | closed `Failed cancelled: ...` with NO `abandoned` text (CONTROL) |
| `kill_on_a_finished_run_is_refused_as_before` | finished run, `kill=true` | `already finished` (CONTROL) |
| `the_timer_of_a_killed_run_is_disarmed` | killed run that had an armed timer | `timer_store` holds zero entries for it |
| `the_kill_is_journalled` | killed run | journal holds `run.cancel_killing` then `run.cancelled`, in that order |
| `the_cli_passes_kill_through` | CLI against the test daemon, `--kill` on a hung run | exit 0, run `failed`, text names the node |
| `the_cli_without_kill_keeps_the_refusal` | CLI, no flag, running node | non-zero exit and the refusal text (CONTROL) |

The requirement a plausible implementation gets wrong is R3: closing the run right after tripping the token reports a stopped
run while the agent still holds the workdir. The did-not-stop row (status stays `Running`) and the agent-gone row catch it.

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test cancel_kill 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-cli --test run_cancel_kill 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-cli --test run_cancel 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `grep -q "cancel_killing" crates/pii-observe/src/names.rs`
- [ ] `cargo clippy -q -p pii-server -p pii-cli --all-targets -- -D warnings`

## Guardrails

- `specs/` are READ-ONLY for the implementer. If `cancel_run` or the control request is not shaped as described, STOP and report
  the mismatch.
- `run_cancel.rs` (the existing refusal test) must stay green UNCHANGED. Never make `kill` the default, never weaken the refusal
  for a call without the flag.
- Real processes in tests are `sleep`/the fake agent, started in their own group and killed by a `Drop` guard even when an
  assertion fails. A test that leaves one behind is a defect.
- Add the new event names to `crates/pii-observe/src/names.rs`; the observability lint checks it.
- Tests write under `_tmp/`. 450-effective-line file gate and `#![forbid(unsafe_code)]` apply. Do NOT commit.

## Closure

_(empty — the implementer's readings go here)_

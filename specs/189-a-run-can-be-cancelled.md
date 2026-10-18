# Spec: a run can be cancelled

## Overview

`pii run` offers only `ls`, `start` and `get`. When production held FOUR suspended digest runs for ONE cron entry
(measured 2026-10-18 18:54Z, 070 section 0.1), there was no supported way to retire three of them. The only options were a
hand edit of the store — which this project refuses — or removing the cron entirely, which is what was done to keep the next
occurrence from executing all four.

**Project**: /projects/pi-improved

## Requirements

`pii run cancel <run-id> --reason <text>` closes a run that has not started or is suspended, records the reason, and emits
`run.cancelled` carrying the run id and reason. A run whose node is executing is REFUSED with `run.cancel_refused` naming
the running node — cancellation must never abandon work half-done.

## Exit Criteria

- [ ] `cargo test -p pii-cli --test run_cancel`
- [ ] `grep -q 'run.cancelled' crates/pii-observe/src/names.rs`

## Guardrails

- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT delete the run: cancellation is a STATUS with a reason, visible in `run ls`.
- Do NOT allow cancelling a run whose node is executing; refuse and name it.

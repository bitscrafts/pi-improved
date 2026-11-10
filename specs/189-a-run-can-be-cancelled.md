# Spec: a run can be cancelled

## Overview

`pii run` offers only `ls`, `start` and `get`. When production held FOUR suspended digest runs for ONE cron entry
(measured 2026-10-18 18:54Z, 070 section 0.1), there was no supported way to retire three of them. The only options were a
hand edit of the store — which this project refuses — or removing the cron entirely, which is what was done to keep the next
occurrence from executing all four.

**Project**: <repo>

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

## Closure

Verified by the orchestrator on 2026-10-19, on the committed tree (`4182856`), independently of any worker report.

**The two criteria, run by hand.**

- `cargo test -p pii-cli --test run_cancel` → `1 passed; 0 failed`, rc=0 (2.30 s and again 2.38 s on the restored
  bytes).
- `grep -q 'run.cancelled' crates/pii-observe/src/names.rs` → rc=0. The constant is at `names.rs:163` and is the
  one `cancel_run` emits (`crates/pii-server/src/engine/runs.rs:340`) — not a string asserted only by a test.

**The fix, reddened by the orchestrator's own mutation.** `runs.rs:319` `if engine.is_in_flight(run_id.as_str())`
→ `if false && engine.is_in_flight(run_id.as_str())` (the executing-node refusal bypassed, nothing else touched):
`cargo test -p pii-cli --test run_cancel` → **FAILED, rc=101**, `timed out waiting for a refusal naming the
executing node` (20.28 s), `0 passed; 1 failed`. The criterion therefore bites on the half the spec exists for —
cancellation must never abandon work half-done.

**Restore, byte-identical, then green on those bytes.** The file was copied aside before the mutation; after the
red run it was restored with `cp` (not `cp -p`) and `touch`ed, per the false-red pitfall measured on spec 187.
`sha256sum` before mutation and after restore are the same value:
`be463cd5df652a33180440515581895df2d1a44b840fe7d250ed80e55f2cdadd` (also `POST` after the green run), and the
re-run on those bytes is `1 passed; 0 failed`, rc=0.

**Two measurement traps, recorded so the next account is honest.**

1. **The first mutation never applied and its green was not evidence.** The naive replacement string
   (`if engine.is_in_flight(...)`) matches **two** sites in `runs.rs` (275 in `close_failed`, 319 in `cancel_run`),
   so the edit aborted on an `assert s.count(old) == 1` and the untouched tree still ran green. A green produced
   by a mutation that did not land certifies nothing; the count assertion is what caught it.
2. **A panicking `run_cancel` test leaves its scratch daemon alive and the pipeline hangs.** In two red runs the
   test binary had already exited while its child `pii daemon` (`crates/pii-cli/_tmp/run-cancel-*/`) still held the
   harness's stdout pipe, so `cargo test | tail` never saw EOF and the command sat for minutes. Both were killed by
   exact pid (`ps -eo pid,cmd`), never by pattern. The test itself is correct; this is a note for whoever reads a
   hung red run, not a defect to fix here.

**What this closes and what it does not.** The verb exists and is proven: a suspended run is retired with its
reason on the record and `run.cancelled` logged; an executing run is refused by name. What the deployed system
still owes is the production half — the four suspended digest runs on the live store (`armed 4` against
`cron_entries 1`, 070 §5) are retired by the operator's script on the host, and the next occurrence at
2026-10-19 09:00 UTC is the measurement that closes 187/189 together.

# Spec: Three incidents with no ending, and what the log actually says

## Overview

`pii incidents scan --source runtime` on the production log reports **three** `run-stalled` incidents, all from
2026-11-02 05:03: `r000000004356`, `r000000004577`, `r000000004578`. The rules were fixed twice today (a cancelled
run is terminal; one boot's two start marks are not two starts) and the count came down 14 → 4 → 3.

What remains is not a rule defect and not a flake: those runs' logs carry **no terminal event at all**. Either the
log was truncated, the daemon died mid-run, or the run is genuinely still open — and the rule cannot tell those
apart, so it reports all three as stalls.

The honest question is: *what does the log say, and what should a reader conclude when a run has no ending?*

**Project**: /projects/pi-improved
**Phase**: production hardening — the last unexplained residue in the incident reader.

## Verified premises — READ FROM THE LOG, 2026-11-02 22:14Z

The three incidents are NOT three stalls. The host log says what each run was:

```
r000000004356  cron.rearmed x5  ->  cron.disarmed{"disarmed":0,"reason":"digest is on demand (operator decision)"}
               stood down by an operator decision; it never ran and never ended. Not a stall.

r000000004577  trajectory.write.failed{"error":"Permission denied (os error 13)"}  ->  channel.posted{"bytes":94}
r000000004578  trajectory.write.failed{"error":"Permission denied (os error 13)"}  ->  channel.posted{"bytes":81}
               both runs DID their work and posted to Discord. What failed was the TRAJECTORY WRITE, so the run
               cannot be reconstructed afterwards. That is a real defect, and it is invisible today because the
               scanner only ever asks "did this run end".

Measured: `~/.local/state/pii/trajectory/` was created at 17:03 today, AFTER those 05:03 runs, and no
trajectory.write.failed has appeared since the deploy — so the EACCES belongs to the pre-deploy binary and is not
recurring. That does not make it harmless: it is the only class of failure that destroys the evidence needed to
diagnose the next one.
```

## Requirements

### Functional

1. **A stood-down run is not an incident.** `cron.disarmed{reason}` after a run's last event ends it as
   `run-disarmed`, and that is NOT reported as an incident: an operator decision is not a fault. The reason string
   travels into the evidence line so a reader sees whose decision it was.
2. **A posted run is not stalled.** `channel.posted` for the run after its last start means the run did its work;
   it is not `run-stalled`, whatever else is missing.
3. **A lost trajectory is its own incident** — `observability-lost:{run}`, carrying the error and the run id. This is
   the opposite of a stall: the run may have finished perfectly, and the evidence of it is gone. It is the failure
   that makes every later diagnosis harder, so it must be visible on its own line.
4. **The scan over the real host window reports**: 0 `run-stalled`, 1 `run-disarmed` (r4356), 2
   `observability-lost` (r4577, r4578). Any other count is a regression.
5. **The fixture is a real copy, anchored by extraction**: `crates/pii-learn/tests/fixtures/production/runtime-0503.jsonl`
   is produced by `scripts/checks/copy-runtime-window.sh <host>` — which copies every line of the daemon's log
   between the `daemon.started` before the window and the one after it, unedited. Never hand-written.

### Non-Functional

- Verdicts stay data-driven and testable without a daemon.
- A verdict needs the event that supports it; the evidence line quotes the event's own fields.

## TDD Contract

| Test (`crates/pii-learn/tests/incidents_unfinished.rs`) | Assertion |
|---|---|
| `a_disarmed_cron_is_not_an_incident` | `cron.disarmed` after the last event → `run-disarmed`, and it appears in no incident list |
| `CONTROL_a_disarmed_run_that_then_keeps_logging_is_stalled` | the same run with later run events and no ending → `run-stalled` |
| `a_posted_run_is_not_stalled` | `channel.posted` after the last start → no `run-stalled` |
| `a_lost_trajectory_is_observability_lost` | `trajectory.write.failed` → `observability-lost:{run}` naming the error |
| `CONTROL_a_run_that_never_ends_and_never_posts_is_still_stalled` | the near-miss that must keep the old verdict |
| `the_0503_window_yields_its_verdicts` | the real fixture → exactly 0 stalled, 1 disarmed, 2 observability-lost |

## Exit Criteria

- [ ] `cargo test -q -p pii-learn --test incidents_unfinished 2>&1 | grep -q '6 passed'`
- [ ] `cargo test -q -p pii-learn --test incidents_runtime`
- [ ] `bash scripts/checks/gates.sh`

## Guardrails

- Do NOT report `run-disarmed` as an incident: an operator decision is not a fault, and treating it as one trains
  the reader to ignore the list.
- Do NOT infer a stall from a missing terminal event when the log says the run posted.
- Do NOT hand-write the fixture: extract it, and record the command in its header.
- Do NOT fix the trajectory EACCES here — it is a separate defect with its own evidence (see Closure).

## Files

- `crates/pii-learn/src/incident_runtime.rs`
- `crates/pii-learn/tests/incidents_unfinished.rs`
- `crates/pii-learn/tests/fixtures/production/runtime-0503.jsonl`
- `scripts/checks/copy-runtime-window.sh`

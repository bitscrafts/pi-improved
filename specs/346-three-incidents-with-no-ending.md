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

## Verified premises — from the real fixture and the current binary, 2026-11-02 22:2xZ

`cargo run -q -p pii-cli -- incidents scan --source runtime --log fixtures/production/runtime-0503.jsonl`:

```
incident refusal-repeated:channel.poll.failed count=731 first_seen=02:08:49Z last_seen=03:05:45Z
incident run-stalled:r000000004578 latency_secs=1
incident run-stalled:r000000004577 latency_secs=2
starts_considered=9 incidents=3
```

What the log says about each, reading the events themselves:

- **r000000004356 is NOT an incident.** Its ending is `run.cancelled` with
  `reason:"digest is on demand (operator decision)"` — an operator decision, and the rule already counts
  `run.cancelled` as terminal (commit `177e4ef`). This spec's first draft assumed it was one of the three; the scan
  says otherwise, which is exactly why the fixture is worth having.
- **r000000004577 and r000000004578 posted to Discord** (`channel.posted`, 94 and 81 bytes) and their
  `trajectory.write.failed{"error":"Permission denied (os error 13)"}` is the real event: the run did its work, and
  the record of it was destroyed. Calling that a *stall* accuses the run of a fault it did not commit, and hides the
  fault it did.
- **`refusal-repeated:channel.poll.failed × 731`** between 02:08 and 03:05 predates the deploy and stopped when the
  deployed binary took over. It is not this spec's subject, but it is the reason the scan must not be read as "the
  host is fine": a refusal repeated 731 times is a channel that was down for an hour.

## Requirements

### Functional

1. **A posted run is not stalled.** `channel.posted` for the run after its last start means the run did its work;
   it is not `run-stalled`, whatever else is missing.
2. **A lost trajectory is its own incident** — `observability-lost:{run}`, carrying the error and the run id. This is
   the opposite of a stall: the run may have finished perfectly, and the evidence of it is gone. It is the failure
   that makes every later diagnosis harder, so it must be visible on its own line.
3. **The scan over the real host window reports**: 0 `run-stalled`, 2 `observability-lost` (r4577, r4578), and the
   historical `refusal-repeated:channel.poll.failed` unchanged. Any other count is a regression.
4. **The fixture is a real copy, anchored by extraction**: `crates/pii-learn/tests/fixtures/production/runtime-0503.jsonl`
   is produced by `scripts/checks/copy-runtime-window.sh <host>` — which copies every line of the daemon's log
   between the `daemon.started` before the window and the one after it, unedited. Never hand-written.

### Non-Functional

- Verdicts stay data-driven and testable without a daemon.
- A verdict needs the event that supports it; the evidence line quotes the event's own fields.

## TDD Contract

| Test (`crates/pii-learn/tests/incidents_unfinished.rs`) | Assertion |
|---|---|
| `a_posted_run_is_not_stalled` | `channel.posted` for the run → no `run-stalled` for it |
| `a_lost_trajectory_is_observability_lost` | `trajectory.write.failed` → `observability-lost:{run}` naming the error |
| `CONTROL_a_run_that_never_ends_and_never_posts_is_still_stalled` | the near-miss that must keep the old verdict |
| `the_0503_window_yields_its_verdicts` | the real fixture → 0 `run-stalled`, 2 `observability-lost`, the channel refusal unchanged |

## Exit Criteria

- [ ] `cargo test -q -p pii-learn --test incidents_unfinished 2>&1 | grep -q '4 passed'`
- [ ] `cargo test -q -p pii-learn --test incidents_runtime`
- [ ] `bash scripts/checks/gates.sh`

## Guardrails

- Do NOT infer a stall from a missing terminal event when the log says the run posted.
- Do NOT hand-write the fixture: extract it, and record the command in its header.
- Do NOT fix the trajectory EACCES here — it is a separate defect with its own evidence (see Closure).

## Files

- `crates/pii-learn/src/incident_runtime.rs`
- `crates/pii-learn/tests/incidents_unfinished.rs`
- `crates/pii-learn/tests/fixtures/production/runtime-0503.jsonl`
- `scripts/checks/copy-runtime-window.sh`

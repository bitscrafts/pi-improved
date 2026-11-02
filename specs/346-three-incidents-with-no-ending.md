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

## Verified premises (2026-11-02, on the host log)

- `pii incidents scan --source runtime` → `starts_considered=14 incidents=3 files=1`.
- The three are `run-stalled`, first seen 05:03:11–05:03:36, latencies 1–2 s.
- `crates/pii-learn/src/incident_runtime.rs` counts a run finished on `run.outcome`, `run.start_refused` and
  `run.cancelled`; a run with none of those and an old last event is `run-stalled`.
- The 05:03 window also carries the daemon's own restart, so "no terminal event" may be the restart, not the run.

## Requirements

### Functional

1. **Model first, in minilam's language.** `crates/pii-learn/tests/fixtures/incidents-0503.json` is a data model of
   the rule (events → verdicts) for the same log; `minilam` says what each run's verdict should be. The diff against
   `pii incidents scan` is the evidence, printed side by side.
2. **The scan must distinguish three states** and say which it means:
   - `run-stalled` — the log continues after the run's last event and the run never ended;
   - `run-unfinished` — the log ENDS before the run ended (the log itself stops, so nothing can be concluded);
   - `run-restarted` — a `daemon.stopped`/`daemon.starting` follows the run's last event, so the run was cut short
     by the restart, and the incident names the restart.
3. **The three 05:03 incidents must land in one of those**, with the evidence quoted (the event before and after the
   run's last event, or the end of the log).
4. **A control proves each verdict can fail**: a run that never ends while the log continues is `run-stalled`; a log
   that stops mid-run is `run-unfinished`; a restart after the last event is `run-restarted`.

### Non-Functional

- The rule stays data-driven and testable without a daemon: fixtures in, verdicts out.
- No widening "just in case": a verdict needs an event that supports it.
- minilam is the ORACLE here, never the driver (its A1 drive half is open).

## TDD Contract

| Test (`crates/pii-learn/tests/incidents_unfinished.rs`) | Assertion |
|---|---|
| `a_log_that_stops_mid_run_is_unfinished_not_stalled` | truncated fixture → `run-unfinished`, names the run |
| `CONTROL_the_same_run_in_a_continuing_log_is_stalled` | same events + later unrelated events → `run-stalled` |
| `a_restart_after_the_last_event_is_restarted` | `daemon.stopping` + `daemon.started` after → `run-restarted` |
| `CONTROL_a_run_that_ended_is_no_incident` | with `run.outcome` → none |
| `the_0503_log_yields_its_verdicts` | the real log copy → each of the three lands in one state, with its evidence line |

## Exit Criteria

- [ ] `cargo test -q -p pii-learn --test incidents_unfinished`
- [ ] `cargo test -q -p pii-learn --test incidents_runtime`
- [ ] `bash scripts/checks/gates.sh`

## Guardrails

- Do NOT keep reporting a log that simply ends as `run-stalled`: it accuses the run of a fault nobody can see.
- Do NOT re-classify the three 05:03 incidents by hand — the fixture decides, and the control proves it can.
- Do NOT add a fourth verdict without a test that fails when it is removed.

## Files

- `crates/pii-learn/src/incident_runtime.rs`
- `crates/pii-learn/tests/incidents_unfinished.rs`
- `crates/pii-learn/tests/fixtures/runtime-unfinished.jsonl`
- `crates/pii-learn/tests/fixtures/runtime-restarted.jsonl`
- `crates/pii-learn/tests/fixtures/incidents-0503.json`

# Spec 323: a park whose cause was repaired is offered again, through a recorded transition

## Overview

**Project**: <repo>
**Phase**: autonomy lane

Row **A185** (spec 321, the parity oracle in a clean export) was parked after three attempts. Commit
`558759e` repaired its cause, and the repair was written into the row's `last` field — *"park cause
repaired in 558759e: … the next orchestrator tick runs it and closes the row"*. The status, however,
stayed `parked`.

`state.py pick` treats `parked` as terminal by design (spec 169: a parked row waits for a person), so the
row was never offered. The loop then ran a cycle at 13:31:39Z and printed `OUTCOME=queue-empty` with
nothing dispatched for over an hour, while the queue brief said *"5 row(s) NOT done"* and the one real
leftover — A185 — sat at the top of that list. The queue stopped while it was not empty.

The defect is not that a park is respected; it is that **the repair is written as prose, not as a
transition**, so the machine cannot tell a repaired park from a live one. This spec makes the repair an
operation with a marker the picker understands.

## Requirements

**R1 — the repair is an operation.** `state.py repair <id> <note…>` records the repair on the row:
`last` becomes `park-cause-repaired: <note>`, and the status becomes `needs-review` when the row has a
session (the session is what a resumed dispatch needs) or `pending` when it has none. `attempts` is
never touched: the attempt cap stays honest. The subcommand prints the resulting row as JSON.

**R2 — the picker understands the marker.** `resumable()` accepts a `needs-review` row whose `last`
starts with `park-cause-repaired` and which has a non-empty session, exactly as it already accepts
`window-closed`, `gate-failed`, `verify-failed` and the `worker-rc=` forms.

**R3 — it is offered, and named.** `pick` then dispatches that row with its session as the last TSV
field, and names it on stderr the way the other resumed rows are named.

**R4 — the controls that must NOT be offered:**
- a `parked` row whose `last` is `3-strikes` (the cap is terminal until someone repairs it);
- a `parked` row carrying `park-cause-repaired` **but still `parked`** — the A185 shape: the marker
  must go through R1's transition, so the recorded status never lies about a row being worked;
- a `needs-review` row with the marker and an **empty** session (nothing to resume);
- a `done` row.

**R5 — the row that waits is named.** When `pick` finds nothing dispatchable and at least one row waits
for a person, it says so on stderr with the row ids (it already prints "every open row is parked: …";
this keeps that behaviour and the ids must appear), so a log line reading `queue-empty` is never the only
trace.

## Files

- `scripts/autonomy/state.py`
- `scripts/autonomy/tests/pick-resumes-park-cause-repaired.test.sh`

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| repair_turns_a_parked_row_into_needs_review | `repair X1 fixed` on `{parked, session S1}` | status `needs-review`, `last` starts `park-cause-repaired:`, session `S1`, attempts unchanged |
| repair_without_a_session_returns_it_to_pending | `repair X1 fixed` on `{parked}` | status `pending` |
| a_repaired_park_is_offered | `{needs-review, last park-cause-repaired: x, session S1}` | `pick` prints `X1` with `S1` last |
| a_three_strikes_park_is_not_offered | `{parked, last 3-strikes, session S1}` | `pick` prints `NONE` |
| the_untranslated_marker_is_not_offered | `{parked, last park-cause-repaired: x, session S1}` | `pick` prints `NONE` |
| a_repaired_park_without_a_session_is_not_offered | `{needs-review, marker, session ""}` | `pick` prints `NONE` |
| a_done_row_is_never_offered | `{done, marker, session S1}` | `pick` prints `NONE` |

## Exit Criteria

- [ ] `bash scripts/autonomy/tests/pick-resumes-park-cause-repaired.test.sh`
- [ ] `bash scripts/autonomy/tests/pick-resumes-window-closed.test.sh`
- [ ] `bash scripts/autonomy/tests/cycle-requeue-outcome.test.sh`
- [ ] `uv run --no-project python scripts/autonomy/state.py repair NOSUCH fixed` exits non-zero
- [ ] `cargo run -q -p xtask -- lint-files` exits 0

## Guardrails

- Do NOT touch `attempts` in `repair`: the 3-strikes cap is a safety property, not bookkeeping.
- Do NOT make `pick` accept a `parked` row directly: the status must change, or the record lies.
- Do NOT change what `parked` means for a row with no repair marker (spec 169 stands).
- Do NOT edit `cycle.sh` in this spec: the picker is the only producer of dispatchable rows.

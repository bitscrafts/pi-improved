# Spec: an environment block parks the row instead of spinning
#: depends=433 status=pending block=-

## Overview
Finding F7 of the Opus analysis: `lane-not-started` writes `status=pending`, which is the same value a row has before
it is ever tried. `pick` (`state.py:197-205`) returns the first `pending` row, so the queue re-picks the blocked row
every tick, the lane refuses again, and **every row behind it never runs** — head-of-line blocking. Measured
2026-11-17T14:14:40Z: `OUTCOME=lane-not-started ITEM=425 RC=9`, and the spec 430 row behind it was never dispatched.

The cycle already does the right thing with the row's attempts ("keeps its place and its attempts; nothing was spent"),
so this is only about the *dispatch decision*, not about failure handling.

The defect is also written down as a requirement: `specs/430-a-graph-manages-the-workers-container.md` R8 says "no
runtime -> the row stays `pending`", which is precisely this stall. R8 is reworded to say the row is **skipped for
dispatch with a named reason**.

## Requirements
R1. `lane-not-started` records `blocked=runtime:<short reason>` and leaves `attempts` untouched. It is not a failure and
    it is not `pending`.
R2. `pick` skips a row whose `blocked` starts with `runtime:` **when every remaining row needs the same lane** — and
    when a row that does not need the lane exists, that row is dispatched instead.
R3. When all open rows are blocked on the same resource, the cycle prints one line naming the block and exits
    successfully; it does not spin per tick.
R4. The block is cleared automatically when the resource appears (the next successful lane start for any row clears
    `blocked=runtime:*` on all rows), and a human can clear it with `state.py set <row> blocked=`.
R5. A row blocked for any other reason (`blocked=dependency:<row>`, `blocked=disk`) is unaffected by R4's clearing.
R6. Nothing about a blocked row is lost: paths, attempts, session and phase marker survive.

## TDD Contract
| Test Name | Input | Expected Output |
| --- | --- | --- |
| a_runtime_block_is_not_pending | lane-not-started | `blocked=runtime:…`, attempts unchanged |
| control_a_lane_failure_is_still_a_failure | lane ran and returned non-zero | `status=needs-review`, not `blocked` |
| pick_skips_a_runtime_blocked_row | row A blocked runtime, row B needs no lane | B dispatched |
| control_pick_never_skips_an_unblocked_row | row A pending, row B pending | A dispatched (order kept) |
| all_blocked_prints_one_line_and_exits_zero | every open row blocked runtime | one line, exit 0 |
| a_successful_lane_start_clears_the_runtime_block | block set, then any row starts | no `blocked=runtime:*` left |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/pick-skips-a-runtime-block.test.sh`

### New in this row
- [ ] `bash scripts/autonomy/tests/pick-skips-a-runtime-block.test.sh`

## Files
- `scripts/autonomy/state.py`
- `scripts/autonomy/cycle.sh`
- `specs/430-a-graph-manages-the-workers-container.md`

## Guardrails
- Do NOT treat an environment block as a row failure: attempts must not grow.
- Do NOT skip a row that is merely pending; order is the queue's contract.
- Do NOT clear a non-runtime block when a lane starts.

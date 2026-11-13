# Spec: the queue does not offer a row whose dependency is unfinished
#: depends=419 status=pending block=-

## Overview
Every spec carries a card line `#: depends=<row> status=<status> block=<row>`, and nothing reads it. Measured 2026-11-13: row 413 (spec 413) was offered while
its dependency 414 was still `pending`, because `pick` (`scripts/autonomy/state.py`) orders rows by position in `plan.tsv` and knows only `parked` /
`needs-review` as reasons to skip. That day the mistake was caught by hand — the row was parked so the dependency could go first — but the mechanism that
was supposed to prevent it does not exist on the machine, only in the document.

`depends=` is not decoration: it is the order the work must happen in. A row dispatched before its dependency spends a worker on code that will not compile or,
worse, lands a change whose premise is missing.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 419

## Requirements
- R1 `pick` reads each candidate's spec card line and offers the row only when every row named by `depends=` has status `done` or `skipped` in the state file.
- R2 A blocked row is skipped for dispatch order only — its state is never touched — and is named on stderr in the same list the parked rows use, as
  `<id>(waits <dep>)`. The dispatcher's line therefore says why the queue went to the next row.
- R3 A spec with no `depends=` key, or with `depends=-`, is offered exactly as today: the check must be inert for the 250 rows that carry the key as `-`.
- R4 A dependency that names a row absent from `plan.tsv` is skipped and named `<id>(waits <dep>:not-in-plan)`, never a plan error: a spec may be written
  before its predecessor is queued.
- R5 `pick --row <id>` does not consult `depends=`: naming a row is a deliberate act, and the dispatcher uses it when a row owns the dirty tree.
- R6 Nothing else about `pick` changes: the output columns, the statuses it offers, the parked/needs-review rules and the exit behaviour are as before.

## Architecture
```
pick:  for r in rows():
         status = ...                     (unchanged)
         parked/needs-review?             (unchanged)
         for dep in card_depends(r.spec):          <- new
            state[dep].status in (done, skipped)?  no -> skipped.append(f'{id}(waits {dep})'), continue to the next row
         row-paths judge, row-verify judge, plan_import check   (unchanged)
```
Key decision: read the dependency from the card line the specs already carry, so no new column and no backfill of 250 rows. Rejected: a new `plan.tsv`
column (the plan is generated from the specs; the column would drift). Rejected: making an unfinished dependency a plan error (a spec is often written
before its predecessor is queued, and the queue would then refuse to dispatch anything at all).

## TDD Contract
New `scripts/autonomy/tests/pick-respects-depends.test.sh`, driving the real `pick` with `PII_PLAN` and `PII_STATE` pointed at a scratch plan, a scratch
state file and scratch specs.
| Test Name | Input | Expected Output |
|---|---|---|
| `a_row_whose_dependency_is_done_is_offered` | 20 done, 21 depends=20 | `pick` prints row 21 |
| `CONTROL_a_row_whose_dependency_is_pending_is_not_offered` | 20 pending, 21 depends=20, 22 none | `pick` prints row 22, and names `21(waits 20)` on stderr |
| `CONTROL_a_spec_without_depends_is_offered` | `depends=-` on the first row | that row is printed |
| `CONTROL_a_dependency_not_in_the_plan_is_skipped_and_named` | 21 depends=99 | row 22 is printed, stderr names `21(waits 99:not-in-plan)` |
| `CONTROL_a_forced_row_ignores_its_dependency` | `pick --row 21` with 20 pending | row 21 is printed |
| `CONTROL_a_blocked_rows_state_is_untouched` | as above | the state file for row 21 is byte-identical before and after |

Acceptance: the six pass; each of these mutations makes at least one fail (the mutation script is the orchestrator's): the dependency check removed; the
status set widened to every status; the card-line parser returning nothing; the `--row` bypass removed.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `bash scripts/autonomy/tests/pick-respects-depends.test.sh`

## Guardrails
- Keep green, unedited: `bash scripts/autonomy/run-tests.sh` and `bash scripts/autonomy/sync-verify.sh --check`.
- Do NOT change the columns `pick` prints, and do NOT change which statuses are offered.
- Do NOT touch any row's state from this check, and do NOT read the network.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `scripts/autonomy/state.py`
- `scripts/autonomy/tests/pick-respects-depends.test.sh`

# Spec 327: an open row's verify column is derived from its spec, and a finished row's is frozen

## Overview

Spec 309 made the dispatcher refuse a row whose verify column is not its spec's `## Exit Criteria`, compared
command for command. That check is per-row and only runs on the row about to be dispatched. Measured
2026-10-31 across the whole plan: **76 of 183 rows cannot be reconciled with today's spec text**, 30 of them a
dropped tail — A154's row simply lost the `coder-demo.toml` grep its spec requires:

```
A154 → specs/290-…   row 9 commands, spec 10 criteria   (criterion 9 missing from the row)
```

A row *is* the gate: if its list silently loses a criterion, nothing runs it and nothing notices. Copying the
list by hand makes that drift inevitable, so the column stops being an independent copy:

- an **open** row's verify is **derived** from its spec at read time — the dispatcher cannot hand a worker a
  list the document no longer holds;
- a **finished** row's verify is **frozen history** — regenerating it from a spec amended later would claim the
  work was held to criteria that did not exist when it ran. Those rows are judged by `check-row-truth.sh`
  against the spec *at their own commit*.

## Requirements

**R1 — derivation at read time.** `state.py rows()` replaces the verify column of a row whose status is
`pending`, `in-progress` or `needs-review` with its spec's `## Exit Criteria`, joined by `;` in document order.
A finished row keeps its stored text untouched.

**R2 — the column is a cache, and staleness is an error.** `scripts/autonomy/sync-verify.sh --check` names every
open row whose stored text disagrees with its spec, writes nothing, and exits non-zero. Without `--check` it
rewrites only those rows, atomically, touching no other column and no other row.

**R3 — the rule survives an export.** A fresh-gate export has no `_tmp/autonomy/state.json`, so openness cannot
be known there. The step therefore always checks the invariant that needs no state: every row names a spec
that exists and has executable criteria. Rows whose specs predate the rule and state their criteria as prose
(16 of 183 — A38/173 wrote "one loop cycle produces a visible sequence of pii events…") are **reported** and
never silently counted as passing; an *open* row in that shape is a failure, because that is the row a worker
will be held to.

**R4 — the gate runs it.** `plan-verify|bash scripts/autonomy/sync-verify.sh --check` is a gate step, so a spec
edited without regenerating its open row turns the gate red rather than sending a worker a stale list.

**R5 — the auditor reads the file, not the derivation.** `check-row-truth.sh` parses `plan.tsv` itself. Auditing
through `state.rows()` would compare the spec with itself, which is exactly the failure it exists to catch.

## Files

- `scripts/autonomy/state.py` (`SPECS`, `OPEN`, `derived_verify`, derivation in `rows`)
- `scripts/autonomy/sync-verify.sh` (new)
- `scripts/autonomy/tests/sync-verify.test.sh` (new)
- `scripts/autonomy/check-row-truth.sh` (`stored_rows`)
- `gate-steps.txt` (the `plan-verify` step)

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| pick derives the full criteria set | open row storing only 2 of 3 criteria | the dispatcher's column 5 carries all 3 |
| the file still holds the stale text | same row, read from disk | unchanged until a write is asked for |
| --check is red on a stale row | `--check` | rc 1, names the row, plan byte-identical |
| the write path fixes it | no flag | stored text becomes the spec's; other columns untouched |
| a finished row is not a violation | done row whose spec moved | rc 0, text unchanged, by `--check` and by the write path |
| an empty spec is a refusal | open row over a spec with no criteria | rc 1, row keeps its text |

## Exit Criteria

- [ ] `bash scripts/autonomy/tests/sync-verify.test.sh 2>&1 | grep -qE "[0-9]+ ok, 0 FAIL"`
- [ ] `bash scripts/autonomy/sync-verify.sh --check`
- [ ] `grep -q "^plan-verify|bash scripts/autonomy/sync-verify.sh --check" gate-steps.txt`
- [ ] `bash scripts/autonomy/run-tests.sh 2>&1 | tail -1 | grep -qE "[0-9]+ passed, 0 failed"`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q "fresh-gate: PASS"`

## What this does not fix

The 76 historical rows stay as they are. Regenerating them from today's specs would rewrite history: a spec
amended after its item ran is indistinguishable from a row that dropped a criterion, and inventing an answer
for the difference would be worse than recording that it cannot be known. They are frozen, and
`check-row-truth.sh` says so (`182 historical, no commit: not judgeable`).

## Guardrails

- Do NOT derive for a finished row: its text is what the work was actually held to.
- Do NOT let `--check` write: a check that repairs what it audits certifies the bug.
- Do NOT audit through `state.rows()`: the derived value makes the comparison a tautology.
- Do NOT treat a prose-criteria spec as a pass: report it, and fail the row if it is still open.

# Spec 326: the row names its work, and its work's commit

## Overview

Agreed 2026-10-31, after measuring the relation between the three identifiers one unit of work carries:

```
queue row    A187                     keyed in state.json; status, attempts, session, last
spec file    specs/325-….md          committed; the document a worker reads
minilam item A186 (for spec 322)     the flow: spec→red→implement→green→criteria→gate→done
```

`state.json` rows carry exactly `{status, attempts, session, last}` (all 183 of them, measured). Neither the
spec nor the commit is a field: the sha lives inside free prose (`"…the work committed in 593a936..59cfd4c"`),
and the only stored id↔spec edge is column 2 of `scripts/autonomy/plan.tsv`. Meanwhile the `A` prefix means
nothing — all 183 rows carry it, the plan's own header never defines it, and no `B` row has ever existed (the
`B1…B7` in the minilam gap register are table categories, a different document).

Two changes, one idea: **the row names its work, and the work's commit, in fields a program can read.**

## Requirements

**R1 — the spec number is the id for new rows.** For any row whose spec is numbered `>= 326`, its id MUST equal
that number (`326`, not `A188`). Rows written earlier keep their `A`-id, which stays a valid alias. Why a
boundary and not a bulk rename: 183 historical rows are done, and renaming them would orphan every session id
and every amem key that names one.

**R2 — a row can be annotated with what only the commit step knows.** `state.py annotate <id> --spec <path>
--commit <sha>` writes both fields, and refuses (non-zero, nothing written) when the spec file does not exist or
`git cat-file -e <sha>` does not know the commit. A field that can hold a lie is worse than a missing field.

**R3 — `list` is self-describing.** `state.py list` prints the id, the spec number, and the short sha, and marks
a row where either field is absent (`-`) instead of silently printing blanks.

**R4 — the row is checked against its sources.** `scripts/autonomy/check-row-truth.sh` walks the plan and, for
every row that carries a commit, asserts: the commit exists in git; the spec file exists; and the row's verify
column equals the spec's `## Exit Criteria`, command for command and in order. It exits non-zero naming each
violation. This is the rule that makes R2 safe — a denormalized field with no validator is a second opinion.

**R5 — the cycle fills them in.** After a successful commit, `cycle.sh` annotates the row, so no human has to
remember. A row that reached `done` without both fields is a defect the next `check-row-truth.sh` names.

## Files

- `scripts/autonomy/state.py` (`annotate`, `list`)
- `scripts/autonomy/cycle.sh` (one call after the commit)
- `scripts/autonomy/check-row-truth.sh` (new)
- `scripts/autonomy/check-ids.sh` (new)
- `scripts/autonomy/tests/row-names-its-work.test.sh` (new)

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| annotate writes spec and commit | a row + a real sha + a real spec | both fields present, `list` shows them |
| annotate refuses an unknown sha | `--commit deadbeef` | rc != 0, row unchanged |
| annotate refuses a missing spec | `--spec specs/999-nope.md` | rc != 0, row unchanged |
| list marks a row without them | a legacy row | `-` in both positions, never a blank |
| check-row-truth is green on a faithful row | row copied from its spec | rc 0 |
| check-row-truth reddens a drifted criterion | one word changed in the row's verify | rc != 0 naming the criterion |
| check-row-truth reddens a bogus sha | commit set to a sha git does not know | rc != 0 |
| check-ids accepts a legacy alias | spec 325, id `A187` | rc 0 |
| check-ids refuses `A188` for spec 326 | spec 326, id `A188` | rc != 0 |
| check-ids accepts `326` | spec 326, id `326` | rc 0 |

## Exit Criteria

- [ ] `bash scripts/autonomy/tests/row-names-its-work.test.sh 2>&1 | grep -qE "[0-9]+ ok, 0 FAIL"`
- [ ] `bash scripts/autonomy/check-row-truth.sh`
- [ ] `bash scripts/autonomy/check-ids.sh 2>&1 | tail -1 | grep -q "row-ids:"`
- [ ] `python3 scripts/autonomy/state.py list | awk -F'\t' '$1=="A187" && $7=="325"' | grep -q A187`
- [ ] `bash scripts/autonomy/run-tests.sh 2>&1 | tail -1 | grep -qE "[0-9]+ passed, 0 failed"`

## Measured while writing this (2026-10-31)

Applied to the whole plan, the criteria comparison finds that **76 of 183 rows cannot be reconciled with
today's spec text**: 30 are a dropped tail (the row's commands are a prefix of the spec's criteria — A154 lost
the `coder-demo.toml` grep entirely) and 46 differ another way. None of these are judgeable as defects, because
a spec amended *after* its item ran looks identical to a row that dropped a criterion — which is exactly why
the checker judges a committed row against `git show <sha>:specs/…` (the contract it was actually held to) and
refuses to judge a finished row that named no commit. The permanent fix for the class is deriving the verify
column instead of copying it (change 3 of the agreed plan).

## Guardrails

- Do NOT rename historical rows. The `A`-ids are aliases that sessions and memory keys already point at.
- Do NOT write `commit` without proving the sha exists: a wrong sha is indistinguishable from a right one
  until someone tries to read it.
- Do NOT let the row's verify column drift from the spec: `check-row-truth.sh` compares commands, not prose,
  and it compares them in order.
- Do NOT add a composite id such as `322-A186`: `A186` has no row at all (measured), so half of that reference
  names nothing, and a re-opened spec would produce two references for one unit of work.

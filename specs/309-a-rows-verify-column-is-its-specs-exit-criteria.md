# Spec 309: a row's verify column is its spec's exit criteria, checked before dispatch

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the queue's own reliability. `scripts/autonomy/plan.tsv` states the rule in its own header
(`plan.tsv:10`): "The verify column MUST be the spec's own Exit Criteria, copied verbatim — same commands, same
sense." Nothing enforces it. The row's column 5 is hand-written from a reading of the spec, and it is the only
thing that decides whether an item passed: a criterion dropped from the column is a gate nobody runs, and a
criterion paraphrased stricter re-fails a correct worker for ever.

**Measured 2026-10-28, on this tree:** of 169 rows, 92 carry exactly their spec's `## Exit Criteria` and **77 do
not** (74 `done`, 2 `skipped`, 1 `revoked` — none open). The drift is silent because the only reader that compares
the two is a human; the cycle runs column 5 and never opens the spec's criteria. The cost is already recorded in
this repository: A8 failed twice on a hand-written `! grep -q Dockerfile` that was STRICTER than spec 141's
criterion, against a document that was right; A9/A11/A12 had each dropped one of their spec's criteria; and
A55/188 carried 5 criteria that the `;`-joined column split into 6 commands, whose first command's failure the
shell discarded.

Spec 283 closed the sibling gap for column 6 (the commit paths, compared with the spec's `## Files` block before a
worker is bought). This spec closes column 5 with the same seam, the same output shape and the same teeth.

## Requirements
### Functional
- **R1. The two declarations are compared, never inferred.** A spec's exit criteria are its `## Exit Criteria`
  block: lines `- [ ] \`command\``, in order, one command each. The row's column 5 is the same commands in the same
  order, joined with `;` (the separator `cycle.sh` and this checker both split on). The comparison is by exact
  string after trimming leading and trailing whitespace; nothing else is normalised, and no criterion is ever
  rewritten.
- **R2. One reader, no guessing.** New `scripts/autonomy/row-verify.py` (Python 3, stdlib only; root discovery and
  `PII_PLAN` handling as `scripts/autonomy/state.py`):
  - `criteria <spec.md>` → the spec's criteria, one per line, in order; exit 0. A spec with no `## Exit Criteria`
    block, or an empty one, → `NO-CRITERIA <spec>` on stdout, exit 2. A spec whose block holds a line that is not
    the house shape (a `- [ ]` with no single backticked command) → `NOT-A-CRITERION <line>` , exit 3.
  - `check <row-id>` → `OK <id> <n>` and exit 0 when column 5 equals the spec's criteria exactly; otherwise the
    differing indices, one per line — `COUNT spec=<n> row=<m>`, `DIFF <i> spec=<command> row=<command>` — and exit
    1. A row whose spec cannot be read, declares no criteria, or declares one the column cannot carry (R3) is
    never reported `OK`.
  - `judge <row>` (the same contract `row-paths.py` exposes: `(rc, lines)`) so `state.py` can call it in-process.
  - `check-all` → one line per row: `OK <id> <n>` or `REFUSED <id>: <why>`; exit 1 if any **dispatchable** row (a
    state that is not `done`, `skipped`, `revoked`) is refused, 0 otherwise. A closed row that differs is printed so
    the drift stays visible and does not turn the check red. Read-only: it writes nothing.
- **R3. A criterion the column cannot carry is refused, not split.** The column joins with `;`, so a criterion whose
  command itself contains an unquoted `;` cannot round-trip: the split turns one criterion into two, and the first
  one's failure is discarded by the shell (`cmd1; cmd2` returns cmd2's status — measured on A55/188). `check` and
  `judge` refuse such a row with `SPLIT <i>` and exit 1, and the message names the repair: join the commands with
  `&&` inside one criterion.
- **R4. The teeth are at the dispatch boundary.** `scripts/autonomy/state.py`'s `pick` calls `row-verify.judge(r)`
  next to the existing `row-paths.judge(r)` (`state.py:86-92`), in the same place and with the same channel: a row
  that fails prints `refused <id>: <why>` to **stderr** and is skipped, so the next dispatchable row is offered and
  the stdout contract (one TSV row or `NONE`) does not change. `state.py` is executed by `cycle.sh` on every tick,
  so it is written through a temp file and `mv`'d into place.
- **R5. Tests that could fail, with controls.** New `scripts/autonomy/tests/row-verify-test.sh`: every row builds its
  OWN scratch directory under `_tmp/` (created by the test, never `/tmp`), writes a fixture `plan.tsv` and a fixture
  spec, runs the real `scripts/autonomy/row-verify.py` against them, and prints `PASS <row>` or `FAIL <row>: <why>`;
  the last line is `RESULT: PASS` when every row passed, else `RESULT: FAIL <n>`, and the script exits non-zero on
  any failure. Rows:
  - (a) `a-paraphrased-criterion-is-refused`: one command rewritten → rc=1, `DIFF 2` naming both sides.
  - (b) `a-dropped-criterion-is-refused`: the row carries two of the spec's three → rc=1, `COUNT spec=3 row=2`.
  - (c) `a-reordered-row-is-refused`: same three commands, moved → rc=1.
  - (d) `CONTROL-an-exact-row-is-accepted`: the same commands, same order → rc=0, `OK <id> 3` — the near miss.
  - (e) `CONTROL-trailing-whitespace-is-not-a-difference`: the row's commands padded with spaces → rc=0, so the
    check cannot re-fail a correct row for a formatting accident.
  - (f) `a-spec-without-criteria-is-refused`: the spec has no `## Exit Criteria` → rc=2 and never `OK`.
  - (g) `a-criterion-holding-the-separator-is-refused`: a criterion written `grep -q A; grep -q B` → `SPLIT 2`, rc=1.
  - (h) `pick-refuses-the-mismatched-row-and-offers-the-next`: a fixture plan of two rows, the first mismatched and
    the second exact; the row runs the REAL `scripts/autonomy/state.py pick` with `PII_PLAN` pointing at the
    fixture, asserts stderr carries `refused <id1>`, stdout is the second row's TSV, and the mismatched row's state
    is untouched.
- **R6. Nothing already closed is retro-failed.** The check runs on dispatchable rows only (R4); the 77 closed rows
  that differ today stay closed and are reported, never rewritten. No row's column is edited by this change, and no
  spec's criteria are edited by it: this spec adds a reader, a refusal and their tests.

### Non-Functional
- Python 3 stdlib only, no Rust and no new crate; `row-verify.py` under 200 lines; the test script under 450 lines;
  `bash scripts/autonomy/selftest.sh` gains no row from this change and must stay green.

## Verified Premises
- **CONFIRMED** `scripts/autonomy/plan.tsv:10` carries the rule as a comment, and `state.py:72-108` is `pick`: it
  walks the plan in order, skips parked rows to stderr, refuses a spec-283 row via `rowpaths.judge(r)`
  (`state.py:86-92`), and prints `NONE` when nothing is dispatchable.
- **CONFIRMED** `scripts/autonomy/row-paths.py` exists (3679 bytes) with a `judge(row)` returning `(rc, lines)`, and
  `scripts/autonomy/tests/row-paths-test.sh` (4213 bytes) is the house test shape this spec's test follows.
- **MEASURED 2026-10-28** by reading every row against its spec: 169 rows; 92 equal, 77 differ (74 done, 2 skipped,
  1 revoked); 0 open rows differ. No criterion in any current spec contains a `;` — the splitter hazard is
  historical (A55/188), which is why R3 must be enforced by a test rather than assumed away.
- **CONFIRMED** the `;` split is the column's real contract: `cycle.sh` runs `IFS=';' read -ra` on column 5, so
  `cmd1; cmd2` inside one criterion returns `cmd2`'s status and discards the first failure.
- **NOT CLAIMED**: that the 77 closed rows' columns were wrong when they ran (some predate the house shape and some
  specs' criteria were amended after the commit). This spec does not re-adjudicate them; it stops the next one.

## TDD Contract
| Test (`scripts/autonomy/tests/row-verify-test.sh`) | Input | Expected |
|---|---|---|
| `a-paraphrased-criterion-is-refused` | spec criterion `grep -q 'a b'`, row `grep -q 'a  b'` | rc=1, `DIFF` names index 2 and both strings |
| `a-dropped-criterion-is-refused` | spec 3 criteria, row 2 | rc=1, `COUNT spec=3 row=2` |
| `a-reordered-row-is-refused` | the same 3 commands in another order | rc=1, at least one `DIFF` |
| `CONTROL-an-exact-row-is-accepted` | the same 3 commands in order | rc=0, `OK <id> 3` |
| `CONTROL-trailing-whitespace-is-not-a-difference` | the row's commands padded with spaces | rc=0, `OK` |
| `a-spec-without-criteria-is-refused` | a spec with no `## Exit Criteria` block | rc=2, `NO-CRITERIA`, never `OK` |
| `a-criterion-holding-the-separator-is-refused` | criterion 2 is `grep -q A; grep -q B` | rc=1, `SPLIT 2` |
| `pick-refuses-the-mismatched-row-and-offers-the-next` | fixture plan: row1 mismatched, row2 exact | stderr `refused row1`, stdout row2's TSV, row1's state unchanged |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/row-verify-test.sh 2>&1 | tail -1 | grep -q 'RESULT: PASS'`
- [ ] `uv run --no-project python scripts/autonomy/row-verify.py check A170 | grep -q '^OK A170 9$'`
- [ ] `uv run --no-project python scripts/autonomy/row-verify.py criteria specs/306-every-call-has-an-entry-counter-so-no-subgraph-is-entered-without-bound.md | grep -c . | grep -qx 9`
- [ ] `uv run --no-project python scripts/autonomy/row-verify.py check-all >/dev/null`
- [ ] `grep -q 'row-verify' scripts/autonomy/state.py`
- [ ] `git diff --quiet -- specs scripts/autonomy/plan.tsv`

## Files
- `scripts/autonomy/row-verify.py`
- `scripts/autonomy/state.py`
- `scripts/autonomy/tests/row-verify-test.sh`

## Guardrails
- Do NOT edit `specs/`, `scripts/autonomy/plan.tsv`, an existing assertion, or any row's column. The spec and the row
  are the author's; this change is the reader and the refusal.
- Do NOT normalise a real difference away to make a row pass: whitespace at the edges is the ONLY tolerance, so a
  changed argument, a dropped criterion or a reordering stays red.
- Do NOT make the check advisory. It refuses at `pick` (R4), which is the boundary where a worker would be bought;
  a warning printed by a cycle that then dispatches is the defect this spec exists to remove.
- If the house shape differs from what is written here (for example `pick` no longer imports `row-paths` at
  `state.py:86-92`), STOP and say so in `## Closure` instead of forcing the change onto a different seam.
- Scratch under `_tmp/` inside the repository; never `/tmp`.

## Closure

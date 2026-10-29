# Spec 312: a row whose worker stopped early is offered again, not parked for ever

## Overview
**Project**: /projects/pi-improved
**Phase**: production — unattended queue. The queue's own contract says a worker that stops early is resumed:
`scripts/autonomy/cycle.sh:301` prints `OUTCOME=worker-rc-$WRC … leaving the WIP for the next cycle to resume`, and
the operating procedure's outcome table reads **rc 8 `worker-rc-N` → "the worker stopped early; its session is in
state.json → the next tick RESUMES it"**. No tick ever resumes it.

**Measured 2026-10-29.** `cycle.sh:300` records `status=needs-review … last=worker-rc=$WRC` with the session, and
`state.py`'s `pick` offers a `needs-review` row **only** when its `last` is `window-closed` and it holds a session
(spec 280, `state.py:77-84`). Every other `needs-review` row is skipped as "waiting for a person"
(`state.py:80-84`, spec 169), so a row that ended `worker-rc≠0` is never offered again by the loop.

The cost, twice in one night:
- A173 (spec 309) and A174 (spec 310) ended `worker-rc` at 21:49Z and 22:03Z on 2026-10-28 with their sessions and
  their work in the tree; **21 consecutive cycles** then printed `queue empty — nothing pending` (23:01:58Z →
  00:19:56Z, `_tmp/autonomy/logs/cycle-last-detached.log`) while both rows sat dispatchable-with-a-session.
- A175 (spec 311) ended `worker-rc=1` at 00:39:52Z on 2026-10-29; the next cycle at 00:40:51Z reported
  `pick: every open row is parked: A175(needs-review) - waiting for a person`. It advanced only because a hand set
  the row back to `pending` — the loop cannot do that for itself.

**Second defect on the same line.** `cycle.sh:300` does not increment `attempts`. The attempt cap at
`cycle.sh:210-215` (`attempts >= 3` → park for a person) reads the counter `pick` prints, so with the counter never
raised a row that keeps failing `worker-rc≠0` would be resumed for ever — the cap that exists precisely for this
case can never fire.

This spec implements the contract the operator already reads, and makes the existing cap reachable. It does **not**
make any other kind of parked row dispatchable: a dead login (`last=lane-auth-failed`) and a `parked` row are a
person's, and they stay skipped.

## Verified Premises
- **CONFIRMED (read 2026-10-29)** `scripts/autonomy/state.py:77-84`: `window = s == "needs-review" and
  st[r["id"]].get("last") == "window-closed" and st[r["id"]].get("session")`; then
  `if s in ("parked", "needs-review") and not window: skipped.append(...); continue`. The dispatch condition is
  `if s in ("pending", "in-progress") or window`.
- **CONFIRMED (read)** `scripts/autonomy/cycle.sh:300`: `python3 scripts/autonomy/state.py set "$ITEM"
  status=needs-review "session=${NEWSID:-}" last=worker-rc=$WRC` — no `attempts=`; `:301` prints the resume promise
  and exits 8.
- **CONFIRMED (read)** `scripts/autonomy/cycle.sh:210-215`: `if [ "${ATTEMPTS:-0}" -ge 3 ]; then … status=parked
  attempts="$ATTEMPTS" … OUTCOME=parked-attempt-cap … exit 11`. `ATTEMPTS` is field 8 of the pick TSV
  (`cycle.sh:106`), so the cap is only reachable if the worker-rc path raises it.
- **CONFIRMED (read)** `scripts/autonomy/cycle.sh:182-199`: a resume whose stored `last` matches
  `*gate-failed*|*verify-failed*|*worker-rc*` builds a corrective brief and passes `--resume`; a `worker-rc` last is
  **not** a phase marker (`:137-140` matches only `verified|gated|worker-ok`), so an offered row is re-dispatched to
  its own session rather than sent straight to verify. R1 alone therefore delivers the documented behaviour.
- **CONFIRMED (measured, log)** `_tmp/autonomy/logs/cycle-last-detached.log` holds the 21 `queue empty` cycles of
  2026-10-28 23:01Z → 2026-10-29 00:19Z, each after an `OPEN` probe, and the `A175(needs-review)` line at 00:40:51Z.
- **CONFIRMED (read)** two fixtures in `scripts/autonomy/tests/pick-resumes-window-closed.test.sh` encode the old
  contract with the input `last=worker-rc=9` (lines 40-42 and line 73). They must change **input**, not assertion:
  the control they express is "a needs-review row whose last names a person's problem stays skipped", and
  `lane-auth-failed` carries exactly that meaning after this spec.
- **NOT CLAIMED**: that a resumed worker succeeds. Only that the row is offered again with its session, and that
  three failures park it for a person.

## Requirements
### Functional
- **R1. `pick` offers a row whose worker stopped early.** In `scripts/autonomy/state.py`, a row with
  `status == "needs-review"` **and** a `last` that starts with `worker-rc=` **and** a non-empty `session` is
  dispatchable exactly like a `pending` row: same plan-order position, same TSV (id, spec, brief, title, verify,
  paths, gate, attempts, session), the same `pick: skip parked … -> dispatching …` note when a parked row precedes
  it. Every other `needs-review` row — including `last=worker-rc=…` with an **empty** session and
  `last=lane-auth-failed` with a session — stays skipped with the same stderr note, and `parked` rows are untouched.
  Update the `pick` docstring line to say it. `pick` stays read-only: it must not write the state file.
- **R2. The worker-rc path counts its attempts.** In `scripts/autonomy/cycle.sh`, the `set` on the worker-rc path
  carries `attempts=$((ATTEMPTS+1))` alongside the existing status, session and `last=worker-rc=$WRC`. The exit code
  (8), the `say` line and the `echo OUTCOME=worker-rc-…` text do not change, and the cap at `:210-215` is not
  touched: three worker runs, then a fourth pick parks the row for a person.
- **R3. A test with controls, reading the live text.** New `scripts/autonomy/tests/pick-resumes-worker-rc.test.sh`
  (bash, `set -u`, scratch under `_tmp/` of this repo removed by its own trap, one `ok <name>` / `FAIL <name>` line
  per assertion and a final `<N> ok, <M> FAIL`, exit 0 only when `M` = 0). Part A runs the **real**
  `uv run --no-project python scripts/autonomy/state.py pick` with `PII_STATE` and `PII_PLAN` pointed at scratch
  files it writes. Part B extracts the worker-rc block **from `cycle.sh` as text at run time** (`sed -n` from the line
  carrying `last=worker-rc=` to the line printing `OUTCOME=worker-rc-`, closed with its `}`), the pattern
  `tests/cycle-requeue-outcome.test.sh` established, and runs it under `set -u` with stubs for `python3` (records its
  argv) and `say`, with `ITEM`, `ATTEMPTS=0`, `NEWSID=sid-9`, `WRC=1` — so a change to `cycle.sh` is followed by the
  check, not bypassed.
- **R4. The superseded fixtures change input only.** In `scripts/autonomy/tests/pick-resumes-window-closed.test.sh`,
  the two fixtures whose input is `last=worker-rc=9` (the `(b)` control at lines 40-42 and the `X2` of case `(h)` at
  line 73) take `last=lane-auth-failed` instead. Every assertion string, comparison and count stays byte-identical:
  the control still says "another needs-review row is not offered", it just names a status that remains a person's.
- **R5. Nothing else moves.** No other rule in `pick` changes, no other `needs-review` status becomes dispatchable,
  and `tick.sh`, `window.sh`, `leftover.sh`, `row-paths.py`, `row-verify.py` are not edited.

### Non-Functional
- `pick` stays a single read of the state file and prints exactly what it printed before for every input it already
  accepted (a `pending` row's TSV is byte-identical).
- The new test completes in seconds and needs no network, no daemon, no worker and no gate.

## TDD Contract
| Test (`scripts/autonomy/tests/pick-resumes-worker-rc.test.sh`) | Input | Expected |
|---|---|---|
| `a-worker-rc-row-is-offered` | `X1` needs-review, `last=worker-rc=1`, `session=S1` | `pick` prints a line whose field 1 is `X1`, whose field 9 (last) is `S1`; rc 0 |
| `the-attempts-travel-with-the-row` | same, `attempts=2` | field 8 of the printed row is `2` |
| `CONTROL-no-session-is-not-resumable` | `last=worker-rc=1`, `session=""` | `NONE` — a resume handle that does not exist is a person's |
| `CONTROL-a-dead-login-waits-for-a-person` | `last=lane-auth-failed`, `session=S1` | `NONE` |
| `CONTROL-a-parked-row-stays-skipped` | `status=parked`, `last=worker-rc=1`, `session=S1` | `NONE` |
| `order-is-plan-order` | a `pending` `X0` before the worker-rc row, then the worker-rc row first | `X0` first; then `X1` first |
| `a-done-row-is-never-offered` | `status=done`, `last=worker-rc=1`, `session=S1` | `NONE` |
| `the-cycle-counts-the-attempt` | the real worker-rc block of `cycle.sh`, `ATTEMPTS=0`, `WRC=1` | the recorded `state.py` argv holds `attempts=1` and `session=sid-9`; the block's rc is 8 |
| `CONTROL-a-block-that-drops-the-attempt-is-caught` | the same block with `attempts=$((ATTEMPTS+1))` removed | the recorded argv holds **no** `attempts=` — so the assertion above reads the live text and is not a fixture of itself |
| `pick-does-not-write-the-state` | the `(a)` fixture, `sha256sum` of the state file before and after `pick` | identical |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/pick-resumes-worker-rc.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `! bash scripts/autonomy/tests/pick-resumes-worker-rc.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `bash scripts/autonomy/tests/pick-resumes-window-closed.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `bash scripts/autonomy/tests/cycle-disk-wiring.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `uv run --no-project python -m py_compile scripts/autonomy/state.py`
- [ ] `bash -n scripts/autonomy/cycle.sh`

## Files
- `scripts/autonomy/state.py`
- `scripts/autonomy/cycle.sh`
- `scripts/autonomy/tests/pick-resumes-worker-rc.test.sh`
- `scripts/autonomy/tests/pick-resumes-window-closed.test.sh`

## Guardrails
- Do NOT edit `specs/`, `scripts/autonomy/plan.tsv`, or any existing assertion's TEXT, comparison or expected value.
  R4 changes two fixtures' INPUT (`worker-rc=9` → `lane-auth-failed`) and nothing else; any other edit to that file
  is out of scope and must be reported instead of made.
- Do NOT make any other `needs-review` status dispatchable, and do NOT touch `parked` rows. The defect is one dead
  rule; a blanket "offer every needs-review row" reproduces spec 169's failure (a row waiting for a person holds the
  queue behind it).
- Do NOT raise, remove or move the attempt cap, and do NOT change the `exit 8` code or the `OUTCOME=worker-rc-…`
  text: a caller reads both.
- Do NOT write to the real `_tmp/autonomy/state.json` from a test; every case sets `PII_STATE` and `PII_PLAN`.
  Scratch lives under `_tmp/` inside this repository, never `/tmp`.
- **If `cycle.sh`'s worker-rc block no longer matches the shape described here** (for example the `last=worker-rc=`
  set is gone, or `ATTEMPTS` no longer comes from the pick TSV), STOP and say so in `## Closure` instead of forcing
  the change onto a different seam.
- If `scripts/autonomy/selftest.sh` regresses, that is a finding for the orchestrator's review, not a reason to
  weaken a criterion: record both readings in `## Closure`.

## Follow-ups (not this spec)
1. `scripts/autonomy/tick.sh:137` still texts `pii autonomy · <item> needs a hand (worker-rc-N)` for every rc-8
   cycle. With this spec the loop resumes those rows, so the message should distinguish "resumed" from "parked".
2. A `worker-rc` row with **no** session cannot be resumed and stays parked; a fresh re-dispatch after the cap, with
   the leftover stashed (spec 311), is the next question — and it must respect the same three-attempt ceiling.

## Closure
(pending — the orchestrator records both readings here: the six criteria on the committed tree, and
`bash scripts/autonomy/selftest.sh`'s verdict, plus the mutation that reddens the new test and its byte-identical
restore, before this spec is closed.)

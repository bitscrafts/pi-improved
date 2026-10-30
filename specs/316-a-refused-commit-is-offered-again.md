# Spec 316: a refused commit is offered again, keeps its phase marker, and reaches the phone

## Overview
**Project**: /projects/pi-improved
**Phase**: production — unattended queue. Spec 315 put a build gate between verify and `git commit`: when the
pending tree does not build, `cycle.sh` does not commit, sets the row `status=needs-review` with
`last=precommit-broken: <first error>`, and exits 18 with `OUTCOME=precommit-broken`, deliberately leaving the index
staged "so the next cycle resumes at verify instead of re-buying the spec".

Three things make that sentence false today. All three are measured on the committed tree at `e8a04f2`:

- **The row is never offered.** `scripts/autonomy/state.py:81` offers a `needs-review` row again only when its `last`
  is `window-closed` or starts with `worker-rc=` (specs 280, 312). A `precommit-broken` row with a session is
  skipped, so `pick` prints `NONE` and every cycle reports `queue empty` while the worker's work sits staged and
  unverifiable — the same stranding spec 312 closed for `worker-rc`, re-opened by a new outcome.
- **The phase marker is lost.** `cycle.sh:139` resumes at verify only when `last` carries
  `*verified*|*gated*|*worker-ok*`. The precommit block (`cycle.sh:427-437`) writes `last=precommit-broken: …`, which
  overwrites the `worker-ok` marker `cycle.sh:312` wrote, so even a row the queue does offer is re-dispatched into the
  worker instead of resuming at verify. The house already knows the shape: `cycle.sh:386` writes
  `last=worker-ok; gate-timeout after …` precisely to keep the marker.
- **The phone is silent.** `scripts/autonomy/tick.sh:135-147` has no arm for `precommit-broken`, so an operator hears
  nothing about a refused commit; `grep -c precommit scripts/autonomy/tick.sh` returns **0**.

## Verified Premises
- **MEASURED (2026-10-30, this tree at `e8a04f2`, scratch state/plan/specs, the real `state.py pick`)** — a row
  `{"status":"needs-review","last":"precommit-broken: error[E0425]: cannot find function","session":"S1"}` prints
  `NONE`; the same row with `last` `worker-rc=1` prints `X1`; with
  `last="precommit-timeout — the check was KILLED, not failed"` it prints `NONE`. So the stranding is the pick rule
  and nothing else.
- **READ (2026-10-30)** `cycle.sh:138-140` sets `SKIP_DISPATCH=1` on `*verified*|*gated*|*worker-ok*` only, and
  `cycle.sh:427-437` writes `last=precommit-broken: ${PCSYM:-…}` / `last=precommit-timeout — …` with no marker
  prefix. `cycle.sh:386` is the precedent that keeps it (`last=worker-ok; gate-timeout after ${GATE_TIMEOUT}s — …`).
- **MEASURED (2026-10-30)** `grep -c precommit scripts/autonomy/tick.sh` = 0; the `case "$OUT"` block at
  `tick.sh:135-147` ends at `esac`, so an unlisted outcome prints nothing. Spec 313 already gave the tick the
  mechanism this spec reuses: `NEXT=$(python3 scripts/autonomy/state.py pick …)` is the row the next cycle will
  dispatch, first field the id.
- **CONFIRMED (2026-10-30)** `row-paths.py` refuses a `## Files` entry only when it *is a directory* or the block is
  missing; a file the worker is about to create is accepted, so this spec may declare its own new fixture.
- **NOT CLAIMED**: that the build failure is the worker's fault, that a resumed verify will pass, or that the
  precommit check is the right gate. This spec only makes the queue apply its own stated rule to this outcome.

## Requirements

- **R1. One resume predicate.** `scripts/autonomy/state.py` gains a module-level
  `def resumable(status, row)` returning a bool, and `pick` calls it in place of the inline expression at line 81.
  It is **True** iff `status == "needs-review"`, `row.get("session")` is non-empty, and `row.get("last","")` is
  `window-closed` or starts with `worker-rc=`, `precommit-broken` or `precommit-timeout`. It is **False** for every
  other status — `parked`, `done`, `skipped`, `revoked`, `pending`, `in-progress` — and for a `needs-review` row with
  no session. The behaviour of the two shipped prefixes does not change.
- **R2. The docstring follows.** The `state.py` module docstring's `pick` line names the precommit outcomes next to
  the window and `worker-rc=N` ones, so the file's own description is not stale.
- **R3. The refusal keeps the phase marker.** In `cycle.sh`'s precommit block the two `last=` values become
  `worker-ok; precommit-broken: <first error line>` and
  `worker-ok; precommit-timeout — the check was KILLED, not failed`, matching `cycle.sh:386`'s shape. Nothing else in
  the block changes: same `status=needs-review`, same `OUTCOME=precommit-broken`, same exit **18**, `attempts`
  untouched, the index still left staged, the helper still called through `PRE_COMMIT_BUILDS` under `timeout`.
- **R4. The phone learns the outcome.** `tick.sh`'s `case "$OUT"` gains exactly one arm, `precommit-*)`, and inside
  it the same `NEXT` test the `worker-rc-*` arm uses (`[ -n "$ITEM" ] && [ -n "${NEXT:-}" ] && [ "${NEXT%%$'\t'*}" = "$ITEM" ]`):
  - resumed → `say_once "regate:$ITEM:$OUT" "pii autonomy · $ITEM re-gating with its session (the commit was refused: the pending tree did not build)" "log: ${LASTLOG##*/}"`
  - not resumed → `say_once "bad:$ITEM:$OUT" "pii autonomy · $ITEM's commit was refused and no cycle can resume it — needs a hand ($OUT)" "log: ${LASTLOG##*/}"`

  The two keys differ, and neither sentence claims the item is done — the loop has not verified anything yet.
- **R5. Edits reach a running loop safely.** `cycle.sh`, `tick.sh` and `state.py` are written as a complete new file
  and moved into place with `mv` (never rewritten in place): a cycle is executing `cycle.sh` while this worker edits
  it, and a script rewritten under a live cycle has already killed one (DO-NOT-REPEAT 2.15). `state.py` already
  ships that way.
- **R6. A fixture that drives the live paths.** New `scripts/autonomy/tests/precommit-refusal.test.sh`, house style
  (`#!/usr/bin/env bash`, `set -u`, `cd "$(dirname "$0")/../../.."`, `ok`/`FAIL` lines, last line
  `N ok, M FAIL`, scratch under `_tmp/` removed by its own trap):
  - **Part A** runs the real `state.py pick` against scratch `PII_STATE`/`PII_PLAN`/`PII_SPECS` files written by the
    fixture (the shape of `pick-resumes-worker-rc.test.sh`, including its own scratch spec with a `## Files` line and
    an `## Exit Criteria` line so both row judges pass).
  - **Part B** extracts the live `case "$OUT"` block from `tick.sh` at run time and runs it with a recording
    `say_once` (the shape of `tick-message-worker-rc.test.sh`) — it never re-types the sentences.
  - **Part C** reads `cycle.sh`: the `last=` literals the precommit block writes are extracted from the file and
    matched against the phase-marker glob taken from the live `SKIP_DISPATCH` case line — both sides read from the
    script at run time, so the check cannot certify a shape the file no longer has.
- **R7. Nothing else moves.** No change to `pre-commit-builds.sh`, `quality-gate.sh`, `gate-steps*.txt`, `plan.tsv`,
  the three-attempt cap, or anything under `specs/`.

### Non-Functional
- Every new file stays under 450 effective lines; the fixture under 200.
- The fixture runs in under ~10 s and buys nothing: no cycle, no worker, no `cargo`, no network.
- `state.py` keeps one code path for the resume decision — a second copy of the rule is the defect this spec removes.

## TDD Contract
`scripts/autonomy/tests/precommit-refusal.test.sh` prints `N ok, M FAIL` as its last line and exits 0 only when the
FAIL count is 0. Each row names what it drives; the controls are the near misses that must keep today's behaviour.

| Test | Input | Expected |
|---|---|---|
| `a-precommit-broken-row-is-offered` | Part A, `needs-review` / `last=precommit-broken: error[E0425] …` / `session=S1` | `pick` prints the row, field 9 `S1`, rc 0 |
| `a-precommit-timeout-row-is-offered` | Part A, `last=precommit-timeout — the check was KILLED, not failed` | the row is printed |
| `CONTROL-no-session-is-not-resumable` | the same row with `session=""` | `NONE` |
| `CONTROL-a-parked-row-stays-parked` | `status=parked` with the same `last` and a session | `NONE` |
| `CONTROL-an-auth-failure-waits-for-a-person` | `needs-review`, `last=lane-auth-failed`, `session=S1` | `NONE` |
| `CONTROL-a-worker-rc-row-is-still-offered` | `needs-review`, `last=worker-rc=1`, `session=S1` | the row is printed (the shipped rule is not narrowed) |
| `CONTROL-a-window-closed-row-is-still-offered` | `needs-review`, `last=window-closed`, `session=S1` | the row is printed |
| `the-row-is-not-blocked-behind-a-pending-one` | plan `X1` (precommit-broken) then `X2` (pending) | `X1` is the pick, not `X2` |
| `the-refusal-that-will-be-retried-says-re-gating` | Part B, `OUT=precommit-broken`, `ITEM=A1`, `NEXT=A1` | key `regate:A1:precommit-broken`, one line containing `re-gating with its session` |
| `CONTROL-a-refusal-nobody-can-resume-asks-for-a-hand` | Part B, `NEXT=NONE` | key `bad:A1:precommit-broken`, one line containing `needs a hand` |
| `CONTROL-the-two-keys-differ` | the two runs above | keys differ |
| `the-live-block-has-the-precommit-arm` | the extracted block | contains `precommit-` and still every outcome it had (`done`, `verify-failed`, `gate-failed`, `worker-rc-`, `dirty-tree`, `bad-plan`, `window-closed`) |
| `the-cycle-writes-a-marker-the-glob-matches` | Part C, the live `last=` literal + the live `SKIP_DISPATCH` glob | the literal matches the glob |
| `CONTROL-the-unmarked-literal-the-spec-removes-does-not-match` | Part C, `precommit-broken: error[E0425] …` (the shape 315 shipped) | does **not** match the glob — the check discriminates |
| `CONTROL-the-house-precedent-matches` | Part C, `worker-ok; gate-timeout after 3600s — …` | matches the glob |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/precommit-refusal.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `! bash scripts/autonomy/tests/precommit-refusal.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `bash -n scripts/autonomy/cycle.sh && bash -n scripts/autonomy/tick.sh`
- [ ] `uv run --no-project python -m py_compile scripts/autonomy/state.py`
- [ ] `[ "$(grep -c 'resumable(' scripts/autonomy/state.py)" -ge 2 ]`
- [ ] `grep -qF 'precommit-*)' scripts/autonomy/tick.sh`
- [ ] `grep -qE "last=worker-ok[[:punct:]][[:space:]]precommit-broken" scripts/autonomy/cycle.sh`
- [ ] `grep -qE "last=worker-ok[[:punct:]][[:space:]]precommit-timeout" scripts/autonomy/cycle.sh`
- [ ] `git diff --quiet -- scripts/autonomy/plan.tsv quality-gate.sh gate-steps.txt gate-steps-quick.txt scripts/autonomy/pre-commit-builds.sh`

## Files
- `scripts/autonomy/state.py`
- `scripts/autonomy/tick.sh`
- `scripts/autonomy/cycle.sh`
- `scripts/autonomy/tests/precommit-refusal.test.sh`

## Guardrails
- Do NOT weaken a criterion, an assertion or a fixture to make this pass. If the mandated shape cannot be met
  without a design change, STOP and say which premise is wrong — a worker that flags a wrong premise is right, and
  the orchestrator will correct the brief.
- Do NOT rewrite `cycle.sh`, `tick.sh` or `state.py` in place: write the new file, then `mv` it over (R5).
- Do NOT change `attempts`, the three-attempt cap, the `parked` transition, the exit code 18, or leave the index
  unstaged on a refusal.
- Do NOT add a sleep, raise a timeout, or make any check skip.
- Do NOT touch `pre-commit-builds.sh`, `quality-gate.sh`, `gate-steps*.txt`, `plan.tsv` or `specs/`.
- Never `rm -rf`; the fixture removes only its own `_tmp/` scratch, through `find "$T" -depth -delete` in its trap.

## Closure

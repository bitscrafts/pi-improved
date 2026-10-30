# Spec 318: a row whose gate or criteria failed is offered again, with its session

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the loop must not idle while a person is away (user, 2026-10-29: "nao pode parar!!! se para
deixa de ter efeito!")

Measured on 2026-10-30: item **A181** (spec 317) finished its worker, failed the gate, and the row was written
`status=needs-review last=gate-failed` with its session. From 11:29Z until an orchestrator repaired it by hand every
cycle printed

```
pick: every open row is parked: A181(needs-review) - waiting for a person
queue empty — nothing pending
```

— roughly seven hours in which the queue had exactly one item and nothing was dispatched, because `state.py`'s
`resumable()` (lines 46-51) offers a `needs-review` row again only for `window-closed`, `worker-rc=`,
`precommit-broken` and `precommit-timeout`. `verify-failed` has the same hole. Both of those are outcomes the loop
itself produces (`cycle.sh:331` and `cycle.sh:390`) and both are outcomes the loop can act on: a failed gate is
re-run, a failed criterion is re-attempted with the worker's own session, and `attempts` already carries the
three-strike cap that parks what keeps failing.

The second half of the defect is what a repair without the marker would cost. `cycle.sh:139` resumes at verify only
when the row's `last` matches `*verified*|*gated*|*worker-ok*`. The gate branch writes a **bare** `last=gate-failed`,
so even once `pick` offers the row the cycle would fall through to the dispatch branch and buy a worker for work
that is already in the tree — the exact defect spec 316 fixed for `precommit-broken`, whose house precedent
(`cycle.sh:387`, `last=worker-ok; gate-timeout after …`) is three lines above the branch this spec changes.

The third is the phone. `tick.sh`'s `verify-failed|gate-failed|dirty-tree|bad-plan)` arm says **"needs a hand"** for
outsourcing that the loop will in fact retry, so the operator is told to look at work that needs nobody — the class
spec 313 closed for `worker-rc` and 316 for a refused commit.

## Requirements
### Functional
- R1 (pick offers both rows). `state.py`'s `resumable(status, row)` offers a row again when `status == "needs-review"`,
  the row holds a non-empty `session`, and its `last` starts with `gate-failed` or `verify-failed` — alongside the four
  prefixes it already offers, whose behaviour does not change. A `parked` row is never offered, whatever its `last`,
  and every other `last` (`dirty-tree`, `bad-plan`, `unlisted-changes`, `lane-auth-failed`, `3-strikes`, a window
  marker that is not the resumable one) keeps waiting for a person. The function's comment names spec 318 with the
  others.
- R2 (the gate failure keeps its phase marker). `cycle.sh`'s gate-FAILED branch (the `else` at line 390, not the
  `124` timeout branch) writes
  `last=worker-ok; gate-failed: <the first failing step, when the log names one>` — the same marker-then-outcome shape
  as the timeout branch above it — so the next cycle's `SKIP_DISPATCH` arm matches and resumes at **verify**, without
  dispatching a worker. `attempts` still increments by one on that branch.
- R3 (a failed criterion stays a dispatch, and must not be marked). The verify branch's literal stays bare
  `last=verify-failed`: the work itself is what failed, so the correct resume is the worker's own session through the
  ordinary dispatch path (spec 312), with its corrective brief. This requirement exists to stop R2 being applied
  "helpfully" to both branches; the fixture asserts that the verify literal is **not** matched by the live
  `SKIP_DISPATCH` glob.
- R4 (the phone says which). `tick.sh`'s combined `verify-failed|gate-failed|dirty-tree|bad-plan)` arm splits:
  - `gate-failed`, when the queue's own `pick` names this item (`NEXT`'s first field equals `ITEM`): key
    `regate:$ITEM:$OUT`, line `pii autonomy · $ITEM re-gating with its session (the gate failed: nothing was spent on
    a worker)`, third line `log: <last log>`.
  - `verify-failed`, under the same condition: key `retry:$ITEM:$OUT`, line `pii autonomy · $ITEM re-implementing
    with its session (its criteria failed)`, third line `log: <last log>`.
  - Either of them with no item, or with an `ITEM` the queue does not name: the existing `bad:$ITEM:$OUT` /
    "needs a hand" text, unchanged.
  - `dirty-tree|bad-plan)` keeps exactly the arm it has today.
  Both new lines say the row **will be retried**; neither says the item is done.
- R5 (the cap is untouched). Both branches keep `attempts=$((ATTEMPTS+1))`, and the three-strike park
  (`status=parked`, `last=3-strikes`) keeps firing at 3 for both. A row parked that way carries no resumable prefix
  and stays out of `pick` — control in the fixture.

### Non-Functional
- Shell and Python only; no new dependency. Every file stays well under the 450-effective-line rule.
- The fixture never runs a cycle, a worker, `cargo`, the network or a paid model: Part A runs the real `state.py pick`
  against scratch state/plan/specs; Part B reads `tick.sh`'s live `case "$OUT" in` block as text at run time and runs
  it with a stub `say_once`; Part C reads `cycle.sh`'s literals and the live `SKIP_DISPATCH` glob from the same file.
  Scratch lives under this repo's `_tmp/` and the script's own `trap` removes it.
- A scratch spec under `_tmp/` supplies the row's `## Files` and `## Exit Criteria`, so the row judges of specs 283
  and 309 reach the rule under test instead of refusing every scratch row `NO-FILES` (the trap spec 312 paid for).

## Architecture
```
state.py  resumable()          + gate-failed, verify-failed   -> pick offers the row with its session
cycle.sh  gate-failed branch   last=worker-ok; gate-failed: … -> SKIP_DISPATCH matches -> resume at verify
cycle.sh  verify-failed branch bare last=verify-failed        -> no marker -> re-dispatch with the session (312)
tick.sh   gate-failed arm      regate:$ITEM:$OUT              -> "will be retried", not "needs a hand"
tick.sh   verify-failed arm    retry:$ITEM:$OUT               -> "will be retried", not "needs a hand"
```
Decisions: (1) the marker, not a second rule in `pick`, decides *how* the row resumes — `pick` only decides
*whether*. (2) `verify-failed` deliberately gets no marker, because a re-gate of a tree whose criteria fail would only
fail again and burn the attempts cap without touching the code; the worker's session is what can fix it. (3) The
phone keys follow spec 313/316 (`resumed:` / `regate:`), so a reader of the log can tell a retry from a park.

## TDD Contract
Acceptance test: `scripts/autonomy/tests/gate-refusal.test.sh`, in the shape of
`scripts/autonomy/tests/precommit-refusal.test.sh` (spec 316). The requirement a plausible implementation gets wrong:
**offering the row without the marker** — `pick` names it, the cycle then re-dispatches it, and the same spec is paid
for twice. Hence the three parts, and hence the `verify-failed` control that must stay unmarked.

Names that must exist and pass:
- Part A (the real `pick`): `a-gate-failed-row-is-offered` (id in field 1, session in field 9, rc 0),
  `a-verify-failed-row-is-offered`, `CONTROL-no-session-is-not-resumable`, `CONTROL-a-parked-row-stays-parked`,
  `CONTROL-a-dirty-tree-waits-for-a-person`, `CONTROL-an-unlisted-changes-row-waits-for-a-person`,
  `CONTROL-a-worker-rc-row-is-still-offered`, `CONTROL-a-window-closed-row-is-still-offered`,
  `CONTROL-a-precommit-broken-row-is-still-offered`, `the-row-is-not-blocked-behind-a-pending-one`.
- Part B (the live `tick.sh` block): `the-live-block-has-both-arms` (the block still carries `gate-failed`,
  `verify-failed`, `dirty-tree`, `bad-plan`, `precommit-`, `worker-rc-`), `a-gate-failure-that-will-be-re-gated-says-so`
  (key `regate:A1:gate-failed`, line mentions re-gating), `a-failed-criterion-that-will-be-retried-says-so`
  (key `retry:A1:verify-failed`), `CONTROL-a-gate-failure-nobody-can-resume-asks-for-a-hand`
  (key `bad:A1:gate-failed`), `CONTROL-the-retry-keys-differ-from-the-park-keys`, and
  `CONTROL-a-dirty-tree-still-asks-for-a-hand` (key `bad:A1:dirty-tree`, unchanged).
- Part C (`cycle.sh` literals against the live glob): `the-gate-branch-writes-a-marker-the-glob-matches` (the
  `gate-failed` literal, variable part cut, matches `SKIP_DISPATCH`), `CONTROL-the-verify-literal-does-not-match`
  (bare `verify-failed` must NOT match), `CONTROL-the-house-precedent-still-matches`
  (`worker-ok; gate-timeout after 3600s — the gate was KILLED`), and
  `CONTROL-the-unmarked-literal-the-spec-removes-does-not-match` (bare `gate-failed` must not match, so the change is
  not vacuous).
The script ends `"$OK ok, $FAILS FAIL"` and exits non-zero when `$FAILS` is not 0, exactly as 316's does.

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/gate-refusal.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `! bash scripts/autonomy/tests/gate-refusal.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `bash scripts/autonomy/tests/precommit-refusal.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `bash -n scripts/autonomy/cycle.sh && bash -n scripts/autonomy/tick.sh`
- [ ] `uv run --no-project python -m py_compile scripts/autonomy/state.py`
- [ ] `grep -qE "last=worker-ok[[:punct:]][[:space:]]gate-failed" scripts/autonomy/cycle.sh`
- [ ] `grep -q "regate:" scripts/autonomy/tick.sh && grep -q "retry:" scripts/autonomy/tick.sh`
- [ ] `grep -q "spec 318" scripts/autonomy/state.py`
- [ ] `git diff --quiet HEAD -- quality-gate.sh gate-steps.txt gate-steps-quick.txt scripts/autonomy/pre-commit-builds.sh`

## Files
- `scripts/autonomy/state.py`
- `scripts/autonomy/cycle.sh`
- `scripts/autonomy/tick.sh`
- `scripts/autonomy/tests/gate-refusal.test.sh`

## Guardrails
- These are the loop's own mechanism files. **Cycle.sh and tick.sh must be written complete and moved into place
  with `mv`**, never edited in place, because a live cycle may be executing them (DO-NOT-REPEAT 2.15).
- Do NOT change the four prefixes `resumable()` already offers, the timeout branch's marker, the `dirty-tree` /
  `bad-plan` arm, or any existing assertion in `precommit-refusal.test.sh` (criterion 3 is the neighbour control: it
  must stay green untouched).
- Do NOT mark `verify-failed` (R3). If the mechanism seems to need it, stop and report: the shape is wrong.
- Do NOT touch `plan.tsv`, the gating scripts or the specs; do not weaken any criterion.
- Mutations the tests must kill, each by the named assertion above: `resumable()` offering `gate-failed` **without**
  the session (kills `CONTROL-no-session-is-not-resumable`); `resumable()` offering every `needs-review` row
  (`CONTROL-a-dirty-tree-waits-for-a-person`); dropping the `worker-ok; ` prefix the gate branch writes
  (`the-gate-branch-writes-a-marker-the-glob-matches`); marking the verify branch (`CONTROL-the-verify-literal-does-
  not-match`); keeping the combined tick arm (`a-gate-failure-that-will-be-re-gated-says-so`); keying the retry as
  `bad:` (`CONTROL-the-retry-keys-differ-from-the-park-keys`); plumbing `NEXT` but ignoring `ITEM`
  (`CONTROL-a-gate-failure-nobody-can-resume-asks-for-a-hand`).

## Closure
<!-- filled by the orchestrator when the row closes: the criteria's output, the gate, the mutation and its restore -->

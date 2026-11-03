# Spec: the resume marker the cycle writes is one the queue accepts

**Project**: /projects/pi-improved · **Phase**: production hardening · **Source**: measured in the autonomy lane,
2026-11-03 — the third member of the family spec 312 (`worker-rc=`), 316 (`precommit-broken`) and 318
(`gate-failed` / `verify-failed`) closed.

## Overview

`cycle.sh` refuses to sweep a dirty tree it does not own — unless the tree carries a worker session, in which case it
resumes that session instead of buying the spec twice. It records what it did at `cycle.sh:163`:

```
state.py set "$ITEM" status=needs-review "session=$RECOVER" "last=resumed-from-worker-log"
```

`state.py`'s `resumable()` accepts a fixed list of markers: `window-closed`, `worker-rc=`, `precommit-broken`,
`precommit-timeout`, `gate-failed`, `verify-failed`, `worker-ok; gate-failed`, `park-cause-repaired`.
`resumed-from-worker-log` is **not** on it, so a row the cycle itself just resumed becomes undispatchable the moment
that cycle ends — and its uncommitted work is exactly the dirty tree that blocks every later row:

```
[20:22:02Z] refused 356: EXTRA crates/pii-server/src/engine/queue_methods.rs, ...
pick: skip parked 353(needs-review), 354(needs-review), 357(parked), 358(needs-review), 359(needs-review) -> dispatching 360
[20:22:07Z] tree: 8 changed paths before dispatch
[20:22:07Z] DIRTY TREE that no queue row owns (FOREIGN 8): saved under .../leftovers, nothing cleaned, nothing dispatched
OUTCOME=dirty-tree ITEM=360
```

Every five-minute firing repeated that pair: the queue looked busy and bought nothing, while the eight paths of row
359's own work sat unowned in the tree. The window before it was the same shape — the loop had been dispatching
nothing since 13:54Z. The defect is not that the marker is unknown to `pick` in the abstract; it is that the writer
(`cycle.sh`) and the reader (`resumable()`) hold two lists of one vocabulary and nothing compares them, so a marker
written by the loop can be invisible to the loop. Spec 318 fixed the same class one marker later and left this hole.

## Requirements

### Functional
- **R1 (pick offers the row the cycle resumed).** `state.py`'s `resumable(status, row)` offers a row again when
  `status == "needs-review"`, the row holds a non-empty `session`, and its `last` **is** `resumed-from-worker-log` —
  alongside the markers it already offers, whose behaviour does not change.
- **R2 (a session is what makes it resumable).** The same marker with no `session` is **not** offered; nor is a
  `needs-review` row whose `last` is `dirty-tree`, `bad-plan`, `unlisted-changes`, `3-strikes`, or anything else.
  Those keep waiting for a person.
- **R3 (one vocabulary, read from the writer).** The accepted markers stay in the single place they are today
  (`resumable()`); `cycle.sh` keeps writing the literal it writes and gains no second list. The acceptance test reads
  the literal **out of `cycle.sh` itself** and feeds it to `pick`, so a future marker pick does not know can never
  ship green.

### Non-functional
- No change to dispatch order, to the three-attempt cap, to `attempts`, or to any other marker.
- `state.py` stays a single-file, stdlib-only script of the size it is today.

## TDD Contract

Acceptance test: `scripts/autonomy/tests/resume-marker.test.sh`, in the shape of
`scripts/autonomy/tests/gate-refusal.test.sh` (spec 318). It writes its own scratch state, plan and specs under
`_tmp/` and points `PII_STATE` / `PII_PLAN` / `PII_SPECS` at them — data and setup only.

The requirement a plausible implementation gets wrong: **offering every needs-review row**. A row parked by a person
would then be re-dispatched behind the operator's back, which is the failure spec 169 exists to prevent. Hence the
controls, and hence R3: the test must take the marker from the live `cycle.sh`, not hand-write it, or it certifies a
constant while the writer says something else.

Names that must exist and pass:
- Part A (the real `pick`): `a-resumed-from-worker-log-row-is-offered` (id in field 1, the session in field 9, rc 0),
  `CONTROL-no-session-is-not-resumable`, `CONTROL-a-dirty-tree-marker-still-waits`,
  `CONTROL-a-hand-parked-row-stays-parked`, `CONTROL-a-worker-rc-row-is-still-offered`.
- Part B (the writer, from the live file): `the-literal-cycle-writes-is-one-pick-accepts` (the marker is extracted
  from `scripts/autonomy/cycle.sh` by grep, fed to `pick` on a scratch row, and must be offered),
  `CONTROL-the-extracted-literal-is-not-empty` (so the extraction cannot pass vacuously),
  `CONTROL-an-unknown-marker-is-not-offered` (a marker planted next to the real one is refused — the test's own
  near-miss).
The script ends `"$OK ok, $FAILS FAIL"` and exits non-zero when `$FAILS` is not 0, exactly as 318's does.

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/resume-marker.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `! bash scripts/autonomy/tests/resume-marker.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `bash scripts/autonomy/tests/gate-refusal.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `uv run --no-project python -m py_compile scripts/autonomy/state.py`
- [ ] `bash -n scripts/autonomy/cycle.sh`
- [ ] `grep -q "spec 366" scripts/autonomy/state.py`
- [ ] `bash scripts/autonomy/selftest.sh 2>&1 | grep -q "VERDICT: ALL MECHANISMS PASS"`
- [ ] `git diff --quiet HEAD -- quality-gate.sh gate-steps.txt gate-steps-quick.txt`

## Files
- `scripts/autonomy/state.py`
- `scripts/autonomy/tests/resume-marker.test.sh`

## Guardrails
- These are the loop's own mechanism files. `state.py` must be written complete and moved into place with `mv`, never
  edited in place, because a live cycle may be executing it (DO-NOT-REPEAT 2.15).
- Do NOT change the markers `resumable()` already offers, the three-attempt cap, or any existing assertion in
  `gate-refusal.test.sh` / `precommit-refusal.test.sh` / `precommit-refusal.test.sh`'s neighbours: criterion 3 is the
  neighbour control and must stay green untouched.
- Do NOT touch `plan.tsv`, the gating scripts (`quality-gate.sh`, `gate-steps*.txt`) or any other spec; do not weaken
  any criterion to make this pass.
- Mutations the test must kill, each by the named assertion: dropping the new marker (`a-resumed-from-worker-log-row-
  is-offered`); offering every `needs-review` row (`CONTROL-a-hand-parked-row-stays-parked`); offering the marker
  without a session (`CONTROL-no-session-is-not-resumable`); replacing the extraction with a hand-written literal
  (`the-literal-cycle-writes-is-one-pick-accepts` must read the file); an extraction that returns nothing
  (`CONTROL-the-extracted-literal-is-not-empty`); widening the list to accept anything
  (`CONTROL-an-unknown-marker-is-not-offered`).
- If the mechanism seems to need more than this list, stop and report the shape rather than forcing the change.

## Closure
<!-- filled by the orchestrator when the row closes: the criteria's output, the gate, the mutation and its restore -->

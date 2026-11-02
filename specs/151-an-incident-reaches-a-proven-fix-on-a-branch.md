# Spec: an incident reaches a proven fix on a branch, never master

## Overview

Spec 150 gives the loop eyes: an incident opened from its own records, with no model. Spec 153 makes the
operator's records fit to measure. Nothing yet turns an incident into a **fix**. The measured distance
(061 Q4, baseline: 44 commits since 2026-10-11T15:00, 34 by hand, 17 of those repairs to the loop itself
≈ 3.4 hand-made commits per spec closed) is spent exactly there: a person reads the incident, finds the
cause, fixes it, and proves it.

061 Q3 already answers for this class: drafting a brief into `_tmp/autonomy/briefs/`, opening a branch with
a fix and a proving test, and merging a proven, gated branch into `master` are all `yes` actions **without a
person, while `master` ≠ production** — and the proof is run by the loop, never reported by the worker.

This spec builds that path. The one thing it does NOT do is deploy: the card (spec 152) is the permission,
and `install-pii.sh` stays behind it.

**Project**: /projects/pi-improved
**Phase**: proactivity (the Opus 061 judgement; order 153 → 150 → 154 → **151** → 152)

## Requirements

### Functional

R1. A **second plan source** beside `scripts/autonomy/plan.tsv`: an open incident in
    `_tmp/autonomy/incidents.jsonl` (spec 150) becomes a brief at `_tmp/autonomy/briefs/<fingerprint>.md`,
    built from the incident's own evidence — fingerprint, source, `first_seen`, `count`, and the evidence
    `path:line`. The brief is **never** a spec: nothing on this path writes under `specs/` (planner-only).
R2. The work happens on branch `incident/<fingerprint>` in its own git worktree, created from the recorded
    base revision. `master` is never checked out by this path and never committed to by it.
R3. The loop runs the **proving test itself, both ways**: the test named by the brief must be **RED on the
    base revision** and **GREEN on the branch**, and `fresh-gate` must be green on the branch. Any missing
    leg — not red on base, not green on branch, gate not green — **parks** the incident with the evidence,
    keeps no merge and offers no card.
R4. **Budget**: at most one dispatch per fingerprint per local day, plus a recorded cost ceiling per
    incident. A second dispatch inside the window is refused, with the refusal and its reason recorded — it
    is not silently throttled and never "retried smaller".

### Non-Functional

R5. Every leg records what it measured: base and branch exit codes, the gate's verdict, the compared
    `master` tip before and after. A leg that cannot be measured parks (R3), it does not pass by default.

## Architecture

Three files, one entry point, no new daemon surface:

- `scripts/autonomy/incident-fix.sh <fingerprint>` — the entry point the loop calls. It reads the incident
  from `_tmp/autonomy/incidents.jsonl`, writes `_tmp/autonomy/briefs/<fingerprint>.md`, creates the worktree
  and branch `incident/<fingerprint>` from the incident's base revision, dispatches the worker
  (`PII_INCIDENT_WORKER`, default the Claude batch runner), runs the proving test on base and on the branch,
  runs the gate on the branch, and either fast-forwards `master` from the proven branch or parks with the
  evidence. It records its budget use under `_tmp/autonomy/incidents/<fingerprint>/`.
- `scripts/autonomy/cycle.sh` — gains the incident list as a second plan source **after**
  `scripts/autonomy/plan.tsv`, and calls the entry point once per open incident.
- `scripts/checks/incident-to-branch.sh` — the harness that drives the entry point on a scratch clone and
  prints the legs it measured; `scripts/checks/incident-budget.sh` — the day/fingerprint budget.

Nothing on this path writes under `specs/`, and nothing touches `master` except the fast-forward of a
branch that passed both proof legs and the gate.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| an_incident_becomes_a_brief | an `incidents.jsonl` holding one `dirty-tree` incident | `_tmp/autonomy/briefs/<fp>.md` names the fingerprint, count, `first_seen` and the evidence `path:line`; `specs/` is byte-identical afterwards |
| a_fix_lands_on_a_branch_and_master_does_not_move | an incident with a scripted fix | branch `incident/<fp>` exists; `master`'s tip sha before == after |
| the_proof_is_red_on_base_and_green_on_branch | the same incident | the brief's named test exits non-zero on base and zero on the branch, both recorded |
| a_test_already_green_on_base_parks | the near-miss control: an incident whose named test passes on base | parked with the evidence; no branch kept, no merge |
| a_gate_that_is_not_green_parks | a gate invocation that fails | parked naming the gate; no merge |
| a_second_dispatch_in_the_day_is_refused | the same fingerprint dispatched twice | the second refused, the reason recorded |

## Exit Criteria

- [ ] `bash scripts/checks/incident-to-branch.sh 2>&1 | tail -1 | grep -q 'INCIDENT-BRANCH OK'` — the
      harness drives the loop's own entry point (`scripts/autonomy/incident-fix.sh`) on a scratch clone of
      this repo seeded at the parent of `9fc7f9f`, with the worker injectable (`PII_INCIDENT_WORKER`), so
      the loop's legs are exercised rather than re-enacted
- [ ] `bash scripts/checks/incident-to-branch.sh 2>&1 | grep -q 'legs: base_rc=[^0] branch_rc=0 gate=PASS'` —
      the two proof legs come from that run, not from a constant: one line the harness prints from the
      invocations it made, plus the `master` tips it compared
- [ ] `bash scripts/checks/incident-to-branch.sh --control master-moved 2>&1 | tail -1 | grep -q 'CONTROL OK'`
      — told to commit on `master`, the harness must refuse and say so: a check that cannot fail is worse
      than no check
- [ ] `bash scripts/checks/incident-budget.sh 2>&1 | tail -1 | grep -q 'BUDGET OK'` — inside it, the
      near-miss control: a *different* fingerprint on the same day is still dispatched
- [ ] `cargo xtask fresh-gate` on this spec's commit is `ALL PASS`

## Guardrails

- The brief is never a spec and never a merge: `specs/` stays the planner's, `master` stays untouched until
  both proof legs and the gate are green, and it is only ever fast-forwarded by a **proven** branch (Q3:
  permitted while `master` ≠ production — it is, the deploy is a separate permission).
- Do NOT weaken a leg to close an incident: a test that is not red on base is a **finding** (the incident's
  premise is wrong), and it parks with the evidence saying so. Do not delete or rename the proving test, and
  do not edit `specs/`.
- A worker that stops to flag a wrong premise is correct behaviour: resume its session with the corrected
  brief; never start a fresh session to "clean up".
- The scripted worker used by the harness is a check fixture, not the loop's worker: the harness must call
  the same entry point the loop calls, with only the worker command injected.
- One dispatch per fingerprint per day is a throttle, not a block: the refusal is recorded, and a new
  fingerprint (or a new local day) proceeds.

## Closure

Filled 2026-11-02. Every line below is output of a run made that day on this tree, not a claim about the design.

**The harness, on a scratch clone of this repo, driving the loop's own entry point:**

```
$ bash scripts/checks/incident-to-branch.sh
ok: an_incident_becomes_a_brief
ok: a_fix_lands_on_a_branch_and_master_does_not_move (master a86e85cae115 == a86e85cae115)
legs: base_rc=1 branch_rc=0 gate=PASS master_before=a86e85cae115 master_after=a86e85cae115
ok: the_proof_is_red_on_base_and_green_on_branch
ok: a_test_already_green_on_base_parks
ok: a_gate_that_is_not_green_parks
ok: a_proven_branch_fast_forwards_master
INCIDENT-BRANCH OK
```

Both proof legs are real: the proving test exits **1 on the base revision and 0 on the branch**, the gate says
PASS on the branch, and `master`'s tip is the same sha before and after. The two near-misses park: a test that is
already green on base, and a gate that is not green.

**The budget, with its near-miss control:**

```
$ bash scripts/checks/incident-budget.sh
ok: the refusal and its reason are recorded
ok: near-miss: a different fingerprint the same day is still dispatched
ok: a new local day proceeds
ok: the cost ceiling refuses, with its reason
BUDGET OK
```

**The control that proves the master check can fail** — told to commit on `master`, the harness must notice:

```
$ bash scripts/checks/incident-to-branch.sh --control master-moved
master-moved detected: a86e85cae11534e1f3bd836dac53adad763c90b7 -> ed3bb969d413be3822a7af7abbbe37782d32b556 (refused)
legs seen: legs: base_rc=1 branch_rc=0 gate=PASS master_before=a86e85cae115 master_after=ed3bb969d413
CONTROL OK
```

**What is NOT proven here, stated plainly.** The worker the harness injects is scripted (`git diff … | git apply`),
so what is measured is the *loop*: brief, branch, the two proof legs, the gate, the merge decision, the budget.
It is **not** a measurement of a real model reaching a fix, and it is not the pre-registered economics (≥3 of 7
historical failures yielding a mechanical fix under $1 each). The default worker is a real
`claude -p … --model sonnet` dispatch (default ceiling $5); exercising it needs a real incident whose proving test
is red at the base revision, which the incident record does not carry today. M3 is therefore **mechanism proven,
real-worker dispatch unmeasured** — and a later reader must not read this closure as more than that.

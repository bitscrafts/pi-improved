# Spec: the dispatcher may run on a schedule and still never collide

## Overview

The operator, 2026-11-04, twice: *"cron must always be on!"* and *"we have decided that cron should not
run if anything is running! we need some check before the cronjob start or peek anything from the
queue!"*

Measured the same evening: at 14:52Z the every-five-minutes dispatcher picked row **384** and
dispatched it into a tree holding row **380**'s seven uncommitted paths. Row 380's gate then failed on
384's test while reporting on 380, and the earlier firing had quarantined 380's work as "parked"
mid-flight. Both were possible because the only collision check was the **tree hold**, which is a
*declaration*: it works only when somebody remembers to take it, and nobody did.

This row adds the two **measurements** that hold whoever forgot. The cron stays on; it just refuses to
work when the evidence says the tree is busy.

**Project**: /projects/pi-improved
**Phase**: autonomy

## Requirements

- **R1 — a running worker is a refusal.** Before peeking the queue, if any process matches a lane
  worker or a cycle (`run-batchC-pii`, `run-batchC-claude`, `scripts/autonomy/cycle.sh`), the drain
  prints what it found and exits 0 having dispatched nothing.
- **R2 — a dirty tree is a refusal.** Before peeking the queue, if `git status --porcelain` is not
  empty, the drain prints the paths and exits 0. Uncommitted work at the start of a firing belongs to
  someone else, and "the diff the gate measures is the diff this row wrote" requires a clean start.
- **R3 — both are measurements, not declarations.** Neither may depend on the hold, on a file an agent
  must write, or on a variable an operator must export.
- **R4 — the hold keeps its own meaning.** Priority 100 (a person) is never preempted; automation below
  100 yields. `hold.sh` gains `PII_HOLD` so a test can point it elsewhere (already needed by R1's test).
- **R5 — the refusal is visible in one line.** Every refusal names its reason and, for R2, the paths —
  a silent yield is indistinguishable from an empty queue.
- **R6 — controls.** Each rule needs a case that must NOT refuse: a clean tree with no hold must reach
  the cycle; the test states that case and skips it loudly rather than passing it when the tree is dirty.

## TDD contract

| Test | Input | Expected |
|---|---|---|
| `a_running_worker_yields` | a process whose argv matches a lane worker | drain prints `a worker or cycle is already running`, dispatch 0 |
| `CONTROL_did_not_dispatch` | the same run | no `OUTCOME=`, no `drain: dispatch`, no `pick:` |
| `a_dirty_tree_yields` | one planted untracked path | drain prints `the tree has uncommitted work` and the path |
| `CONTROL_clean_tree_is_not_refused` | clean tree, `MAX_CYCLES=0` | neither guard line appears |

## Exit Criteria

- [ ] `bash scripts/autonomy/tests/drain-guards.test.sh`
- [ ] `grep -q 'a worker or cycle is already running' scripts/autonomy/drain.sh`
- [ ] `grep -q 'the tree has uncommitted work' scripts/autonomy/drain.sh`
- [ ] `grep -q 'PII_HOLD' scripts/autonomy/hold.sh`

## Guardrails

- Do NOT implement the check by pausing the cron job. The cron runs always; the check runs per firing.
- Do NOT treat the hold as sufficient. It is the human's declaration, not the machine's measurement.
- Do NOT widen the running-worker pattern to everything with the word "pii" in it: a false positive
  stops the queue silently, which is the failure this row exists to avoid.
- Do NOT clear a dirty tree to get past R2. The tree is somebody's work; quarantine or commit it.
- Do NOT make either check spend anything: they run before any model, any lock, any worker.

## Files

- `scripts/autonomy/drain.sh`
- `scripts/autonomy/hold.sh`
- `scripts/autonomy/tests/drain-guards.test.sh`

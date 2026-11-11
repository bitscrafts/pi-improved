# Spec: the queue runs in the repository it was started from
#: depends=402 status=pending block=-

## Overview
The queue loop is the only thing that lets a spec become a commit without a person, and it works on exactly one repository:
`/projects/pi-improved` is written into 11 lines of `scripts/autonomy/cycle.sh`, `drain.sh`, `hold.sh`, `head-builds.sh`,
`pre-commit-builds.sh`, `row-run-criteria.py` and `env.sh` (measured 2026-11-10, `git grep`). A second repository that wants pii to manage its
specs (laya-rs is the first) cannot have its own queue: a cycle started in it would `cd` into pi-improved, lock pi-improved, read pi-improved's
`plan.tsv` and commit into pi-improved.

Decision (owner, 2026-11-10): one queue per repository, not one shared plan with a `repo=` column. Each repository carries its own
`scripts/autonomy/plan.tsv`, its own `_tmp/autonomy/` state and its own lock, and the queue's scripts find that repository from where they
live. This spec makes the scripts do that. It does not change what a gate is: a repository's own gate command is the next spec (406); until then the
gate stays pi-improved's.

**Project**: <repo>
**Phase**: the queue serves more than one repository
**Depends on**: 402

## Requirements
- R1 One root. `scripts/autonomy/env.sh` exports `PII_ROOT`: the value already in the environment when it is set and non-blank, otherwise the repository
  that contains the script (`readlink -f` of the script, three directories up), never the caller's working directory. A set-but-blank `PII_ROOT` is refused
  loudly (the same rule `state.py` applies to a blank `PII_*`, row X1), not treated as unset.
- R2 No literal root in code. After this change no line of `cycle.sh`, `drain.sh`, `hold.sh`, `head-builds.sh`, `pre-commit-builds.sh`,
  `row-run-criteria.py` or `env.sh` carries `/projects/pi-improved` outside a comment. Each uses `PII_ROOT` (shell) or the same rule in Python
  (`os.environ`, else the file's own location). Existing overrides (`PII_LOCK`, `PII_DRAIN_ROOT`, `PII_REPO`, `HEAD_BUILDS_REPO`, `PRE_COMMIT_REPO`,
  `PII_STATE`, `PII_PLAN`, `PII_SPECS`) keep winning over `PII_ROOT`, so every existing test and caller behaves as before.
- R3 The defaults `env.sh` derives from the root follow it: `FRESH_GATE_TARGET`, `GATE_WRITE_DIRS` and `PII_LANE_FILE` are under `$PII_ROOT/_tmp/`, not under a
  fixed path. A value already in the environment still wins.
- R4 State and lock are the repository's: a cycle started from a copy of the queue scripts inside another git repository reads that repository's
  `plan.tsv`, takes that repository's `_tmp/autonomy/lock`, writes that repository's `_tmp/autonomy/logs/`, and touches nothing under any other repository.
- R5 A cycle started in a directory that is not a git repository, or whose root has no `scripts/autonomy/plan.tsv`, refuses before it takes the lock or probes
  the account: `OUTCOME=plan-error`, exit 4, nothing spent, and the message names the root it looked at.

## Architecture
```
env.sh   PII_ROOT = ${PII_ROOT:-<script>/../../..}   (blank => refuse)
           |-- FRESH_GATE_TARGET, GATE_WRITE_DIRS, PII_LANE_FILE  defaults under $PII_ROOT/_tmp
cycle.sh / drain.sh / hold.sh / head-builds.sh / pre-commit-builds.sh      ROOT=$PII_ROOT
row-run-criteria.py                                                        root = env or <file>/../../..
```
Key decisions:
- The root comes from the script's own location, not `$PWD`: cron and the drain start the loop from arbitrary directories, and "the repository I was
  started in" must mean the repository the queue scripts belong to. Rejected: reading `git rev-parse --show-toplevel` from `$PWD`; a cycle started from
  inside a worktree or a sibling checkout would then commit into the wrong tree.
- One variable, one derivation, in the file every script already sources. Rejected: a per-script `ROOT=` line; that is how 11 copies of one literal happened.
- No `repo=` column and no per-row gate lookup (see Overview). Rejected: a shared plan across repositories; every reader of `plan.tsv` (53 files) changes and
  two repositories' rows then share one state file and one lock.

## TDD Contract
New test file `scripts/autonomy/tests/queue-root.test.sh`; it builds its scratch repository under `_tmp/` (a `git init` with a copy of
`scripts/autonomy/`, a one-row `plan.tsv`, a fake `claude` first on `PATH` that answers the probe) and moves the scratch to `_trash` when done.

| Test Name | Input | Expected Output |
|---|---|---|
| `root_is_the_repository_the_script_lives_in` | `env.sh` sourced from the scratch copy, cwd = `/` | `PII_ROOT` is the scratch repository |
| `CONTROL_a_root_in_the_environment_wins` | `PII_ROOT=/x` set, then sourced | `PII_ROOT` is `/x` |
| `a_blank_root_is_refused` | `PII_ROOT=` (set, empty) | non-zero exit; the message names `PII_ROOT` |
| `a_dry_cycle_in_the_scratch_repo_reads_its_plan_and_its_lock` | `DRY=1 cycle.sh` from the scratch copy, cwd = the real repository | output names the scratch row's spec; the scratch `_tmp/autonomy/lock` exists |
| `a_cycle_in_the_scratch_repo_touches_nothing_in_the_real_one` | the same run | no file under the real `_tmp/autonomy/` is newer than the run's start |
| `CONTROL_the_real_repo_still_runs_in_itself` | `DRY=1 cycle.sh` from the real repository | output names the real repository's own pick (or `NONE`), the real lock is touched |
| `a_repo_without_a_plan_is_refused_before_the_lock` | scratch repo with `plan.tsv` removed | `OUTCOME=plan-error`, exit 4, no lock file created, the fake `claude` was not called |
| `no_script_carries_the_literal_root` | the 7 files | no non-comment line contains `/projects/pi-improved` |

Acceptance test: `a_cycle_in_the_scratch_repo_touches_nothing_in_the_real_one`. The dangerous requirement is R4: a plausible implementation changes
`ROOT=` in `cycle.sh` and leaves `drain.sh`/`hold.sh` on the literal, so the dry cycle looks right and the first real dispatch takes pi-improved's hold. The
test therefore compares the real repository's `_tmp/autonomy/` before and after, not only the scratch repository's output.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `bash scripts/autonomy/tests/queue-root.test.sh 2>&1 | grep -qE '^PASS: [1-9][0-9]* FAIL: 0$'`
- [ ] `! grep -n -E '^[^#]*/projects/pi-improved' scripts/autonomy/cycle.sh scripts/autonomy/drain.sh scripts/autonomy/hold.sh scripts/autonomy/head-builds.sh scripts/autonomy/pre-commit-builds.sh scripts/autonomy/row-run-criteria.py scripts/autonomy/env.sh`

## Guardrails
- Keep green (they pass today, so they are not criteria): `bash scripts/autonomy/tests/run-tests.sh`, `bash scripts/autonomy/selftest.sh`.
- Do NOT add a column to `plan.tsv` or a `repo=` field; do NOT change `state.py`'s row format.
- Do NOT change what the gate runs or how a gate is chosen (`gate-env.sh`, `gate-steps.txt`): that is spec 406.
- Do NOT start a cycle, a drain or a real dispatch from a test: every scratch run uses `DRY=1` or a refusal that happens before the probe.
- Do NOT write under `scripts/autonomy/` of a real repository from a test; scratch lives under `_tmp/`.
- A file over 450 lines does not get longer (`cycle.sh` is baselined): each swap is in place, one literal for one variable.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `scripts/autonomy/env.sh`
- `scripts/autonomy/cycle.sh`
- `scripts/autonomy/drain.sh`
- `scripts/autonomy/hold.sh`
- `scripts/autonomy/head-builds.sh`
- `scripts/autonomy/pre-commit-builds.sh`
- `scripts/autonomy/row-run-criteria.py`
- `scripts/autonomy/tests/queue-root.test.sh`

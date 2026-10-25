# Spec 268: the cycle refuses to commit while the worker's changes include a file its row did not list

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the queue's own commit step (found 2026-10-25: `04146bb` did not build)

The cycle commits "explicit paths only" (`scripts/autonomy/cycle.sh:375-394`): `git add -- <the row's paths>`, then commit. A worker that also edits a file the row did not name leaves that edit in the working tree, uncommitted. The gate runs on the WORKING TREE, so it passes; the commit carries only the listed paths, so HEAD does not build. Measured twice on 2026-10-25 in one hour: A131 (`04146bb`, `crates/pii-graph/src/nodes.rs` had `eval_ctx` made `pub(crate)` but was not in the row) and A132 (`15ee36c`, the same missing file at that moment). `head-builds` caught it after the commit and parked both rows `needs-review`; the repair was a hand commit `c29046f`. The earlier cases were A118 and A120 (rows with no paths).

The failure is cheap to prevent BEFORE the commit: after `git add`, any tracked change or non-ignored new file still unstaged, other than the queue's own files, means the row's path list is wrong. The work is not lost (it stays in the tree); only the commit must not happen on a list that is known incomplete.

## Verified Premises
- **CONFIRMED** (read) `cycle.sh:375-394`: the commit step runs `git add -- "${P[@]}"` then `git diff --cached --quiet` (nothing staged = "already satisfied") then `git commit`. There is no check of what is left unstaged.
- **CONFIRMED** (read) `cycle.sh:143` counts dirty paths BEFORE dispatch only; `cycle.sh:147-160` handles pre-existing leftovers by committing them as a named leftover.
- **CONFIRMED** (run, 2026-10-25) `git show --stat 04146bb` lists 6 files and no `nodes.rs`; `git status` after it showed ` M crates/pii-graph/src/nodes.rs`; `head-builds` said `HEAD 04146bb does NOT build` with `error[E0603]: function eval_ctx is private`.
- **CONFIRMED** (run) the queue's own files that are normally dirty after a worker: `HANDOFF.md`, `QUEUE-BRIEF*`, `scripts/autonomy/__pycache__/*`; `scripts/autonomy/plan.tsv` and `specs/*` are committed by hand beforehand.
- **NOT CLAIMED** that no row legitimately leaves a file unstaged on purpose; if one does, the refusal names it and a person lists it.

## Requirements
### Functional
- **R1. A helper decides.** New `scripts/autonomy/unlisted-changes.sh <paths>` (paths `;`-separated, the row's own format): prints one path per line for every path `git status --porcelain` shows as modified, deleted or untracked-and-not-ignored that is NOT listed (a listed entry that is a directory covers everything beneath it) and is NOT one of the ignored queue files: `HANDOFF.md`, anything under `scripts/autonomy/__pycache__/`, anything matching `QUEUE-BRIEF*`. Prints nothing and exits 0 when there are none; exits 1 when it printed any.
- **R2. The commit step uses it.** In `cycle.sh`, immediately after the `git add` and before `git diff --cached --quiet`, run the helper on `$PATHS`. If it exits 1: do NOT commit; unstage with `git reset -q`, set the item `status=needs-review last=unlisted-changes: <first 5 paths, space-separated>`, print `OUTCOME=unlisted-changes ITEM=$ITEM` and exit 17. The tree is left exactly as the worker made it.
- **R3. A test with controls.** New `scripts/autonomy/tests/unlisted-changes.test.sh` (new file; edit no existing test). In a temp git repo it asserts as `ok`/`FAIL` lines: (a) an unlisted modified tracked file is printed and exit is 1 (the A131 case); (b) the same file listed is not printed, exit 0; (c) CONTROL: a listed directory covers a file beneath it; (d) CONTROL: `HANDOFF.md`, a `__pycache__` file and a `QUEUE-BRIEF-x.md` modified or created are NOT reported; (e) an unlisted untracked source file is printed; (f) an ignored file (in `.gitignore`) is NOT printed; (g) an unlisted deleted file is printed; (h) a clean tree prints nothing and exits 0; (i) `cycle.sh` calls the helper between `git add` and `git diff --cached --quiet` (a grep on line order), and contains the text `unlisted-changes`. It prints `N ok, 0 FAIL`.

### Non-Functional
- Bash only (`set -u`); `bash -n` passes. No Python added. The test never runs `cycle.sh`, the queue or a gate.
- Do not edit `gate-steps.txt`, `quality-gate.sh`, `env.sh`, `gate-env.sh`, or any existing test.

## TDD Contract
| Assertion | Expected |
|---|---|
| `an unlisted modified file is reported` | printed, rc 1 |
| `a listed file is not` | silent, rc 0 |
| `CONTROL a listed directory covers its files` | silent |
| `CONTROL queue-owned files are ignored` | silent |
| `an unlisted untracked file is reported` | printed |
| `an ignored file is not` | silent |
| `an unlisted deletion is reported` | printed |
| `a clean tree is silent` | rc 0 |
| `cycle.sh checks before the diff` | line order as stated |

**The requirement a plausible implementation satisfies only in appearance:** running the check AFTER `git commit` (or after `head-builds`). It then reports a break that is already in HEAD. Assertion (i) fixes the order.

## Exit Criteria
- [ ] `test -f scripts/autonomy/unlisted-changes.sh`
- [ ] `test -f scripts/autonomy/tests/unlisted-changes.test.sh`
- [ ] `bash scripts/autonomy/tests/unlisted-changes.test.sh | tail -1 | grep -q " 0 FAIL"`
- [ ] `bash scripts/autonomy/tests/unlisted-changes.test.sh > /dev/null`
- [ ] `bash scripts/autonomy/tests/gate-steps.test.sh | tail -1 | grep -q " 0 FAIL"`
- [ ] `bash -n scripts/autonomy/cycle.sh && bash -n scripts/autonomy/unlisted-changes.sh`
- [ ] `grep -q "OUTCOME=unlisted-changes" scripts/autonomy/cycle.sh`
- [ ] `git diff --quiet -- gate-steps.txt gate-steps-quick.txt quality-gate.sh scripts/autonomy/env.sh scripts/autonomy/gate-env.sh scripts/autonomy/tests/gate-steps.test.sh scripts/autonomy/tests/gate-coverage.test.sh`

## Guardrails
- Do NOT edit the files named in the last criterion. Do NOT run the queue, `cycle.sh` or a gate from the test. Do NOT commit.

## Follow-ups (not this spec)
Have the helper also run for `done-satisfied`; derive a row's paths from the spec's own file list instead of by hand.

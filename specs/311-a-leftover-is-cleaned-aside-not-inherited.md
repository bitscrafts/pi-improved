# Spec 311: a leftover is cleaned aside, not inherited by the next item's gate

## Overview
**Project**: <repo>
**Phase**: production — the queue's own reliability, the seam spec 309 and spec 283 both sit on.

**Measured 2026-10-28/29, on this tree.** A173 (spec 309) failed its gate at 21:49:12Z. The next cycle, at 21:49:20Z,
found the worker's `scripts/autonomy/state.py` still modified, copied its diff to
`_tmp/autonomy/leftovers/20261028T214917Z-tracked.patch` — and then dispatched A174 into the SAME dirty tree, because
the branch says so in its own words: `cycle.sh:163`, *"the tree is left as it was"*. A174's gate failed at 22:03:19Z
on the same pre-existing test, `crates/pii-graph/tests/pick_skips_parked.rs::a_parked_head_does_not_freeze_the_queue`,
which A173's change had reddened. A174's own nine criteria were green the whole time: **two gate runs and 11 minutes
of gate time were spent measuring a change that belonged to no item under test, and the loop then parked both rows
"waiting for a person".**

The saving half has its own two holes, both read in the code:
- `cycle.sh:159` saves `git diff`, which by definition holds no untracked file. A worker's new test file — the shape
  most of this queue's items produce (`crates/*/tests/*.rs`) — is not in the patch at all.
- `cycle.sh:160` lists `git status --porcelain | awk '{print $2}'`. For a rename, porcelain's first line reads
  `R  old -> new`, so `$2` is the OLD path: the list names the file that no longer exists and omits the one that does.

Nothing here is a demand to stop the loop: this repository already paid for that (the 2026-10-13 note at
`cycle.sh:146` — stopping on every leftover idled the loop for hours). What is wrong is *which* leftovers are carried
forward, and that they are carried into the next worker's gate.

## Requirements
### Functional
- **R1. One reader for the leftover, `scripts/autonomy/leftover.sh`** (bash, no new dependency), four verbs:
  - `save <dir> <cid>` → writes three files into `<dir>`: `<cid>-tracked.patch` (`git diff`), `<cid>-untracked.tar`
    (only when there are untracked paths; content, not just names) and `<cid>-paths.txt` (from
    `git status --porcelain -z`, so a rename contributes BOTH of its paths, one per line, NUL-split). Prints
    `SAVED <dir>/<cid>-*` and exits 0. Read-only with respect to the worktree: it creates files in `<dir>` only.
  - `owner` → attributes the working tree's changes to a queue row or to nobody. Reads `state.py`'s rows and state
    (`PII_PLAN`, `PII_STATE`, `PII_SPECS` honoured, as `row-paths.py` does) and the dirty paths from
    `git status --porcelain -z`. It answers on stdout, one line:
    - `OWNER <row-id>` and exit 0 when EXACTLY ONE row is (a) not `done`/`skipped`/`revoked`, (b) holds a `session`,
      and (c) names at least one of the dirty paths in its column 6 — i.e. the loop's own unfinished work;
    - `FOREIGN <n>` and exit 7 otherwise, `n` being the number of dirty paths (0 = clean). Two candidate rows, a row
      without a session, or no row naming the path all count as foreign: an unattributable tree is a person's, and
      the cycle must not start a worker beside it.
  - `clean <cid>` → `git stash push --include-untracked -m "pii-leftover-<cid>"`, then asserts
    `git status --porcelain` is empty; prints `STASH <ref>  (restore: git stash pop <ref>)` and exits 0 only when the
    tree is clean. A stash that fails, or a tree still dirty afterwards, exits 8 and prints `NOT-CLEANED` with the
    remaining paths. Never `git checkout --`, never `reset --hard`, never `clean -fd`: the content must be
    recoverable, and a stash commit is.
  - `list` → one line per entry in the leftover dir, newest last (`<cid> <bytes> <paths-n>`), read-only.
- **R2. The cycle asks `owner` before it buys a worker.** In `scripts/autonomy/cycle.sh`, the dirty-tree branch (the
  `else` at line 157, the no-session case) becomes: `leftover.sh save`, then `leftover.sh owner`:
  - `OWNER <id>` → `leftover.sh clean`, `say` the row it attributed and the stash ref, then continue to dispatch.
    The next worker's gate then measures only its own change. If `clean` fails → **exit 7** naming the paths; no
    worker is bought into a tree whose state is unknown.
  - `FOREIGN <n>` → **exit 7** (`OUTCOME=dirty-tree`), with the three saved files named, and dispatch nothing. This is
    the behaviour this skill's own table already requires for rc 7 ("a tree with no session to resume — STOP and
    report; never sweep someone's WIP"); the current code contradicts it and that contradiction is what cost A174.
  - The order is fixed: save, attribute, then clean. A clean without a save is the only unrecoverable path here.
- **R3. The attribution is by the row's own declared paths, not by a guess.** Column 6 is already the row's commit
  set and is checked against the spec's `## Files` (spec 283), so it is the one honest statement of what a row owns.
  A dirty path outside every row's column 6 is foreign by construction.
- **R4. Nothing else changes.** No dispatch, brief, verify, gate, commit or state behaviour is touched; the park/
  resume/needs-review paths keep their meaning. `specs/` and `plan.tsv` are untouched by the implementation.

### Non-Functional
- bash only, no new dependency; `leftover.sh` under 200 lines; the test script under 450 lines.
- Every test row builds its OWN git fixture under `_tmp/` (created by the test, never `/tmp`), and the live worktree,
  the live `plan.tsv` and the live `state.json` are never read or written.
- `bash scripts/autonomy/selftest.sh` stays green and gains no row from this change.
- **The selftest is the ORCHESTRATOR's check, not an exit criterion (amended 2026-10-29, measured).** It was written as
  criterion 6 and the first worker pass reported 6 of 7 with that one red. It is not the implementation: the same
  selftest read `42 passed, 0 failed / VERDICT: ALL MECHANISMS PASS` on that worker's own tree
  (`M scripts/autonomy/cycle.sh`, `?? leftover.sh`, `?? leftover-test.sh`, HEAD `4c0a877`) when the orchestrator ran
  it at 00:57Z, while the lane's verify sandbox failed it — the selftest drives `tick.sh`, `uv`, a fake `hermes` and
  the repo's `_tmp`, none of which the judge sandbox is obliged to have. A criterion that cannot run where the item is
  verified is a defect of the spec, not of the work (the same class as a `--lib` criterion that collects no tests).
  The review duty is therefore explicit: **the orchestrator runs `selftest.sh` on the worker's tree and again after
  the commit before this spec is closed**, and records both readings in `## Closure`.

## Verified Premises
- **CONFIRMED (read)** `cycle.sh:143-167` is the dirty-tree branch: `DIRTY=$(git status --porcelain | wc -l)`, the
  session-recovery attempt at :152, and the leftover `else` at :157-166 which saves the patch and the path list.
  Line 163 is the sentence that authorises inheriting the tree: *"Nothing is staged or committed by the loop; the
  tree is left as it was."*
- **CONFIRMED (read)** `git diff` excludes untracked files and `git status --porcelain` prints a rename as
  `R  old -> new`, so `awk '{print $2}'` yields the old path. Both are why `save` in R1 must use `-z` and a tar.
- **MEASURED 2026-10-29** the two-gate cost: A174's gate log (`_tmp/autonomy/logs/cycle-20261028T214917Z.log`) fails on
  the spec 169 fixture while A174's own nine criteria are green, and the leftover patch it inherited is
  `_tmp/autonomy/leftovers/20261028T214917Z-tracked.patch` (1006 bytes, `scripts/autonomy/state.py`).
- **NOT CLAIMED**: that every dirty tree is a leftover. A tree whose paths no row claims is exactly the case R2 sends
  to rc 7, which is why `owner` exists rather than a blanket `clean`.

## TDD Contract
| Test (`scripts/autonomy/tests/leftover-test.sh`) | Input | Expected |
|---|---|---|
| `a-tracked-change-is-saved-and-cleaned` | fixture repo, one modified tracked file | `save` writes the patch; `clean` leaves `git status --porcelain` empty; rc 0 |
| `CONTROL-an-untracked-file-keeps-its-content` | fixture repo, one new untracked file | after `save`+`clean`+`git stash pop`, the file is back byte-identical (`sha256sum` equal) |
| `a-rename-is-listed-by-both-paths` | `git mv a.txt b.txt` staged | `-paths.txt` holds `a.txt` AND `b.txt`, so the listed path is not one that no longer exists |
| `CONTROL-a-clean-tree-is-a-no-op` | fixture repo, nothing changed | `owner` prints `FOREIGN 0`, `save` writes no patch, `clean` exits 0 without creating a stash |
| `the-loops-own-work-is-attributed` | fixture plan row R1 with a session, status `needs-review`, column 6 naming the dirty path | `owner` prints `OWNER R1`, rc 0 |
| `a-row-without-a-session-is-foreign` | same, but R1 holds no session | `FOREIGN 1`, rc 7 — the near miss: attribution needs the session, not just the path |
| `two-claimed-rows-are-foreign` | two rows, both with sessions, both naming the same dirty path | `FOREIGN 1`, rc 7 — ambiguity is not a guess |
| `a-clean-step-that-cannot-clean-refuses` | a fixture repo where `git stash` fails (a locked `.git/index.lock`) | `clean` exits 8 with `NOT-CLEANED`, and the test asserts the file is still dirty (nothing was destroyed) |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/leftover-test.sh 2>&1 | tail -1 | grep -q 'RESULT: PASS'`
- [ ] `bash -n scripts/autonomy/leftover.sh && bash -n scripts/autonomy/cycle.sh`
- [ ] `grep -q 'leftover.sh' scripts/autonomy/cycle.sh`
- [ ] `grep -q 'OUTCOME=dirty-tree' scripts/autonomy/cycle.sh`
- [ ] `grep -c 'git checkout\|git reset --hard\|git clean -fd' scripts/autonomy/leftover.sh | grep -qx 0`
- [ ] `git diff --quiet -- specs scripts/autonomy/plan.tsv`

## Files
- `scripts/autonomy/leftover.sh`
- `scripts/autonomy/tests/leftover-test.sh`
- `scripts/autonomy/cycle.sh`

## Guardrails
- Do NOT edit `specs/`, `scripts/autonomy/plan.tsv`, or an existing assertion. Do NOT weaken the `-z` parsing, the
  session requirement of `owner`, or the exit code of `clean` to make a row pass: a `clean` that reports success on
  a tree it did not clean is the defect this spec exists to remove.
- Do NOT use `git checkout --`, `git reset --hard` or `git clean -fd` anywhere in `leftover.sh`. The only removal is
  a named stash, and it is printed with its restore command.
- Do NOT run a worker or a gate from the tests; they drive `leftover.sh` against a throwaway git fixture under `_tmp/`.
- If `cycle.sh`'s dirty-tree branch no longer matches the shape described at lines 143-167 (for example the session
  recovery is gone), STOP and say so in `## Closure` instead of forcing the change onto a different seam.
- Scratch under `_tmp/` inside the repository; never `/tmp`.

## Closure

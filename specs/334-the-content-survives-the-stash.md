# Spec: The content survives the stash — the index is saved, and the save comes first

## Overview

`leftover.sh clean` puts a tree the loop believes is its own into `git stash`. Its header promises that
"the content must stay recoverable", and the mechanism it relied on was **the stash itself**.

Measured 2026-11-02, reading the code against a fixture: `save()` captured `git diff` (unstaged tracked changes)
and `git ls-files --others` (untracked files) — and **nothing for the index**. `git stash push` does carry the
index. So a person who staged an edit and walked away had that edit recorded by exactly one thing: a stash commit
that `git stash drop` removes in one word. `clean()` never called `save()` at all, so even the two cases `save()`
did cover were not covered when the stash was the thing doing the cleaning.

Evidence, before the fix (control run against the script taken from git):

```
FAIL a staged-only change is saved to -staged.patch (got: SAVED …/cid1-paths.txt)
FAIL the content survives dropping the stash (leftovers dir holds: )
FAIL control: the paths file lists what the stash carried
RESULT: 3 ok, 3 FAIL
```

and after it, the same file with the same fixture: `RESULT: 6 ok, 0 FAIL`.

**Project**: /projects/pi-improved
**Phase**: production hardening (path item `n1`, Opus review follow-up)

## Requirements

**R1 — the index is content.** `save <dir> <cid>` writes `<cid>-staged.patch` from `git diff --cached` when the
index is not clean, in addition to `<cid>-tracked.patch` and `<cid>-untracked.tar`. A staged-only change produces
the staged patch and no tracked patch.

**R2 — the save precedes the stash, and a failed save forbids the stash.** `clean <cid>` calls `save` into
`PII_LEFTOVER_DIR` (default `_tmp/autonomy/leftovers`) **before** `git stash push`. When the save fails it returns
8 with `NOT-CLEANED` and does not stash, because a stash must never be the only copy.

**R3 — the record is not mistaken for unfinished work.** A clean whose leftovers dir sits inside the tree must not
report that directory as a path still dirty. `still_dirty` lists individual files (`--untracked-files=all`) and
excludes the leftovers dir.

**R4 — a refusal states its verdict.** Every `clean` that does not clean ends with a line `NOT-CLEANED`, after the
reason, so a caller greps one word. Before this, the ownership refusal printed only `REFUSED`.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `a staged-only change is saved to -staged.patch` | a staged edit, nothing unstaged | `-staged.patch` holds it |
| `and the unstaged patch is not invented for it` | the same | no `-tracked.patch` |
| `control: an unstaged change is saved as -tracked.patch` | the same edit, unstaged | `-tracked.patch`, no `-staged.patch` |
| `clean() stashed the owned tree` | one owning row, a staged edit | tree clean, rc 0 |
| `the content survives dropping the stash` | `git stash drop` after that clean | the edit is still in the leftovers dir |
| `control: the paths file lists what the stash carried` | the same | `r1-paths.txt` names it |

**Control that proves the test can fail**: `PII_LEFTOVER_SH=<old script>` runs the same assertions against the
script taken from git, where three of them fail.

## Exit Criteria

- [ ] `bash scripts/autonomy/tests/leftover-saves-the-index.test.sh` → `RESULT: 6 ok, 0 FAIL`
- [ ] `PII_LEFTOVER_SH=/tmp/old-leftover.sh bash scripts/autonomy/tests/leftover-saves-the-index.test.sh` → at least 3 FAIL
- [ ] `bash scripts/autonomy/tests/leftover-owns-the-whole-tree.test.sh` → 0 FAIL
- [ ] `bash scripts/autonomy/tests/leftover-test.sh` → `RESULT: PASS`

## Guardrails

- Do NOT delete a stash to "clean up": the stash is a convenience, the file in the leftovers dir is the record.
- Do NOT relax the ownership rule (one row names every dirty path) to make a clean succeed.
- Do NOT `git checkout --`, `git reset --hard` or `git clean` anywhere in this path.

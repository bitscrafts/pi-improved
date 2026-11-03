# Spec: A park cannot wedge the queue

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening · **Source**: Opus optimisation review 2026-11-03,
workstream **(lane hardening, not from the report)** (measured 2026-11-03).

## Verified premises

- `scripts/autonomy/cycle.sh` parked spec 352 at the attempt cap and left **nine** changed/untracked files in
  the tree (measured: `git status --porcelain | wc -l` → 9 after the park).
- `scripts/autonomy/cycle.sh` refuses to dispatch on a dirty tree, so row 353 was never dispatched: the night
  produced nothing after the park, and the parked work was good (352's five tests pass).
- `git checkout --` is banned in this workspace (it destroys uncommitted work), so returning the tree to HEAD
  cannot use it.

## Requirements

**R1.** A tool `scripts/autonomy/quarantine-wip.sh <row> [reason]` copies every changed and untracked path to
`/projects/_trash/<row>-<reason>-<ts>/` with the tree layout preserved, then returns the tree to HEAD content using
`git show HEAD:<path>` for tracked files and `mv` for untracked ones. It prints the aside directory, or `CLEAN`.
It never deletes anything and never invokes `git checkout`.

**R2.** Both park paths in `cycle.sh` (attempt cap, and three strikes at verify) call it, and the row's park note
names the aside directory.

**R3.** If the tree cannot be returned to clean, the tool exits 3 and cycle.sh says so loudly instead of pretending.

**R4.** The tool skips its own aside directory if a caller places it inside the repository.

## The requirement a plausible implementation satisfies in appearance only

A worker can make the park delete the changes (`git checkout -- . && git clean -fd`) and the tree is clean — and the
parked work, which was worth three worker passes, is gone forever. The test below asserts the WORK SURVIVES ASIDE,
not only that the tree is clean.

## TDD contract

`scripts/checks/tests/quarantine-wip.test.sh`:

| case | setup | assertion |
|---|---|---|
| `a_modified_untracked_and_nested_path_survive` | scratch repo: one modified tracked file, one untracked file, one nested untracked file | tree clean afterwards; all three present under the aside dir; the tracked file's aside copy holds the WORK, the tree holds the HEAD content |
| `CONTROL_a_clean_tree_says_clean` | clean scratch repo | prints `CLEAN`, creates no aside dir |
| `CONTROL_checkout_is_never_used` | run it, then `grep -c 'checkout'` the tool | 0 |

## Exit Criteria

- [ ] `bash scripts/checks/tests/quarantine-wip.test.sh 2>&1 | grep -q 'QUARANTINE: 3 passed, 0 failed'`
- [ ] `grep -c 'git checkout' scripts/autonomy/quarantine-wip.sh | grep -q '^0$'`
- [ ] `grep -q 'park_and_quarantine' scripts/autonomy/cycle.sh`
- [ ] `bash -n scripts/autonomy/cycle.sh`

## Guardrails

- Do NOT use `git checkout --`, `git restore`, or `git clean` inside the repository: the work must be moved, and the
  tree restored from HEAD content only.
- Do NOT delete the aside directory; retention is the trash job's business (30 days).
- Do NOT quarantine when a worker is in flight: only a park calls this.
- The worker does not commit; the orchestrator reviews and commits.

## Files

- `scripts/autonomy/quarantine-wip.sh` (new)
- `scripts/checks/tests/quarantine-wip.test.sh` (new)
- `scripts/autonomy/cycle.sh` (both park paths)

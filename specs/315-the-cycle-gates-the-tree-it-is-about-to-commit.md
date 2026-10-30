# Spec 315: the cycle gates the tree it is about to commit

## Overview
**Project**: /projects/pi-improved
**Phase**: production — gate honesty, the other half of spec 314. Spec 314 made `bash quality-gate.sh` run the tests
of the crates a change touches, so the *working tree* is now judged. The tree that **lands** is a different object:
`git archive HEAD` plus exactly the paths the row stages. §9a of `_docs/05-operations/open-items.md` filed the gap and
measured it twice:

- `7f98624` (spec 175) committed a call to `pii_server::resolve_graph_dir`, a function that existed **only in the
  uncommitted working tree** — `HEAD` did not build `pii-cli` (rc=101 on a clean export).
- `HEAD`'s `Cargo.lock` was stale for a committed `pii-channels` dev-dependency: `cargo metadata --locked` failed
  (rc=101) on a clean export of `HEAD`, and passed once the lock was committed.

Both passed the loop's gate because the gate ran on the dirty tree. Spec 245 added `head-builds.sh` — an export of
`HEAD` plus `cargo check --workspace` — but `cycle.sh:435` runs it **after** `git commit`: the broken commit exists in
history, and the item then parks as `needs-review last=head-broken`. That is detection, not prevention. The door
already exists and is not used: `head-builds.sh --worktree` is documented on its own third line as "the pre-commit
door", and it overlays the worktree, which is not what a commit carries.

**The cost is now measured, which is what §9a asked for** (2026-10-30, this host, `git archive HEAD` into
`/projects/_tmp/stage-gate-cost-20261030T002931Z`): `cargo metadata --locked` **0 s**; `cargo check -q --workspace` in
a **new** target dir **38 s**; the same check of a second export against that target **8 s**. A pre-commit door that
reuses the warm `_tmp/head-builds-target` (2.0 G, already kept by spec 245) is therefore ~8 s per item — affordable
where a 45-min export gate is not (the reason this was not authored as "run fresh-gate before the commit").

## Verified Premises
- **CONFIRMED (read, 2026-10-30)** `cycle.sh:399-431`: the commit step stages exactly the row's `;`-separated paths
  (`git add -- "${P[@]}"`, line 401), refuses unlisted changes (`unlisted-changes.sh`, 404), closes an already-satisfied
  item (410-421), then commits at 430. There is no build of the pending tree anywhere in that span.
- **CONFIRMED (read, 2026-10-30)** `head-builds.sh` (32 lines) exports `HEAD` with `tar -x -m`, overlays modified +
  untracked-not-ignored files **only** under `--worktree`, runs `cargo check -q --workspace` with
  `CARGO_TARGET_DIR=${HEAD_BUILDS_TARGET:-$ROOT/_tmp/head-builds-target}`, prints `HEAD <sha> builds|does NOT build`,
  and removes only its own `mktemp -d`. `cycle.sh:435` calls it **without** `--worktree`, after the commit.
- **CONFIRMED (measured above, 2026-10-30)** the three cost readings; and `_tmp/head-builds-target` exists at 2.0 G,
  so the warm path is real and not a hope.
- **CONFIRMED (read, 2026-10-30)** `git diff --cached --name-only -z --diff-filter=ACMR` / `--diff-filter=D` are the
  index's own account of what the commit will carry; `Cargo.lock` (100590 bytes) sits at the repo root.
- **CONFIRMED (read, 2026-10-30)** `cycle.sh` already uses exit codes 3-17 and reserves 18 for nothing; `env.sh` is
  sourced at line 31 with a comment that the loop's PATH contract lives there.
- **NOT CLAIMED**: that `cargo check` is a substitute for the gate's tests. It is not; it is what proves the *commit*
  builds. The tests stay where 314 put them.

## Requirements

### Functional
- **R1.** A committed helper `scripts/autonomy/pre-commit-builds.sh` exports `HEAD` (`git archive HEAD` piped into
  `tar -x -m`, the `-m` spec 245 needed so two commits a second apart do not share a cargo fingerprint) into a scratch
  directory it creates under `_tmp/pre-commit-<mktemp>` itself, applies **the index** onto that export, and runs
  `cargo metadata --locked --format-version 1` then `cargo check -q --workspace` there.
- **R2.** The applied change is the **index, never the worktree**: `git diff --cached --diff-filter=ACMR --name-only -z`
  paths are copied from the worktree into the export, `--diff-filter=D` paths are removed from it. A modification
  present only in the worktree is not gated — that is the whole point: the commit is the object under test.
- **R3.** `--print` prints the tree it would judge (`HEAD <sha> + <n> staged path(s), <m> deleted`), the exact commands
  it would run, and the target dir, then exits 0 without running anything. With an empty index either mode prints
  `pre-commit: nothing staged — nothing to gate` and exits 0 — a printed line, never silence.
- **R4.** On green it prints `pre-commit: the pending tree builds (<s>s)` and exits 0. On red it prints
  `pre-commit: the pending tree does NOT build` plus up to 10 lines matching `^error` from the cargo output and exits
  **1**. If the export, the index read or `cargo` on `PATH` fails, it prints `pre-commit: <why>` and exits **2** — a
  loud failure, never a green.
- **R5.** The helper writes nothing to the repository tree, runs no network step, adds no dependency, stays under 450
  effective lines, and removes only the scratch directory it created in that run.
- **R6.** `cycle.sh` runs the helper **after** the unlisted-changes guard and the already-satisfied branch, and
  **before** the `git commit` line — under `timeout "${PRE_COMMIT_TIMEOUT:-600}"`, with
  `PRE_COMMIT_BUILDS` defaulting to `scripts/autonomy/pre-commit-builds.sh` (the `HEAD_BUILDS` pattern at line 435).
- **R7.** On a non-zero helper status the cycle does **not** commit: it sets the row `status=needs-review` with
  `last=precommit-broken: <first error line, truncated>` (rc 124 instead sets `last=precommit-timeout — the check was
  KILLED, not failed`), appends the helper's output to the cycle log, prints the verdict, and exits **18** with
  `OUTCOME=precommit-broken`. `attempts` is **not** incremented: a tree that does not build is not a worker failure
  signature the three-strike rule should spend, and the index stays staged so the next cycle resumes the same session
  at verify instead of re-buying the spec.

### Non-Functional
- The step prints its own duration, so the cycle log shows what gating the commit cost.
- The helper is a bash script like its neighbours: `#!/usr/bin/env bash`, `set -u`, `. env.sh` for the PATH contract,
  no `set -e` around the cargo calls (the statuses are the verdicts).

## TDD Contract
`scripts/autonomy/tests/pre-commit-builds.test.sh` prints `N ok, M FAIL` as its last line (the house style) and drives
the helper **through its live interface** — it never re-types the export/overlay logic. It builds its own scratch cargo
workspace under `_tmp/` (a `app` crate with a path dependency on `util`, committed as `HEAD`) and points the helper at
it with `PRE_COMMIT_REPO`, so every row is a real `git archive` + real `cargo`.

| Test | Input | Expected |
|---|---|---|
| `a-staged-broken-file-refuses` | stage an edit calling `util::no_such_fn()` | exit **1**, output matches `cannot find` or `error\[E` |
| `CONTROL-a-staged-good-change-builds` | stage an edit calling an existing `util` fn | exit **0**, prints `the pending tree builds` |
| `CONTROL-the-worktree-alone-is-not-read` | the same broken edit left **unstaged** | exit **0**, prints `nothing staged` |
| `a-staged-deletion-is-applied` | stage deleting `src/gone.rs` **and** the `mod gone;` line | exit **0** |
| `CONTROL-a-deletion-without-its-mod-refuses` | stage deleting `src/gone.rs` only | exit **1**, output names the missing module |
| `a-stale-lock-refuses` | stage a new `[dependencies]` line in `app/Cargo.toml`, not `Cargo.lock` | exit **1**, output matches `--locked` or `lock file` |
| `CONTROL-a-committed-lock-passes` | stage both the manifest and the updated lock | exit **0** |
| `CONTROL-a-clean-index-says-so` | nothing staged | exit **0**, prints `nothing to gate` |
| `--print-names-the-warm-target` | `--print` with one path staged | output names `_tmp/head-builds-target` and `cargo check -q --workspace`, and no compile happens |
| `the-cycle-calls-it-before-the-commit` | `cycle.sh` read at run time | the `pre-commit-builds.sh` invocation's line number is **lower** than the `git -c user.name=` line's, and the invocation carries `timeout` |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/pre-commit-builds.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `! bash scripts/autonomy/tests/pre-commit-builds.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `bash -n scripts/autonomy/pre-commit-builds.sh`
- [ ] `bash scripts/autonomy/pre-commit-builds.sh --print | grep -q "head-builds-target" || bash scripts/autonomy/pre-commit-builds.sh --print | grep -q "nothing to gate"`
- [ ] `grep -q "PRE_COMMIT_BUILDS:-scripts/autonomy/pre-commit-builds.sh" scripts/autonomy/cycle.sh`
- [ ] `awk '/pre-commit-builds\.sh/ {p=NR} /git -c user.name="/ {c=NR} END{exit !(p && c && p<c)}' scripts/autonomy/cycle.sh`
- [ ] `grep -q "OUTCOME=precommit-broken" scripts/autonomy/cycle.sh && grep -q "exit 18" scripts/autonomy/cycle.sh`
- [ ] `git diff --quiet -- gate-steps.txt gate-steps-quick.txt quality-gate.sh scripts/autonomy/head-builds.sh scripts/autonomy/state.py`

## Files
- `scripts/autonomy/pre-commit-builds.sh`
- `scripts/autonomy/tests/pre-commit-builds.test.sh`
- `scripts/autonomy/cycle.sh`

## Guardrails
- Do NOT gate the worktree. The index is the object under test; copying the worktree in is the defect this spec exists
  to close (`head-builds.sh --worktree` remains the operator's manual door, not the cycle's).
- Do NOT `git stash`, `git checkout`, `git reset`, `git add` or `git commit` in the helper or in the new cycle step:
  the staged content must survive a red so the next cycle resumes at verify.
- Do NOT run the gate's test list here. This step proves the commit builds; `gate-steps.txt` remains the boundary that
  runs everything, and `head-builds.sh` keeps its post-commit role (prevention does not replace detection).
- Do NOT touch `gate-steps.txt`, `gate-steps-quick.txt`, `quality-gate.sh`, `head-builds.sh` or `state.py`.
- Do NOT make a missing `cargo`, an unreadable index or a failed export a pass: exit 2 and say which.
- Do NOT reuse a *new* target directory: the measured 8 s depends on `_tmp/head-builds-target`, and a cold dir is 38 s
  per item (measured); the choice must be visible in `--print`.
- Do NOT edit `specs/` or `scripts/autonomy/plan.tsv`.
- **If the premises do not hold** — `cycle.sh` no longer stages the row's paths before committing, `git archive HEAD`
  is unavailable in this repo, or `head-builds.sh` no longer honours `HEAD_BUILDS_TARGET` — STOP, record the
  measurement in `## Closure`, and change nothing else. A spec whose premise is false is not implemented by force.

## Follow-ups (not this spec)
1. `head-builds.sh` and the new helper share export+overlay logic; unifying them is a refactor worth doing once both
   have run in the loop for a week.
2. `head-builds.sh` does not run `cargo metadata --locked`; the stale-lock class (`§9a`) is currently caught only by
   this spec's pre-commit door, and adding it there would make the post-commit door catch it too.

## Closure
(pending — the orchestrator records the criteria on the worker's tree, the mutation that reddens the new test with
its byte-identical restore, and `fresh-gate`'s verdict.)

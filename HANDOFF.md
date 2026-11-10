# Pii — Handoff

The current state of the project. How it got here, session by session, is in
[`_docs/05-operations/BUILD-LOG-2026-09-to-11.md`](_docs/05-operations/BUILD-LOG-2026-09-to-11.md);
what pii is, how to build it and how a change is specified is in [`README.md`](README.md) and [`AGENTS.md`](AGENTS.md).

## State

- One binary, `pii`, built from 22 crates under `crates/` plus `xtask`. The gate is `gate-steps.txt`, run by `quality-gate.sh`; a change is
  judged by `cargo run -q -p xtask -- fresh-gate --worktree`, which builds a clean checkout.
- Work enters as a spec in `specs/` and a row in `scripts/autonomy/plan.tsv` (7 columns: id, spec file, brief, title, criteria, paths, lane).
  `scripts/autonomy/state.py` holds each row's status; `scripts/autonomy/cycle.sh` dispatches one row, gates it and commits it.
  A worker never commits.
- A cycle runs about every five minutes under cron and takes a lock, so nothing may edit the tree while one runs: take the hold first
  (`scripts/autonomy/hold.sh`) or wait for the cycle to end.

## Queue (as last measured, 2026-11-10)

- Row 404 landed: a called graph's `{{nodes.X}}` prompt placeholders are renamed with the node, like `$.nodes.X`.
- Row 403 (a fan-out of read-only reviewers: a one-reviewer graph and a four-slot graph that calls it, with rubrics and fixtures) is parked.
  Its files are saved outside the tree and in a git stash (`git stash list`, the entry named `row 403 work after attempt 2`). With row 404 in
  place the graph loads; three of the nineteen tests in its test file (`review_graph`) still fail, and the cause has not been pinned down.
  Resume with `scripts/resume-row-atomic.sh 403` only after reading those three failures.
- Rows 354, 366 and 368 are parked or waiting for review.

## Disk

A cold gate rebuilds about 38 GB of build output under `_tmp/gate-target`, and the cycle refuses to gate below 20 GB free. When it refuses, the
row is left `gate-skipped:disk` with its work in the tree; free space (`cargo clean --target-dir` on a target directory) and resume. Moving
`_tmp/gate-target` to a larger volume needs its own spec.

## Open

- Row 403, as above.
- A spec for a repository field on a plan row, so the cycle can work in another project's checkout, and a `github` channel that reads
  maintainer-labelled issues. Neither is written; both go through the spec path.
- The commit history carries two older author identities (556 commits as a host-local address, 306 as an orchestrator) and 84 commits whose
  messages name a tool. It is rewritten once, by the owner, together with a force-push; the 14 commits after `0499c09` are local only.
  `git filter-repo` is not installed here; the laya-rs rewrite used `git filter-branch` with a hash-remapping filter.
- The repository is public with a CI workflow (`checks.yml`, on every push) that has not yet run on GitHub: the commits that add it are
  not pushed. The full gate (`gate.yml`) is manual and weekly until it has run on a runner. The licence is MIT (`LICENSE`).

## Rules that bite

- Never `rm -rf` or `git checkout --` on existing work; move it aside. Never commit from a worker. Never lower a floor or weaken a check to
  make a row land.
- A criterion that passes before the work refuses its row at the RED step. Put such a check under Guardrails, and run
  `scripts/red-audit.sh <row>` before queueing.

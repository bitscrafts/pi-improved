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

## Queue (as last measured, 2026-11-12)

- Landed: 404 (prompt placeholders renamed with their node), 405 (the queue runs in the repository it is started from), 406 (a `github`
  channel that hands the queue only issues carrying the owner's `ready` label, read from the issue's event log), 407 (a repository declares
  its own gate in `.pii/gate`; without the file the gate is unchanged), 408 (a test that only the event-log check can satisfy), 409 (an inlined
  graph's compound expressions are renamed reference by reference), 410 (a refusal's reason can name what was refused, with `{{placeholders}}`),
  403 (the read-only reviewer fan-out, 19 tests).
- Rows 354, 366 and 368 are parked or waiting for review.

## Not yet proven

- No real `ready`-labelled GitHub issue has gone through the `github` channel into the queue; its tests use a fake transport.
- No real row of another repository has run that repository's declared gate through the queue; row 407's tests cover the selectors and a
  dry-cycle refusal.
- Three autonomy shell tests (`lane-graph-knob`, `lane-graph-wall`, `lane-preflight`) pass only at `/projects/pi-improved`: the scripts they
  exercise write under that absolute path. A fresh clone elsewhere fails them, so the full gate cannot be proven from a clone until those
  scripts find their own root. Specify it as a row; do not edit the tests to pass elsewhere.
- `control_plane_is_not_behind_runs` (pii-cli) can time out waiting on a lock when the machine is loaded; it passes alone.

## Disk

A cold gate rebuilds about 38 GB of build output under `_tmp/gate-target`, and the cycle refuses to gate below 20 GB free. When it refuses, the
row is left `gate-skipped:disk` with its work in the tree; free space (`cargo clean --target-dir` on a target directory) and resume. Moving
`_tmp/gate-target` to a larger volume needs its own spec.

## History

The history was rewritten once, on 2026-11-12: one author identity, no commit message naming a tool, commit ids quoted in docs and
comments carried to the new ids. Test data was left alone, because an id inside a fixture is an opaque value. Clones and forks made before
that date still hold the old commits, and ids quoted outside the repository will not match.

## Rules that bite

- Never `rm -rf` or `git checkout --` on existing work; move it aside. Never commit from a worker. Never lower a floor or weaken a check to
  make a row land.
- A criterion that passes before the work refuses its row at the RED step. Put such a check under Guardrails, and run
  `scripts/red-audit.sh <row>` before queueing.

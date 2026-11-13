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
- A row whose spec's card line names an unfinished `depends=` is not offered (`pick` skips it and names the row it waits for, spec 420).
  `pick --row <id>` is deliberately exempt: naming a row is a deliberate act.

## The issue round trip (2026-11-13)

An issue carrying the owner's `ready` label is now a complete path, and each step is a command:

```
scripts/mark-ready.sh <issue>          the owner's label; works on a repo that never had `triage` (spec 417)
  -> the `github` channel starts graphs/github-issue.toml (spec 406)
     -> a comment, one drafted spec under specs/, a comment naming it (specs 413, 414, 415)
  -> the orchestrator queues the draft as a row
scripts/report-row.sh <row> --issue <N>  one comment naming row, status, commit and spec; closes the issue when the row is done (spec 419)
```

The draft is a file for a person to read: nothing is queued and no coder runs until the orchestrator puts it in the plan. Every claim in those
commands is read back from the issue, not assumed from a call's exit status.

## Queue (as last measured, 2026-11-13)

- Landed: 404 (prompt placeholders renamed with their node), 405 (the queue runs in the repository it is started from), 406 (a `github`
  channel that hands the queue only issues carrying the owner's `ready` label, read from the issue's event log), 407 (a repository declares
  its own gate in `.pii/gate`), 408 (a test that only the event-log check can satisfy), 409 (inlined compound expressions renamed by
  reference), 410 (a refusal's reason can name what was refused), 411 (the lane script and its tests find the repository they are in),
  412 (an issue is delivered once per `ready` label, not once per update), 413/414/415 (a `ready` issue is acknowledged and drafted, the
  draft lands in the daemon's workspace, and the run posts only the graph's own comments), 416 (the heavy-test lock waits for a holder that
  keeps its heartbeat), 417 (`mark-ready.sh` on a repository that never had `triage`), 418 (an explicit `--store` owns every durable store
  beside it), 419 (a landed row reports itself on its issue and closes it), 420 (the queue does not offer a row whose dependency is
  unfinished), 403 (the read-only reviewer fan-out).
- Rows 354, 366 and 368 are parked or waiting for review. Rows 298, 299, 300, 303 and 319 are still refused by the row judges.

## Not yet proven

- No row of another repository has run that repository's declared gate through the queue; row 407's tests cover the selectors and a
  dry-cycle refusal.
- The installed `~/.local/bin/pii` predates the `github` channel: only a build from this tree runs it. The deploy card must carry the
  channel before an operator installs it.
- A pull request is not read at all: the channel polls issues only, and the queue lands on `master`. Whether a row should land as a PR is
  undecided.

## Disk

A cold gate rebuilds about 38 GB of build output under `_tmp/gate-target`, and the cycle refuses to gate below 20 GB free. When it refuses, the
row is left `gate-skipped:disk` with its work in the tree; free space (`cargo clean --target-dir` on a target directory) and resume. Moving
`_tmp/gate-target` to a larger volume needs its own spec.

A lock taken with `flock` **without `-o`** is inherited by whatever the holder runs: a `cargo` build starts `sccache`, which outlives the holder
and keeps the lock, so the next cycle waits minutes for a lock nobody holds (measured 2026-11-13). Take locks with `flock -o`.

## History

The history was rewritten once, on 2026-11-12: one author identity, no commit message naming a tool, commit ids quoted in docs and
comments carried to the new ids. Test data was left alone, because an id inside a fixture is an opaque value. Clones and forks made before
that date still hold the old commits, and ids quoted outside the repository will not match.

## Rules that bite

- Never `rm -rf` or `git checkout --` on existing work; move it aside. Never commit from a worker. Never lower a floor or weaken a check to
  make a row land.
- A criterion that passes before the work refuses its row at the RED step. Put such a check under Guardrails, and run
  `scripts/red-audit.sh <row>` before queueing.
- A row owns a dirty tree only if the row's paths column names EVERY dirty path (`scripts/autonomy/leftover.sh owner`). A row with no session
  owns nothing, so orchestrator-written work records this session as the row's session before landing, or the tree is treated as a stranger's.

# Spec: the deployment installs the graphs its config names

## Overview

Spec 142 produced `deploy/install-pii.sh`, `deploy/rollback-pii.sh` and the runbook, and its four exit
criteria hold (closure, 2026-10-13). The orchestrator's live check found one defect the criteria could not
see: **the graphs are never installed**, so the deployed daemon cannot do the one thing it was deployed for.

Measured on the tree at `0fcde3e`:

- The daemon resolves a graph as `<runtime_dir>/graphs/<name>.toml`
  (`crates/pii-cli/src/cmd_daemon.rs:391`, `graph_dir: runtime_dir.join("graphs")`).
  `Engine::load_spec` (`crates/pii-server/src/lib.rs:588`) returns `NotFound("graph '<name>'")` for any name
  but `default` when that file is absent. The learn verifier reads the same dir
  (`crates/pii-learn/src/verify.rs:333`).
- The installer writes `graph = "discord-triage"` (the `[[channel]]`) and `graph = "discord-digest"` (the
  `[[cron]]`) into `pii.toml`, but copies neither `graphs/discord-triage.toml` nor `graphs/discord-digest.toml`
  anywhere. After a live run of the real binary on the generated config, `<runtime_dir>` held `channel.redb`,
  `timer.redb`, `channels/`, `memory/` and **no** `graphs/`. Every Discord message ends in `graph not found`.
- `runtime_dir` is under `/run` (tmpfs): a file placed there once vanishes on reboot. The graphs must live in
  a persistent directory and be **placed in `<runtime_dir>/graphs` on every unit start**.
- Both graphs lint clean today: `target/debug/pii graph lint graphs/discord-triage.toml` and
  `... discord-digest.toml` exit 0 (V5 warnings only).

Two minor defects from the same closure are folded in because they live in the same two files:

- A second `rollback-pii.sh` finds the now-empty newest `.old-*` dir, restores 0 files and exits 0 — a
  rollback that did nothing reports success.
- The appended `pii.toml` comment says the forum id never lives in `pii.toml`, but the cron's
  `input = { to = ... }` writes it there. The comment is false (the id is not a secret; the token is
  correctly kept out).

**Project**: <repo>
**Phase**: post-vision hardening (059 §4 rank 1, completing spec 142)
**Source**: `specs/142-deploy-head-on-the-host.md` `## Closure`

## Requirements

### Functional
- **R1** `deploy/install-pii.sh` copies every `graphs/*.toml` in the repo's top-level `graphs/` dir (not
  `graphs/starter/`) into a **persistent** `$STATE_DIR/graphs/`. An existing file there whose content differs
  is moved aside to the run's dated `$ASIDE` dir first (never deleted, never silently overwritten); an
  identical one is left as is. The installer prints which graphs it installed.
- **R2** The generated unit places the graphs in `$RUNTIME_DIR/graphs` on **every start**, via an
  `ExecStartPre=` line that runs after the existing `mkdir -p $RUNTIME_DIR` line. Either a symlink
  (`$RUNTIME_DIR/graphs -> $STATE_DIR/graphs`) or a copy is acceptable; whichever is chosen must leave
  `$RUNTIME_DIR/graphs/discord-triage.toml` readable after the pre-start lines run on an **empty**
  `$RUNTIME_DIR`, and must be idempotent (a second start on a non-empty `$RUNTIME_DIR` succeeds).
- **R3** Preconditions (before anything is moved): every graph the generated config names
  (`discord-triage`, `discord-digest`) exists as `graphs/<name>.toml` in the repo. A missing one aborts with
  the name and changes nothing.
- **R4** `deploy/rollback-pii.sh`, when the newest `.old-*` dir holds nothing to restore, prints
  `NOTHING TO RESTORE` with the dir it looked at and exits non-zero. A rollback that restores ≥ 1 file keeps
  its current behaviour and output (`RESTORED ...`).
- **R5** The appended `pii.toml` comment states the truth: the token comes only from the environment file;
  the forum id is written into the cron's `input` (it is an id, not a secret).
- **R6** `scripts/checks/deploy-dry-run.sh` asserts R1–R4 in its scratch prefix, before its final line:
  1. after install, `$STATE_DIR/graphs/discord-triage.toml` and `discord-digest.toml` exist and are
     byte-identical to the repo's;
  2. the generated unit's `ExecStartPre=` lines, extracted from the unit file and executed in order against an
     empty `$RUNTIME_DIR`, leave `$RUNTIME_DIR/graphs/discord-triage.toml` and `discord-digest.toml` readable
     and byte-identical to the repo's; executing them a second time also succeeds;
  3. a re-install over a `$STATE_DIR/graphs/discord-triage.toml` the test edited first moves the edited copy
     into the new `.old-*` dir;
  4. a second rollback (after the one the script already runs) exits non-zero and prints `NOTHING TO RESTORE`.
  The last line stays `DRY-RUN OK` only when every assertion held.
- **R7** `_docs/05-operations/deploy-runbook.md` names `$STATE_DIR/graphs` as where the graphs live, says the
  unit places them in the runtime dir on each start, and adds a verification step: after start,
  `ls <runtime_dir>/graphs/` lists `discord-triage.toml` and `discord-digest.toml`.

### Non-Functional
- Shell scripts pass `bash -n`, keep `set -euo pipefail`, quote every path, never `rm` anything (HARD RULE 0:
  move aside, never delete).
- No secret in a committed file. The `pii-mcp` swap stays out of the installer.
- No Rust change. If the worker concludes the daemon itself must change (e.g. a configurable `graph_dir`),
  it STOPS and says so instead of making that change.

## TDD Contract

The acceptance is the scratch-prefix exercise in `scripts/checks/deploy-dry-run.sh`, extended first:

| Exercise | Input | Expected Output |
|---|---|---|
| add the R6 assertions to the dry-run, before touching the installer | current `deploy/` | the dry-run FAILS on R6.1 (no `$STATE_DIR/graphs`) — record the failing line |
| install | scratch prefix | graphs in `$STATE_DIR/graphs`, identical to the repo's |
| run the unit's `ExecStartPre=` lines on an empty runtime dir, twice | generated unit | both graphs readable under `$RUNTIME_DIR/graphs`; second run rc 0 |
| re-install over an edited graph | edited `$STATE_DIR/graphs/discord-triage.toml` | edited copy found in the new `.old-*` dir |
| second rollback | after the first | rc ≠ 0, `NOTHING TO RESTORE` |

## Exit Criteria

- [ ] `bash -n deploy/install-pii.sh`
- [ ] `bash -n deploy/rollback-pii.sh`
- [ ] `bash scripts/checks/deploy-dry-run.sh 2>&1 | tail -1 | grep -q 'DRY-RUN OK'`
- [ ] `grep -q 'ExecStartPre=.*graphs' deploy/install-pii.sh`
- [ ] `grep -q 'NOTHING TO RESTORE' deploy/rollback-pii.sh && grep -q 'NOTHING TO RESTORE' scripts/checks/deploy-dry-run.sh`
- [ ] `grep -q 'STATE_DIR/graphs\|state/pii/graphs\|lib/pii/graphs' _docs/05-operations/deploy-runbook.md`
- [ ] `test -f _docs/05-operations/deploy-runbook.md && grep -q 'rollback' _docs/05-operations/deploy-runbook.md`

## Guardrails

- Do NOT reach the host, enable a service, or touch any real systemd: everything runs in the dry-run's
  scratch prefix.
- Do NOT change any Rust crate. A genuine shape mismatch (the daemon cannot read graphs from a placed dir) is a
  STOP-and-report, not a code change.
- Do NOT weaken or remove any existing dry-run assertion; only add.
- Do NOT delete or overwrite an existing file without moving it aside first.
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit.

## Closure

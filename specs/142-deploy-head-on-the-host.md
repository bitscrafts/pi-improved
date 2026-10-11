# Spec: the deployment exists as files the host can run in one command

## Overview

The single most valuable change measured on 2026-10-11 is not code: the tree is ahead of the machine
(`_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §0, §4 rank 1). The installed `pii` is dated
2026-09-17 and its one-shot exits 5; 86 commits of work have never run anywhere but in tests. Nothing else on
the list can be measured until a real process runs on the host — "a person sees pii work" is the missing fact.

Two constraints shape this spec:

- **The orchestrator has no host access.** Measured: from this container `ssh lnx` is denied and there is no
  `podman`; the deploy is therefore **a script the operator runs**, and its acceptance is what can be proven
  in a scratch prefix on this side plus a runbook with the exact commands.
- **The user's directive is systemd, not Docker** (2026-10-10), and `pii-mcp` **is not** part of the same
  change: swapping the binary Hermes calls is a separate, later step behind its own probe (059 §2.4).

**Project**: /projects/pi-improved
**Phase**: post-vision hardening (059 rank 1); `RESUME-REGISTER` deployment row
**Source**: `_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §4, §2.3

## Requirements

### Functional
- **R1** `deploy/install-pii.sh` — one command, idempotent, safe to re-run:
  1. build the release binaries (`pii`, `pii-mcp`, `pii-web`, `pii-evolve`);
  2. move the existing installed binaries aside to a **dated** directory (`…old-$(date +%s)`), never delete;
  3. install the new binaries to the target prefix (a variable, defaulting to `$HOME/.local/bin`);
  4. write the runtime config from `examples/pii.production.toml` with, explicitly: model
     `llamacpp-server:granite-4.2-3b` and `[llama] host` pointing at the running llama.cpp server
     (`http://10.10.10.217:8082/v1`), trajectories on, **one** Discord forum channel, and **one** daily
     `discord-digest` cron entry;
  5. install the systemd unit (user scope or the documented system scope), and
  6. **print** the exact remaining operator commands (enable/start the unit, place the token) rather than
     doing anything it cannot verify.
- **R2** `deploy/rollback-pii.sh` restores the moved-aside binaries, stops the unit if it is running, and
  prints what it restored. It must be **tested**: install into a scratch prefix, run the rollback against it,
  and show the previous binary answering again. A rollback that has never been run is not a rollback.
- **R3** Preconditions are checked **before** anything is moved: the llama.cpp host answers
  (`curl -s …/v1/models`), the token env var is present for a Discord channel, the target prefix exists or can
  be created. A missing precondition aborts with a named reason and changes nothing.
- **R4** `_docs/05-operations/deploy-runbook.md` — the operator's document: the two host actions (enable the
  unit, place the token), the verification commands with their expected output
  (`pii health` build identity equals the deployed commit; `pii run get` on a fresh run; `pii metrics` shows
  `pii_channel_posts_total` incrementing; the Discord forum receives the digest), and the rollback command.
  It must also state the **measurement window**: ≥ 7 forum posts over 7 days and the Hermes call/error rate
  unchanged (059 §2.3).
- **R5** The `pii-mcp` swap is documented as a **separate step** with its own probe: after swapping, call
  `tools/list` once and compare the `mcp__pii__*` call rate against the previous 161-per-30-days baseline
  (059 §2.3), and roll back if the rate drops.

### Non-Functional
- Shell scripts pass `bash -n`, use `set -euo pipefail`, quote every path, and never `rm -rf` (HARD RULE 0:
  move aside, never delete).
- No secret is written into a committed file: the token is read from the environment or a file path the
  operator controls, and the runbook says which.
- The unit runs the daemon as the operator's user with the runtime dir the config names; no root requirement
  beyond what installing to the prefix needs.

## TDD Contract

Not applicable in the usual sense; the acceptance is the scratch-prefix exercise:

| Exercise | Input | Expected Output |
|---|---|---|
| `bash -n` both scripts | — | rc 0 |
| install into a scratch prefix (`PREFIX=/projects/_tmp/prefix/…`) | a fake "existing" binary placed there first | the old binary is moved aside with a date; the new one is installed; the unit file is written; the printed operator commands are complete |
| the rollback against that prefix | — | the old binary is back and answers; the moved-aside copy is gone from the dated dir (moved back, not copied) |
| a precondition failure (llama host unreachable) | `LLAMA_HOST=http://127.0.0.1:1/v1` | aborts before moving anything, names the host |

## Exit Criteria

- [ ] `bash -n deploy/install-pii.sh`
- [ ] `bash -n deploy/rollback-pii.sh`
- [ ] `bash scripts/checks/deploy-dry-run.sh 2>&1 | tail -1 | grep -q 'DRY-RUN OK'` (the scratch-prefix exercise, committed so it is re-runnable)
- [ ] `test -f _docs/05-operations/deploy-runbook.md && grep -q 'rollback' _docs/05-operations/deploy-runbook.md`

## Guardrails

- Do NOT attempt to reach the host, enable a service, or touch another machine's systemd: this spec produces
  the files and the runbook, and the operator runs them. (There is no host access from here; measured.)
- Do NOT delete or overwrite an existing binary without moving it aside first.
- Do NOT put a token, a path to a secret, or a private key into a committed file.
- Do NOT include the `pii-mcp` swap in the install script — separate step, separate probe (R5).
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit.

## Closure

(outcome, the dry-run transcript, the rollback exercise, and the exact operator commands — filled in when the
item is closed. The deployment itself is NOT closed by this item: it is closed by the operator's run and the
7-day measurement.)

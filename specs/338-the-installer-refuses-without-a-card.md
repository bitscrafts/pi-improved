# Spec: The installer that made production refuses without a card, and never overwrites a store

## Overview

Production on `lnx` was installed on 2026-11-02 by `deploy/install-pii-user.sh` (commit `b6e4f4e`). Opus read that
script the same night:

> That script contains **no card check** (`grep -c card` → 0). Lines 52-54 copy the container's
> `store.redb memory.redb approval.redb heal.redb` **over** production's. Re-running it silently replaces
> production data with dev data.

Both sentences are true and both are worse than the milestone claims: M4 ("one permission: the deploy card") is
enforced in `deploy/install-pii.sh:151` and bypassed by the installer that actually shipped. And the copy is a data
loss with no warning — `memory.redb` holds 12.6 MB of what pii has learned.

**Project**: /projects/pi-improved
**Phase**: production hardening (Opus audit, M4 re-scored stale)

## Requirements

**R1 — the card is required, in the installer that ships.** `deploy/install-pii-user.sh` refuses (exit 2) without
a deploy card, using the same five-field check as `install-pii.sh`. The check is one function in one place, called
by both installers, so the two cannot drift.

**R2 — an existing store is never overwritten.** If the target's `store.redb` (or `memory.redb`, `approval.redb`,
`heal.redb`) already exists, the installer **refuses** with the path and the reason, and prints the move-aside
command the operator may run. Copying dev stores over production ones is removed as a default; it becomes
`--seed-stores` and is itself refused when a target store exists unless `--force-seed` is also passed, which prints
what it will destroy.

**R3 — the remote home is computed remotely.** The `/root` → `$HOME` rewrite (`lines 44-48`) is a no-op round trip:
`$HOME` is evaluated in the container, where it is `/root`, so production's config kept container paths and every
run since deploy logged `trajectory.write.failed`. The installer asks the host for its home
(`ssh … 'printf %s "$HOME"'`) and substitutes that value.

**R4 — dry run is the default.** `--apply` is required to write anything; without it the installer prints every
action it would take, including the card's revision and the target paths.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `no card, no install` | a fixture card directory with no card | rc 2, `REFUSED`, and the target home untouched (byte-identical snapshot) |
| `a card, an existing store` | valid card, `store.redb` present | rc 2, names the store, prints the move-aside command, store **unchanged** (checksum) |
| `a card, no store, --apply` | valid card, empty target | rc 0; the remote home from the fixture, not `/root` |
| `control: the old installer overwrites` | the script from `git show` against the same fixture | the store's checksum changes (the defect) |
| `dry run writes nothing` | no `--apply` | rc 0, every planned action printed, target tree unchanged |

## Exit Criteria

- [ ] `bash deploy/tests/install-pii-user.test.sh 2>&1 | tail -1 | grep -qE 'RESULT: [5-9][0-9]* ok, 0 FAIL'`
- [ ] `bash -n deploy/install-pii-user.sh`
- [ ] `grep -q 'REFUSED' deploy/install-pii-user.sh`
- [ ] `grep -q 'seed-stores' deploy/install-pii-user.sh`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q 'fresh-gate: PASS'`

## Guardrails

- Do NOT run the installer against the host in this spec. It is changed and tested on fixtures; the operator
  decides when it runs.
- Do NOT delete or `mv` a live store from the script. It refuses and prints the command.
- Do NOT weaken the card check to a warning.
- Do NOT touch `deploy/install-pii.sh`'s own card check except to call the shared function.

## Files

- `specs/338-the-installer-refuses-without-a-card.md`
- `deploy/config-python.sh`
- `deploy/install-pii-user.sh`
- `deploy/install-pii.sh`
- `deploy/tests/install-pii-user.test.sh`

# Spec: The stores that hold what pii learned can be brought back

## Overview

The daemon keeps what it learned in redb files under `~/.local/state/pii/` — `store.redb` (18.8 MB) and
`memory.redb` (12.6 MB) among them, plus the durable `channel.redb` and `timer.redb`. **There is no backup and no
restore drill**: measured 2026-11-02, `scripts/checks/*backup*` does not exist, no cron mentions one, and each of
those files has exactly one copy, on the machine that runs them.

That is the only unrecoverable failure in the deployment. A corrupt store, a bad upgrade, or a `--seed-stores`
someone runs by accident destroys weeks of learned state with no way back. Everything else about the deployment is
recoverable by re-deploying a binary.

**Project**: <repo>
**Phase**: production hardening — the top open item on the readiness list.

## Requirements

### Functional

1. **`scripts/checks/pii-backup.sh`** — copies every live store to `<root>/<UTC stamp>/`:
   `--source <dir>` (default `$PII_STATE_DIR` or `~/.local/state/pii`), `--root <dir>` (default
   `~/.local/state/pii/backups`). It copies into `<stamp>.partial/`, re-hashes every copy against its source, writes
   `manifest.tsv` (relative path, bytes, sha256) and only then renames the directory into place. A mismatch leaves
   the `.partial` directory behind and exits non-zero: a half-written backup must never look like a good one.
2. **It reports what it copied**: one line per store (`store.redb 19734528 3f2a…`), the total bytes, and the
   directory it landed in. "ok" with no numbers is not evidence.
3. **`scripts/checks/pii-restore-drill.sh`** — restores the NEWEST backup into a scratch directory and proves it:
   every file matches the manifest's sha256, and a daemon started with `PII_STATE_DIR=<scratch>` answers
   `pii health` with the same store names. It stops that daemon and exits 0 only if both hold. **It never touches
   the live stores** — it reads the backup, writes only under its scratch dir.
4. **Retention**: keep `--keep` days (default 14) and never delete the newest; a backup that is too old to keep is
   moved to `_trash/`, never removed.
5. **`--dry-run`** prints the plan (source, files, bytes, destination) and writes nothing.

### Non-Functional

- Shell only, no new Rust surface: the product stays one binary; this is an operator tool under `scripts/checks/`.
- No `rm`: retention moves. `rm -rf` is banned by AGENTS.md, including here.
- A missing source directory is a refusal (exit 2), not an empty backup.
- The stamp is UTC and sortable: `20261102T190000Z`.

## TDD Contract

| Test (`scripts/checks/tests/pii-backup.test.sh`) | Assertion |
|---|---|
| `backup_copies_every_store_with_a_matching_hash` | manifest rows == store count; each sha256 equals `sha256sum` of the source |
| `CONTROL_a_truncated_copy_is_refused` | a copy shortened before verification → rc≠0, `.partial` left, no stamped dir |
| `a_missing_source_is_a_refusal_not_an_empty_backup` | `--source /nonexistent` → rc 2, nothing created |
| `dry_run_writes_nothing` | rc 0, plan printed, root still absent |
| `the_drill_restores_and_verifies` | drill rc 0; scratch hashes match the manifest |
| `CONTROL_a_corrupt_backup_fails_the_drill` | one byte flipped → drill rc≠0 naming the file |
| `retention_moves_an_old_backup_it_never_deletes` | old stamp lands under `_trash/`, newest stays |

## Exit Criteria

- [ ] `bash scripts/checks/tests/pii-backup.test.sh`
- [ ] `bash scripts/checks/pii-backup.sh --dry-run --source /nonexistent 2>&1 | grep -q 'REFUSED\|refused'`
- [ ] `bash scripts/checks/gates.sh`

## Guardrails

- Do NOT write into the live state dir except under `backups/`.
- Do NOT compress or rewrite a store to "save space": a backup must be byte-identical to what was running.
- Do NOT restore over a live store from this script: the drill restores to a scratch dir, and an operator decides
  what to do with the result.
- Do NOT delete an old backup.

## Files

- `scripts/checks/pii-backup.sh`
- `scripts/checks/pii-restore-drill.sh`
- `scripts/checks/tests/pii-backup.test.sh`

## Closure

Implemented by the loop's worker and landed by the orchestrator, with the row's criteria measured here:

```
bash scripts/checks/tests/pii-backup.test.sh   -> PII-BACKUP: 7 passed, 0 failed   (both controls included)
bash scripts/checks/gates.sh                   -> 15 passed, 0 failed, 1 skipped · ALL GATES PASS
```

The judge refused this row once, and the fault was in this spec, not in the work: its second criterion was
`pii-backup.sh --dry-run` with the default source, and the lane exports a scratch `PII_STATE_DIR`, so the script
correctly REFUSED an empty source and the criterion failed. A criterion that only passes on a machine with live
stores is not a criterion. It now asserts the refusal itself (`--source /nonexistent`), which is true everywhere
and is also the behaviour worth pinning.

The live backup is still an operator command, and it says what it copied:
`bash scripts/checks/pii-backup.sh --source ~/.local/state/pii` -> `plan: 5 stores, 31461376 bytes`.

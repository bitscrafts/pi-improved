# Spec: The incident publisher lives in the repo, and a missing record is loud

## Overview

Two facts, one silence:

```
$ ls _tmp/autonomy/INCIDENTS.md
ls: cannot access '_tmp/autonomy/INCIDENTS.md': No such file or directory

$ sed -n '9p' ~/.hermes/scripts/pii-incidents-discord.sh
[ -f "$MD" ] || exit 0
```

The publisher exits **0** when its record is absent, so "the writer never ran" and "nothing happened" produce the
same non-event. Opus named it exactly: *that is the check that passes while the live path is broken.* The publisher
also lives outside the repository (`~/.hermes/scripts/`), so it is neither reviewed nor versioned, and a spec
cannot fix it.

**Project**: <repo>
**Phase**: production hardening (Opus audit, M1's false green)

## Requirements

**R1 — the publisher moves into the repository.** `scripts/autonomy/incidents-publish.sh` publishes new rows from
the record (`PII_INCIDENTS_MD`, default `_tmp/autonomy/INCIDENTS.md`) to the connected channels, keeping the cursor
(`PII_INCIDENTS_SENT`, default under `_tmp/autonomy/`). It prints **exactly** the rows it published and nothing
otherwise, so a cron with no output stays silent.

**R2 — absence is an error with a message.** Missing record → prints
`incidents-publish: the incident record is missing at <path> — the detector has never written one` and exits **2**.
A present-but-empty record → prints nothing, exits 0. A record with rows → prints each row once, exits 0.

**R3 — the cursor is the only reason a row is skipped.** Re-running publishes nothing twice; a **new** row is
published even when the file's other rows are old. Deleting the cursor republishes (that is the operator's repair,
not a bug).

**R4 — the Hermes job is pointed at the repo script, and the old file is retired by move.** A
`deploy/point-incidents-job.sh --print` prints the one-line change (the job's `script` value) with the exact command
to apply it; it does not edit `~/.hermes` by itself. The old script is `mv`-d to `_trash/`, never deleted.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `a missing record is loud` | `PII_INCIDENTS_MD=/nonexistent` | rc 2, message names the path, nothing published |
| `an empty record is silent` | a record with only the header | rc 0, no output |
| `a new row is published once` | two rows, no cursor | both printed; a second run prints neither |
| `control: the old script is silent` | `~/.hermes/scripts/pii-incidents-discord.sh` with the same missing path | rc 0, no output (the defect) |
| `the cursor survives a rewrite of the file` | the file rewritten with the same rows plus one | only the new row is printed |

## Exit Criteria

- [ ] `bash scripts/autonomy/tests/incidents-publish.test.sh 2>&1 | tail -1 | grep -qE 'RESULT: [5-9][0-9]* ok, 0 FAIL'`
- [ ] `bash -n scripts/autonomy/incidents-publish.sh`
- [ ] `bash -n deploy/point-incidents-job.sh`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q 'fresh-gate: PASS'`

## Guardrails

- Do NOT delete `~/.hermes/scripts/pii-incidents-discord.sh`: `mv` it to `_trash/` with a dated name and print the
  path.
- Do NOT post to a channel from the test: the test stops at "printed the rows it would publish".
- Do NOT invent incidents when the record is missing: absence is an error, not an empty result.
- Do NOT add a retry loop: one run, one verdict.

## Files

- `specs/340-the-incident-publisher-lives-in-the-repo.md`
- `scripts/autonomy/incidents-publish.sh`
- `scripts/autonomy/tests/incidents-publish.test.sh`
- `scripts/autonomy/tests/fixtures/publisher-before-340.sh`
- `deploy/point-incidents-job.sh`

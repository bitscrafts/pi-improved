# Spec: the project's own configuration is tracked
#: depends=436 status=pending block=-

## Overview

`.pii/` (260 K) holds this project's pii configuration and state. It is ignored, which means the settings that make the
queue, the gate and the lane behave the way they do on this repository **exist only on this machine**, in a directory
nobody reviews. A second installation starts from defaults and behaves differently; nobody can tell which difference was
a decision.

The rule this row applies is the one in `_docs/05-operations/disk-ceilings.md`: product configuration is TRACKED;
scratch is CATALOGUED; neither is invisible. `.pii/` is the first path in the table flagged as wrongly ignored.

## Requirements

- **R1** Inventory `.pii/` before changing anything: every file, its size, and whether it is configuration (a decision
  that should be reviewable), state (regenerated, or per-machine), or a secret (never tracked).
- **R2** Secrets are not trackable, and that includes credentials, tokens, local socket paths that leak a username, and
  anything matching the repository's existing secret scan. The row records which files were secret-shaped and what
  happened to them.
- **R3** Configuration files become tracked. State files either move to `.pii-sessions/` (already catalogued, per-machine)
  or are catalogued with a ceiling.
- **R4** `.gitignore` stops ignoring the tracked part and keeps ignoring the rest. The rule must be explicit per path,
  not a blanket `.pii/`, or the next file added there is invisible again.
- **R5** A fresh clone with the tracked configuration produces the same behaviour for the settings that were tracked:
  the daemon starts, the queue picks, and the gate floor is the one in the file. This is checked by reading the effective
  configuration, not by reading the file back.

## TDD contract

| Test | Input | Expected |
| --- | --- | --- |
| configuration is tracked | `git ls-files .pii/` on a clone | non-empty, and names the daemon/queue/gate settings |
| a secret is not tracked | the inventory of R1, plus the repository's secret scan over `git ls-files .pii/` | 0 hits |
| the ignore rule is explicit | `.gitignore` | no bare `.pii/` line; each remaining ignored path listed |
| behaviour comes from the file | change one tracked value in a scratch copy, start the daemon | the change is what takes effect |

The fourth row is the one that stops this becoming a file committed for show: a configuration that is tracked but not
read is worse than one that is ignored, because it looks like a decision.

## Exit Criteria

- [ ] `git ls-files .pii | grep -q .`
- [ ] `! git check-ignore -q .pii/config.toml`
- [ ] `bash scripts/checks/no-secrets-in-tracked.sh .pii`

## Files

- `.gitignore`
- `scripts/checks/no-secrets-in-tracked.sh`
- `.pii/config.toml`

### New in this row
- [ ] `git ls-files .pii | grep -q .`
- [ ] `! git check-ignore -q .pii/config.toml`
- [ ] `bash scripts/checks/no-secrets-in-tracked.sh .pii`

## Guardrails
- The `### New in this row` list is the same list as `## Exit Criteria`, and it exists only because
  `pii-add-row.py:13` reads that heading. The duplication is a defect in the intake tool (spec 439's class:
  a programme outside the repository imposing a format), not an intention of this spec.

- Do NOT track anything that fails the secret scan, and do NOT "sanitise" a file by editing a credential out of it: if a
  credential was ever in a file that gets committed, the credential is spent and must be rotated by the owner.
- Do NOT replace `.pii/` with a symlink to a per-machine directory. That satisfies the letter of "tracked" while leaving
  the configuration outside review.
- Do NOT edit `/root/.pi` or any per-machine agent configuration: this row is about the project's own directory.

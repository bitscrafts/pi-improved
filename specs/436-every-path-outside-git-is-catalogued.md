# Spec: every path outside git is catalogued, or it is a defect
#: depends=- status=pending block=-

## Overview

`git status` on this tree is clean — 0 untracked, non-ignored files — and 193 GB sat behind 20 `.gitignore` rules with
nothing saying what they were, whether they could be regenerated, or what stopped them growing. **Clean is not the same
as accounted for.**

`_docs/05-operations/disk-ceilings.md` (filed 2026-11-20) answers it with three states — TRACKED, CATALOGUED, DEFECT —
and a table giving every catalogued path its size, what it is, how it is recreated, its ceiling and its importance.

A table with no enforcement is decoration. This row is the enforcement.

## Requirements

- **R1** `scripts/checks/tree-is-catalogued.sh` lists every ignored path (`git status --porcelain --ignored`, plus
  `git ls-files --others --ignored --exclude-standard`) and compares the set with the paths named in the catalogue table.
- **R2** An ignored path that is **not** in the table fails the check, naming the path and saying how to fix it (add a
  row to the table, or delete the path). This is the whole point: the defect state must be impossible to occupy.
- **R3** A catalogued path that exists and is over its ceiling fails the check, naming the path, its size and its ceiling.
  A ceiling nobody measures is a wish.
- **R4** A catalogued path that does not exist is not a failure (a clean checkout has no `_tmp/`).
- **R5** Matching is by path *prefix*, not exact string: `_tmp/gate-target/build-gate` belongs to the
  `_tmp/gate-target/` row. The table is the source of truth for the prefix, so the script parses the table rather than a
  second list.
- **R6** The check runs in the gate: one line in `gate-steps.txt`. It must not be a `--routine`-only step, because a
  path that appears while nobody watches is how the 193 GB grew.

## TDD contract

| Test | Input | Expected |
| --- | --- | --- |
| an uncatalogued ignored path | fixture tree, `.gitignore` names `junk/`, `junk/` exists, table does not name it | exit 1, output contains `junk/` and `not catalogued` |
| a catalogued path over its ceiling | fixture with `_tmp/_trash/` at 6 GB and a 5 G ceiling | exit 1, output names both numbers |
| the real tree | this repository after the 2026-11-20 clean | exit 0 |
| a catalogued path that is absent | fixture without `_tmp/` | exit 0 |

The third row is the property; the first two are the near-misses that a script which merely *prints* the paths (without
comparing) would fail to catch.

## Exit Criteria

- [ ] `bash scripts/checks/tree-is-catalogued.sh`
- [ ] `grep -qx 'tree-is-catalogued' gate-steps.txt`
- [ ] `bash scripts/checks/tree-is-catalogued.sh --selftest`

## Files

- `scripts/checks/tree-is-catalogued.sh`
- `gate-steps.txt`
- `_docs/05-operations/disk-ceilings.md`

### New in this row
- [ ] `bash scripts/checks/tree-is-catalogued.sh`
- [ ] `grep -qx 'tree-is-catalogued' gate-steps.txt`
- [ ] `bash scripts/checks/tree-is-catalogued.sh --selftest`

## Guardrails
- The `### New in this row` list is the same list as `## Exit Criteria`, and it exists only because
  `pii-add-row.py:13` reads that heading. The duplication is a defect in the intake tool (spec 439's class:
  a programme outside the repository imposing a format), not an intention of this spec.

- Do NOT add an exemption list to the script. An exemption is a catalogued path with no ceiling, which is the state this
  row exists to remove; the exemption mechanism is the table.
- Do NOT make the ceiling check advisory. A warning in a log nobody reads is how this document was needed.
- Do NOT parse `.gitignore` to decide what is "real". The script asks git, so nested and inherited ignore files are
  handled by the tool that owns the semantics.

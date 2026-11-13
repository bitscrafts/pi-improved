# Spec: <one behaviour, in a sentence, no number here>
#: depends=- status=pending block=-

<!--
How to use this file (spec 421). Copy it to specs/<N>-<slug>.md, where <N> is the next free number; the dispatcher
resolves `specs/<N>-*.md` and would pick THIS file up if it carried a number, so it must not.

The card line above is read by `scripts/autonomy/sync-verify.sh` (the row's verify column must equal ## Exit Criteria,
command for command) and by `pick` (a row whose `depends=` is unfinished is not offered, spec 420).
  depends=  the row that must be done first, or `-`
  status=   left as pending; the queue owns it after this file is queued
  block=    a row that must NOT land before this one, or `-`

Two rules that decide whether a row can land:
  * a criterion must FAIL before the work and pass after it. One that already passes belongs under Guardrails.
  * `## Files` must equal the row's paths column, file for file: `scripts/autonomy/leftover.sh owner` refuses a tree
    whose dirty paths are not all named by the row.
-->

## Overview
What is wrong or missing today, with the measurement that shows it: the command, the error, the date. Name the files and
line numbers you read to find it. Say what this row changes, and name the alternatives you rejected and why.

**Project**: <repo>
**Phase**: <the phase this belongs to>
**Depends on**: <row, or "nothing">

## Requirements
- R1 <a requirement a test can check: one behaviour, no "and also">
- R2 <...>

## Architecture
```
<the shape of the change: files, functions, the flow before and after>
```
Key decision: <the choice, and why the rejected alternatives lost>

## TDD Contract
<where the tests live and how they are driven — the real path, never a re-enactment of a neighbouring script>
| Test Name | Input | Expected Output |
|---|---|---|
| `<name>` | <input> | <what must be observed> |
| `CONTROL_<name>` | <the near-miss> | <it must still take the normal path> |

Acceptance: <which tests must fail before the work, and which mutations must turn one red>

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `<one shell command that exits 0 when the work is done>`

## Guardrails
- Keep green, unedited: `<the tests and checks this row must not disturb>`
- Do NOT <the tempting shortcut this row forbids>
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `<one path, exactly as `git status` prints it>`

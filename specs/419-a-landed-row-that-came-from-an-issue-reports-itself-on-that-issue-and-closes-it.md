# Spec: a landed row that came from an issue reports itself on that issue and closes it
#: depends=418 status=pending block=-

## Overview
The round trip is open at both ends. A `ready` issue reaches the channel (spec 406), is acknowledged and drafted into a spec (specs 413/414/415), and the queue lands
the row — but nothing ever tells the issue that its row landed, and nothing closes it. Measured 2026-11-13 on `bitscrafts/pi-improved`: issues #1–#21 were all
closed by hand by the end-to-end scripts, never by pii.

The missing piece is deliberately NOT in the queue: `cycle.sh` must not reach the network, and a landing must not fail because GitHub is slow. It is a command
the orchestrator runs after a landing, the same way `scripts/mark-ready.sh` is the command it runs before a dispatch:

```
scripts/report-row.sh <row> --issue <N> [owner/repo]
```

It reads the row's state (`scripts/autonomy/state.py get`), comments one fixed, machine-written sentence with the commit and the status, and closes the issue
only when the row is `done`. Its guarantee is the one `mark-ready.sh` learned in spec 417: the claim is what the read-back says.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 418

## Requirements
- R1 `scripts/report-row.sh <row> --issue <N> [owner/repo]` comments on issue `N`: a fixed, single-line sentence naming the row, its status and — when the row
  is done — its commit, plus the spec file. Nothing from the issue's own text and nothing from a worker's output is echoed.
- R2 The issue is closed only when the row's status is `done`, with reason `completed`; a row that is `pending`, `needs-review` or `parked` gets the comment and
  stays open.
- R3 The comment is posted at most once per (row, status, commit): a second run with the same values posts nothing and says so, so a re-run after a failed
  network call is safe. A different commit or status does post again.
- R4 The claim is the read-back: after commenting, the script reads the issue's comments back and exits non-zero unless its own comment is there; after closing,
  it reads the state back and exits non-zero unless the issue is `CLOSED`.
- R5 Every `gh` call is one a fake can serve offline: `issue view --json state,comments`, `issue comment`, `issue close`. The script reads no other field, needs
  no new environment variable, and prints a `REFUSE:` line with exit 3 when the row is unknown.

## Architecture
```
state.py get <row>  ->  status, commit
  comment text = "pii row <row> <status> at <commit>: <spec>"      (fixed shape)
  gh issue view N --json comments  ->  already there?  -> say so, exit 0
  gh issue comment N --body <text>
  gh issue view N --json comments  ->  is it there?   no -> exit 5
  status == done? -> gh issue close N --reason completed -> read state back, must be CLOSED, else exit 6
```
Key decision: a command after the landing, not a step inside it — the queue stays offline, and a landing cannot be blocked by GitHub. Rejected: the channel
posting it (the channel has no notion of a row's later life). Rejected: reading the issue number from the spec's card line (a format change for one
consumer).

## TDD Contract
New `scripts/checks/report-row-comments-and-closes.sh`, driven by a fake `gh` first on PATH (the spec 417 pattern) and a fake `state.py` reader.
| Test Name | Input | Expected Output |
|---|---|---|
| `a_done_row_comments_and_closes` | row done at a commit | one comment naming row/status/commit; issue CLOSED with reason completed |
| `CONTROL_a_pending_row_comments_and_stays_open` | row pending | one comment; no `issue close` call; issue still OPEN |
| `CONTROL_a_second_run_with_the_same_values_posts_nothing` | run twice | exactly one comment; the second run says it is already reported |
| `CONTROL_a_different_commit_posts_again` | same row, new commit | a second comment |
| `CONTROL_a_read_back_without_the_comment_fails` | the fake accepts the comment but the read-back omits it | exit 5, and no close |
| `CONTROL_an_unknown_row_is_refused` | a row with no state | exit 3, and no comment |

Acceptance: the six pass; each of these mutations makes at least one fail (the mutation script is the orchestrator's): the "already reported" check removed;
the close gated on `done` but never verified; the read-back check removed.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `bash scripts/checks/report-row-comments-and-closes.sh`

## Guardrails
- Keep green, unedited: `bash scripts/checks/mark-ready-sets-the-label.sh` and `bash scripts/autonomy/run-tests.sh`.
- The script must not comment or close anything when the row is unknown, and must not close an issue for a row that is not done.
- Do NOT call the real GitHub in the test; the fake `gh` is the only `gh` the test may reach, and it must be first on `PATH`.
- Do NOT touch the queue's landing path, and do NOT make any cycle step reach the network.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `scripts/report-row.sh`
- `scripts/checks/report-row-comments-and-closes.sh`
- `gate-steps.txt`

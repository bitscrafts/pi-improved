# Spec: a row claims the tree it leaves work in
#: depends=430 status=pending block=-

## Overview
Finding F6 of the Opus analysis (`_projects/_tmp/opus-gh-graphs/report.md`, session `0c032ae3`): five queue
failures measured on 2026-11-13/15 are **one defect**. `session` is used as proof that a row owns the working tree.

- `leftover.sh:75` drops a row without a session, so it can never own its own work.
- The cycle writes `session=${NEWSID:-}`, which **clobbers** an existing session with `""` exactly when the lane did
  not start (`cycle.sh:362,351,366,378`).
- `cycle.sh:174` then treats the hand work as a leftover and stashes it (`leftover.sh:152`) — measured at 18:26:51Z and
  18:27:22Z, verify ran on an empty tree.
- Otherwise `owner` reports FOREIGN, and the cycle stops at exit 7 (rows 414, 416, 417, 418, 429).
- My patch — putting the session inside the `OWNER` token — broke every caller that parses `OWNER <row-id>`
  (`cycle.sh:197`), which is the same mistake at a different layer: the proxy was patched, not the contract.

## Requirements
R1. `state.py set <row> tree=claimed` is the single ownership record. It is written by the step that leaves work in the
    tree: `cycle.sh` when it sets `worker-ok`, `resume-row-at-verify.sh` before it runs the cycle, and a person running
    `state.py claim <row>`.
R2. `leftover.sh owner` reads `tree=claimed`, not `session`. A row with the claim owns exactly the paths it names, and
    the verdict token stays `OWNER <row-id>` — unchanged, because its contract is parsed.
R3. The cycle's dirty-tree branch is skipped when the phase marker says resume-at-verify **and** the row holds the
    claim. No stash is created for a claimed row.
R4. No code writes an empty `session`; an unknown id must leave the field absent (`${NEWSID:+session=$NEWSID}`).
R5. A dirty tree with **no** claimed row still says FOREIGN and still stops the cycle (near-miss control: the guard is
    not weakened to make the claimed case work).
R6. The claim is released when the row reaches `done` or `skipped`.

## TDD Contract
| Test Name | Input | Expected Output |
| --- | --- | --- |
| a_claimed_row_owns_its_paths_without_a_session | row with `tree=claimed`, no session, two dirty paths it names | `OWNER <id>` |
| control_an_unclaimed_row_still_cannot_own | same row, claim removed | FOREIGN, exit non-zero |
| control_an_unclaimed_dirty_tree_still_stops_the_cycle | dirty tree, no claimed row | exit 7 |
| a_claimed_row_is_never_stashed | claimed row, dirty tree, resume-at-verify | `git stash list` unchanged |
| the_verdict_token_is_still_a_bare_row_id | any outcome | first token after `OWNER ` parses as the row id |
| an_unknown_lane_id_does_not_blank_the_session | lane did not start | `session` keeps its previous value |
| the_claim_is_released_on_done | row reaches done | no `tree=claimed` |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/tree-claim.test.sh`

### New in this row
- [ ] `bash scripts/autonomy/tests/tree-claim.test.sh`

## Files
- `scripts/autonomy/leftover.sh`
- `scripts/autonomy/state.py`
- `scripts/autonomy/cycle.sh`
- `scripts/resume-row-at-verify.sh`

## Guardrails
- Do NOT change the `OWNER <row-id>` token shape.
- Do NOT delete or weaken the FOREIGN path: an unclaimed dirty tree must still stop the cycle.
- Do NOT use `git checkout --`; a stash is the only recovery, and it must stop being created for claimed rows.

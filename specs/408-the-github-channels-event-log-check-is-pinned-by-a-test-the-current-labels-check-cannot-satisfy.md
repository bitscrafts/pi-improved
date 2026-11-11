# Spec: the github channel's event-log check is pinned by a test that the current-labels check cannot satisfy
#: depends=406 status=pending block=-

## Overview
Spec 406 (landed `11caecf`) says an issue reaches the queue only when its `ready` label was applied by an allowed login, read from the issue's event log (R3), AND `ready`
is on the issue now (R4). A mutation test run after the row landed (2026-11-11) changed the event-log check so that an `unlabeled` event counts as ready: all 16 tests
still passed. The current-labels check hid it, because a removed label is also missing from the issue's labels. That is the same hole a real bug would hide in: if the
current-labels check is ever relaxed, nothing pins the event-log check.

This row adds the one test that separates the two checks, and nothing else.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 406

## Requirements
- R1 A test where the issue's current labels STILL hold `ready` while its latest `ready` event is an `unlabeled`: the issue is not returned and `dropped` counts it.
- R2 A test where the issue's current labels hold `ready`, the first `ready` event is a `labeled` by the owner and the latest is a `labeled` by a stranger: not returned.
  (This one exists in a weaker form; the new row makes the labels the same in both, so only the event log can tell them apart.)
- R3 No change to `crates/pii-channels/src/github.rs` or any other source file. The tests must pass against the code as it is.

## Architecture
```
tests/github.rs  +2 tests, same fake transport:  labels say ready  AND  event log says it was removed / re-applied by a stranger
```
Key decisions:
- Tests only. Rejected: tightening the code. The code is right; what is missing is the proof that two checks are both needed.
- The labels in the fixture are deliberately inconsistent with the event log. That state is rare on a real server and impossible to reach through the other checks, which is why it is the
  only state that distinguishes them.

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `an_issue_still_labelled_ready_whose_latest_event_removed_it_is_not_returned` | labels hold `ready`; events: `labeled` by owner, then `unlabeled` | no `Incoming`; `dropped == 1` |
| `an_issue_still_labelled_ready_whose_latest_event_is_a_strangers_is_not_returned` | labels hold `ready`; events: `labeled` owner, `unlabeled`, `labeled` stranger | no `Incoming`; `dropped == 1` |

Acceptance: both tests pass on the current code, and each is verified to FAIL when the event-log check is mutated (the mutation script is the orchestrator's, in `_tmp`).

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-channels --test github an_issue_still_labelled_ready 2>&1 | grep -qE 'test result: ok. [2-9] passed'`

## Guardrails
- Keep green: `cargo test -q -p pii-channels --test github`, `cargo test -q -p pii-channels --test adapters`.
- Do NOT edit any file under `crates/pii-channels/src/`.
- Do NOT edit an existing test; add two.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-channels/tests/github.rs`
- `crates/pii-channels/tests/github/main.rs`
- `crates/pii-channels/tests/github/event_log.rs`

# Spec: an issue is delivered once per `ready` label, not once per update
#: depends=408 status=pending block=-

## Overview
Measured 2026-11-12, the first real run of the `github` channel (spec 406) against `bitscrafts/pi-improved`: the owner labelled one issue `ready`; the channel started a run
that posted ONE comment; 20 seconds later the issue had THREE comments from three runs, and it would have gone on until the issue was closed. Cause, read in
`crates/pii-channels/src/github.rs` (`poll`): an issue is skipped only when its `updated_at` is not newer than the cursor, and posting a comment moves `updated_at`. The channel's
own reply therefore makes the issue look new, and it answers itself. Any row that writes back to an issue (comment, close) would hit the same loop.

The thing the channel consumes is the owner's act of applying `ready`, not the issue's last change. This row makes the cursor follow that act.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 408

## Requirements
- R1 An `Incoming`'s `cursor` is the time of the latest `labeled` event for `ready` by an allowed login (`created_at` in epoch seconds), read from the issue's event log. It is no longer
  the issue's `updated_at`.
- R2 An issue whose `updated_at` is not newer than the cursor is skipped without reading its events (nothing happened on it). An issue whose `updated_at` is newer is read: its events
  are fetched and, if the label time is not newer than the cursor, it is skipped silently. A skipped issue is NOT counted in `dropped`; `dropped` keeps meaning "labelled by someone not allowed".
- R3 A comment on a delivered issue does not deliver it again. The owner removing `ready` and applying it again does: the new `labeled` event is newer than the cursor.
- R4 An event with no `created_at` falls back to the issue's `updated_at` for that issue (the previous behaviour). GitHub always sends `created_at`; the fallback keeps every landed fixture valid.
- R5 `poll` returns its `Incoming`s oldest label first, so the router, which advances the cursor to each message in turn, never moves it backwards.
- R6 The trust gate (R3/R4 of spec 406) is unchanged: current labels first, event log second, only an allowed actor's `labeled` counts.
- Known limit, documented and not fixed here: the cursor has one-second resolution. An issue labelled in the same second as the last delivered one, in a later poll, is read again only
  when that issue next changes.

## Architecture
```
poll:  list ready issues -> updated_at <= cursor ? skip (no events call)
                         -> events -> latest `ready` event labeled by an allowed login? no -> dropped
                                   -> label time <= cursor ? skip silently
                                   -> Incoming{ cursor = label time }
       sort by cursor ascending
```
Key decisions:
- The cursor follows the owner's act. Rejected: remembering the comment ids the channel posted (state to keep, and a person's own comment would still re-deliver). Rejected:
  filtering out the bot's own comments (the channel does not know which login is itself with a user token).
- One extra events call per issue that changed since the last poll. Issues that did not change cost nothing.

## TDD Contract
New tests, in `crates/pii-channels/tests/github/cursor.rs` (a new file: `main.rs` is at 445 of 450 lines). Their fixtures carry `created_at`; the landed tests are not edited.
| Test Name | Input | Expected Output |
|---|---|---|
| `cursor::a_comment_on_a_delivered_issue_does_not_deliver_it_again` | labeled by owner at T1; `updated_at` T2 > T1 (the bot's comment); poll(cursor = T1) | no `Incoming`; `dropped == 0` |
| `cursor::CONTROL_the_owner_labelling_ready_again_delivers_it_again` | labeled T1, unlabeled T2, labeled T3 by owner; `updated_at` T3; poll(cursor = T1) | one `Incoming`, `cursor == T3` |
| `cursor::the_cursor_is_the_label_time_not_updated_at` | labeled T1; `updated_at` T2; poll(None) | one `Incoming`, `cursor == T1` in epoch seconds |
| `cursor::an_issue_not_updated_since_the_cursor_costs_no_events_call` | `updated_at` <= cursor | no `Incoming`; the fake saw no `/events` request for it |
| `cursor::issues_are_returned_oldest_label_first` | two issues, labelled T1 < T2, listed newest first | `Incoming` cursors `[T1, T2]` |
| `cursor::an_event_without_created_at_falls_back_to_updated_at` | labeled with no `created_at`; `updated_at` T2 | one `Incoming`, `cursor == T2` |

Acceptance: the six pass on the new code; each of these mutations makes at least one of them fail (the mutation script is the orchestrator's, in `_tmp`): cursor back to `updated_at`;
the `updated_at <= cursor` skip removed; the label-time skip removed; the sort removed; the fallback removed.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-channels --test github cursor:: 2>&1 | grep -qE 'test result: ok. [6-9] passed'`

## Guardrails
- Keep green: `cargo test -q -p pii-channels --test github` (the 18 landed tests, unedited), `cargo test -q -p pii-channels --test adapters`.
- Do NOT edit an existing test. `main.rs` gains one line, `mod cursor;`, and nothing else.
- Do NOT make a write to GitHub other than `send`; do NOT add a label, close an issue, or log an issue's text.
- `crates/pii-channels/src/github.rs` is 393 lines; if the change takes it past 450, make `github/` a folder module in the same row.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-channels/src/github.rs`
- `crates/pii-channels/tests/github/cursor.rs`
- `crates/pii-channels/tests/github/main.rs`

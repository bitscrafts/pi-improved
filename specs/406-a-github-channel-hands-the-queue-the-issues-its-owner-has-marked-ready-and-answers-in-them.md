# Spec: a GitHub channel hands the queue the issues its owner has marked ready, and answers in them
#: depends=401 status=pending block=-

## Overview
Pii's specs and issues for a public repository live on GitHub, and the queue should be able to work an issue the maintainer has read and
approved, then report back in the same issue. Pii has channels for Telegram, Matrix, a webhook, MQTT and Discord, but none for GitHub: the only
bridge today is a person running `scripts/mark-ready.sh` and copying an issue into a spec by hand.

A repository's issue tracker is open to anyone. The one rule that makes a GitHub channel safe is the one the Discord channel already has (spec 401): text
from a stranger is never an instruction. The Discord channel drops every author who is not on an allow list, defaulting to the guild owner, and counts
what it dropped. This channel has the same rule with a stronger form: an issue reaches the queue only when the OWNER has labelled it `ready`.

Measured 2026-11-10: `crates/pii-channels` has the `Channel` trait (`lib.rs:76`), a reference adapter to copy (`telegram.rs`, with `writing-a-channel.md`), a
credential resolver (`secret.rs`: an environment variable or a 0600 file, never a config value) and a transport that is injected so a fake can stand in
for the network (`HttpTransport`). `build_raw_channel` in `crates/pii-server/src/daemon/channel_wiring.rs` maps `kind` to an adapter.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 401 (the trust rule this reuses)

## Requirements
- R1 A new channel kind `github` in `crates/pii-channels/src/github.rs` (a folder module `github/` if it passes 450 lines), registered in
  `build_raw_channel`. Configuration: `repo = "owner/name"`, optional `base_url` (default `https://api.github.com`), the token from `token_env`
  (default `PII_GITHUB_TOKEN`) or `token_file` through `secret::resolve`. A token is never in the config, a log line, an error or a `health_issue`.
- R2 `poll(ctx, cursor)` lists the repository's open issues that carry the label `ready` and returns one `Incoming` for each that is new since `cursor`
  (`chat` = the issue number as text, `user` = the issue author's login, `text` = the issue title, a blank line, then the body, `cursor` = the issue's
  `updated_at` as a monotonic integer). It is paged until the last page; a page that cannot be read is `PiiError::Unavailable` (retried next tick, never fatal).
- R3 The trust gate. An issue is returned only if the `ready` label was applied by the repository owner (the `owner` of `repo`, or a login listed in
  `PII_GITHUB_ALLOW_USERS`, which replaces the default as the Discord list does). It is decided from the issue's own event log
  (`GET /repos/{repo}/issues/{n}/events`: the latest `labeled` event whose `label.name` is `ready`, and its `actor.login`), never from who wrote the issue and never from
  the issue text. An issue whose `ready` label was applied by anyone else, or that has no `labeled` event for it, is dropped and counted in `ChannelCounters::dropped`;
  its text is never logged.
- R4 A label applied and then removed is not ready. The gate reads the issue's CURRENT labels first and the event log second: `ready` must be present now
  AND the latest `ready` event must be a `labeled` by an allowed actor (a later `unlabeled` or a later `labeled` by a stranger fails it).
- R5 `send(ctx, to, text)` posts `text` as a comment on issue `to` (`POST /repos/{repo}/issues/{to}/comments`). `to` must be the decimal number of an issue; anything
  else is `PiiError::Invalid` and nothing is sent. A comment is capped at 60,000 bytes (longer text is cut with a marker, never refused silently).
- R6 `egress_hosts` is the host of `base_url` and nothing else (no redirect host is granted). A 401 or 403 sets `health_issue` to `api rejected the token (status N)`
  and returns the named auth error, exactly as the Telegram adapter does; a rate-limit 403/429 with `retry-after` or `x-ratelimit-reset` is `Unavailable`, not a token rejection.
- R7 The channel never writes to GitHub except `send`: no label is added or removed, no issue is closed, no review is requested. (Marking an issue
  `ready` stays the owner's act, `scripts/mark-ready.sh`.)

## Architecture
```
GitHub REST  <--HttpTransport (injected)--  GithubChannel { repo, base, token, allow }
   poll:  GET issues?labels=ready&state=open  -> for each: GET issues/N/events -> latest `ready` event -> actor in allow?
             yes -> Incoming        no -> dropped += 1 (text never logged)
   send:  POST issues/N/comments
build_raw_channel: "github" => adapters::github(c, transport, env)
```
Key decisions:
- The gate reads the event log, not the issue author. Rejected: "the issue's author is the owner" (the owner cannot approve a stranger's issue that way, which is
  the case the label exists for); and "the label is present" (any triager or a bot with write access, or a label left over from an earlier state, would pass).
- One allow list, the same shape as Discord's (`..._ALLOW_USERS` replaces the default). Rejected: a second trust model for one channel; two rules is how one of them drifts.
- Read-only toward the repository except comments. Rejected: letting the channel move labels itself (an `in-progress` label); that is the queue's act, not the transport's, and
  a transport that can write labels can be talked into approving an issue.
- Issue text is carried as data to a spec author, never executed or interpolated into a shell, a path or a prompt template here; this spec stops at the `Incoming`.

## TDD Contract
New test file `crates/pii-channels/tests/github.rs`, using the same fake `HttpTransport` as `tests/adapters.rs`.

| Test Name | Input | Expected Output |
|---|---|---|
| `a_ready_issue_labelled_by_the_owner_is_returned` | one open issue labelled `ready`; its event log: `labeled ready` by the owner | one `Incoming` with chat = the number, user = author, text = title + body |
| `CONTROL_an_issue_labelled_by_a_stranger_is_dropped_and_counted` | same issue, the `labeled` actor is not the owner | no `Incoming`; `dropped == 1`; no log line holds the issue text |
| `an_issue_whose_ready_label_was_removed_is_not_returned` | event log: `labeled` by the owner, then `unlabeled`; current labels no longer hold `ready` | no `Incoming` |
| `a_stale_owner_label_does_not_cover_a_later_stranger_label` | events: `labeled` by owner, `unlabeled`, `labeled` by a stranger | dropped |
| `CONTROL_an_issue_written_by_the_owner_without_the_label_is_not_returned` | an open issue, no `ready` label | no `Incoming`, and no events request is made |
| `the_allow_list_env_replaces_the_default` | `PII_GITHUB_ALLOW_USERS=triager` | a label by `triager` passes, the same label by the owner is dropped |
| `poll_reads_every_page` | two pages of issues | issues from both pages are considered |
| `poll_returns_only_issues_newer_than_the_cursor` | a cursor between two `updated_at` values | only the newer one |
| `a_bad_page_is_unavailable_not_fatal` | a 500 on page 2 | `PiiError::Unavailable` |
| `send_posts_a_comment_on_that_issue` | `send("7", "done")` | one POST to `/issues/7/comments` with the text |
| `CONTROL_send_to_a_non_number_is_refused_and_sends_nothing` | `send("7/../8", "x")`, `send("", "x")` | `PiiError::Invalid`; the fake saw no request |
| `a_401_degrades_health_and_hides_the_token` | the fake answers 401 | `health_issue()` names the status; neither it nor the error holds the token |
| `a_rate_limit_is_not_a_token_rejection` | 403 with `x-ratelimit-remaining: 0` | `Unavailable`; `health_issue()` stays `None` |
| `egress_is_the_api_host_only` | default config | `["api.github.com:443"]` |
| `a_missing_token_is_degraded_not_a_refusal_to_start` | no env, no file | `build` succeeds; `health_issue()` is `token missing` |
| `the_channel_never_writes_a_label` | a full poll and send | the fake saw no request to a labels URL and none with a method other than GET (issues, events) or POST (comments) |

Acceptance test: `CONTROL_an_issue_labelled_by_a_stranger_is_dropped_and_counted`. The dangerous requirement is R3: a plausible implementation checks the label
and the issue author and passes the positive row, and only the stranger-label row fails it. A second plausible implementation reads the events but takes the FIRST
`ready` event instead of the latest, which `a_stale_owner_label_does_not_cover_a_later_stranger_label` catches.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-channels --test github 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `grep -q '"github" =>' crates/pii-server/src/daemon/channel_wiring.rs`

## Guardrails
- Keep green (they pass today): `cargo test -q -p pii-channels --test adapters`, `cargo test -q -p pii-channels --test discord`, `cargo run -q -p xtask -- net-check`.
- Do NOT add a dependency to `pii-channels`, and `pii-net` must not appear in its dependency graph (it goes through `HttpTransport`).
- Do NOT read a token from the config file or print one anywhere; the only sources are `PII_GITHUB_TOKEN` (or the name in `token_env`) and a 0600 `token_file`.
- Do NOT add a write to GitHub other than the comment POST. No label, close, assign or review call exists in the module.
- Do NOT start a network connection from a test; every test uses the fake transport.
- A file over 450 lines does not get longer; split into `github/` (`poll`, `trust`, `send`) before it passes the limit.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-channels/src/lib.rs`
- `crates/pii-channels/src/github.rs`
- `crates/pii-channels/tests/github.rs`
- `crates/pii-server/src/daemon/channel_wiring.rs`
- `crates/pii-server/src/daemon/adapters.rs`
- `_docs/02-guides/writing-a-channel.md`

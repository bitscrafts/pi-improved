# Spec: 195 — a refused post is paced by the retry_after it carries

**Project**: /projects/pi-improved · **Phase**: the exercise matrix's E6, outbound half of the 429 class
**Filed**: 2026-10-19 by the orchestrator, from `EXERCISE-RESULTS-2026-10-19.md` (E6 row) and 070 §0.1.

## Overview

MEASURED 2026-10-18 09:00:03–09:02:49Z: **1,836 `channel.post_refused` in 166 seconds**, every one carrying
Discord's own answer in its text — `discord: forum post status 429: {"message":"You are being rate
limited.","retry_after":132.176,"global":false}`. The retry_after is IN the error; nothing reads it. The
post path re-dialled as fast as runs were started, which is how one 09:00 occurrence produced 1,836
refusals and 10 posts (070 §0.1: 10 processes × one armed cron each).

Read in the tree today:

- `crates/pii-channels/src/discord.rs` `refusal()` (208-216) keeps the body (bounded, scrubbed) — that is
  why the number is measurable at all. `send_message` and `send_forum_post` call it and return the error.
- `crates/pii-channels/src/post.rs::ChannelHub::post` dials immediately; on a definite failure it releases
  the idempotence key and returns. There is no per-channel state between calls, so two calls 1 ms apart
  dial twice.
- `crates/pii-server/src/daemon/router.rs:171-173` `reply()` retries blindly ONCE (`if c.send(...).is_err()
  { let _ = c.send(...); }`) — a second dial into the same closed window, on the daemon's reactive loop.

## Requirements

- **R1 — the hub paces each channel.** `ChannelHub` keeps, per channel name, a `paced_until: Option<Instant>`
  set from `retry_after_ms(e)` when a send returns an error carrying it (the reader spec 194 adds to
  `pii_channels::lib`; if 194 has not landed, add it here — same contract, same tests). Cap the pace at
  600 s; a missing or unparseable token means no pace.
- **R2 — inside the window, nothing dials.** A `post()` on a paced channel returns
  `Err(Unavailable("channel <name>: paced by the provider for <n> ms"))` WITHOUT calling `send_tagged`
  (the fake transport's POST count is the evidence), and releases the idempotence key so the later attempt
  can post. A `PostCounts.refused` is counted (`note_refused`), as today.
- **R3 — after the window, the post lands.** At `paced_until`, the next `post()` dials and delivers exactly
  once. This is E6's second half: "a post still lands".
- **R4 — `reply()` stops double-dialling a rate limit.** Its second attempt happens only when the first
  error carries no `retry_after` (today's behaviour for every other failure, kept as the control). A paced
  refusal logs one `channel.send.failed` (the `ObservedChannel` already emits it) and returns.
- **R5 — controls.** (1) An error WITHOUT `retry_after` creates no pace: the next `post()` dials
  immediately (today's behaviour preserved). (2) A successful send clears any pace. (3) The pace is per
  channel: a second channel's first post is unaffected by the first channel's 429. (4) The paced error
  names the channel and the milliseconds, and contains no secret (the token is not in the text).

## TDD Contract

| Test (`crates/pii-channels/tests/send_pacing.rs`) | Input | Expected |
|---|---|---|
| `a_paced_channel_is_dialled_once` | fake 429 with `retry_after=1s` on every POST; 10 `post()` calls in 200 ms | exactly **1** dial, 0 delivered, 10 refusals |
| `a_post_lands_after_the_window` | same fake, then a success after 1.1 s | 1 dial in the window + 1 after; delivered == 1 |
| `an_error_without_retry_after_does_not_pace` (control) | a 500 with no token | two consecutive posts dial twice, second delivers |
| `a_success_clears_the_pace` (control) | 429 (1 s), success, then a second post | the second dials (pace cleared by the success it observed) |
| `the_pace_is_per_channel` (control) | channel A paced; channel B's post | B dials immediately |

## Exit Criteria

- [ ] `cargo test -p pii-channels --test send_pacing 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-channels --test send_pacing a_paced_channel_is_dialled_once 2>&1 | grep -q '1 passed'`
- [ ] `cargo test -p pii-channels 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `grep -q 'paced_until' crates/pii-channels/src/post.rs`

## Guardrails

- **Do NOT sleep on the caller's thread.** The pace is a state ("not before T"), checked by the next call;
  a `post` inside the window returns at once. Blocking would stall the daemon's reactive loop (`reply()` is
  called from it) — measured as the reason the naive "sleep then retry" shape is wrong here.
- **Do NOT change `PostRequest` or the `post` node's config surface.** The pace is the hub's, not a node
  option: every caller (post node, `reply`, tests) gets it by construction.
- **Do NOT keep the pace forever.** Cap 600 s, and a success clears it — otherwise a stale 429 parks a
  channel silently, which is worse than the storm.
- **450-line file gate**: `post.rs` is 432 lines today. Put the pacer in a submodule
  (`crates/pii-channels/src/post/pace.rs`, `mod pace;` from `post.rs`) rather than growing the file past the
  gate.
- **A green `cargo test` is not a green gate**: `bash quality-gate.sh --routine` must pass on the tree this
  leaves.
- **The tests drive the production path** (`ChannelHub::post` with a fake `Channel`), not a re-statement of
  the pacing arithmetic.
- **Mutation controls to record in the Closure**: (a) delete the `paced_until` check → `a_paced_channel_is
  _dialled_once` reddens; (b) never clear the pace on success → `a_success_clears_the_pace` reddens;
  (c) drop the token from the refusal text (the pre-fix shape) → every pacing test reddens.

## Closure

(To be filled by the orchestrator after independent verification: the criteria's own output, the mutation's
red output, the byte-identical restore, and what this does and does not close in the E6 row.)
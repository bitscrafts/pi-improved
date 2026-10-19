# Spec: 194 — a rate-limited poll waits for what the provider said

**Project**: /projects/pi-improved · **Phase**: the exercise matrix's E5, inbound half of the 429 class
**Filed**: 2026-10-19 by the orchestrator, from `_docs/05-operations/EXERCISE-RESULTS-2026-10-19.md` (E5 row).

## Overview

MEASURED 2026-10-19 07:11:48–07:14:57Z on the deployed binary (`121fb6de67e1`): the runtime reader opens ONE
incident — `refusal-repeated:channel.poll.failed count=22 first_seen=07:11:48Z last_seen=07:14:57Z
latency_secs=7` — and every one of those 22 lines reads `unavailable: discord: poll status 429`. The digest
fired once that morning; Discord had rate-limited the channel after the posting burst, and the inbound loop
asked again every **~1.2 s** (its configured `poll_interval_ms`) because nothing on the failure path knows
what Discord said.

Three facts about the path, read in the tree today (not assumed):

- `crates/pii-channels/src/discord.rs:149-154` builds the poll error as
  `format!("discord: poll status {}", resp.status)`. It DISCARDS the response, so the `retry_after` Discord
  sends in the 429 body never reaches the poll error. The SEND path already carries it, measured in 1,836
  historical refusals: `discord: forum post status 429: {"message":"You are being rate limited.",
  "retry_after":132.176,"global":false}`.
- `crates/pii-server/src/daemon/router.rs:109-112` records the error (`channel_store.record_error(name)`)
  and returns to a `sleep(interval_ms.max(10))`. The loop's only pacing is the configured interval.
- **Correction to the E5 row's second residual** ("the error backoff never leaves the store — `record_error`
  bumps a counter that `router.rs` never reads back"): the counter IS read — `daemon/status.rs:152-161`
  reports it as `ChannelStatus.errors` — but it is CUMULATIVE ("how many API errors this channel has
  absorbed"), so it cannot be re-purposed as a backoff state without changing what the operator sees. The
  pacing state is process-local by design: what it describes is the last exchange, and a restart's first
  poll rebuilds it.

## Requirements

- **R1 — one token reader, beside the house pattern.** `pii_channels::retry_after_ms(e: &PiiError) ->
  Option<u64>` in `lib.rs` (next to `is_auth_rejected`, the existing text-token reader): find
  `"retry_after":<number>` (Discord's JSON body) or `retry_after=<number>s` (the poll form), read it as
  seconds, convert to ms, clamp to `1..=600_000`, `None` when absent or unparseable.
- **R2 — the poll error keeps the provider's reason.** Discord's `poll` non-success arm reuses
  `refusal("poll", &resp)` — the SAME bounded (300 chars), scrubbed formatter the send path uses — instead of
  the bare status string. The token is scrubbed by `refusal`; no second formatting path is introduced.
- **R3 — the router paces itself from the evidence.** In `router.rs`, `poll_once` returns the failure to the
  loop (no signature change to `Channel`), and the loop sleeps
  `max(interval, retry_after_ms(err) if Some, else interval * 2^min(consecutive-1, 4))`, capped at 300 s.
  `consecutive` is per channel, per process, and reset by a poll that returns `Ok`.
- **R4 — the pure function is pinned, the loop is driven.** The pause computation is a pure function
  (`poll_pause(err_text: Option<&str>, consecutive: u32, interval_ms: u64) -> u64`, `pub(crate)`) with exact
  unit tests; ONE integration test drives `spawn_all` with a fake channel that fails with the real 429 text
  and then succeeds, proving the loop uses it (wide margins, no sleeps tuned to the millisecond).
- **R5 — controls (each an assertion).**
  1. An error WITHOUT `retry_after` keeps the documented exponential backoff and never exceeds the 300 s cap.
  2. A success after failures returns to the configured interval (the backoff does not leak past recovery).
  3. The pre-fix historical text (`discord: poll status 429`, no body) parses to `None` — a log or error
     written by the old binary cannot produce a phantom pace.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `the_retry_after_token_is_read_from_a_real_refusal` (pii-channels lib) | the literal 429 body measured in the 1,836 refusals, and `retry_after=1.5s` | `Some(132_176)` and `Some(1_500)`; the pre-fix text → `None` |
| `a_429_with_retry_after_paces_the_next_poll` (pii-server `tests/poll_backoff.rs`) | fake channel: one 429 with `retry_after` = 1 s, then success; interval 10 ms | calls ≤ 3 within 400 ms (unpaced would be ≥ 20), then a success resets the count |
| `an_error_without_retry_after_uses_the_capped_backoff` (pure) | `(None, 1..=20, 1000)` | 1000, 2000, 4000, 8000, 16000, 16000… and ≤ 300_000 always |
| `a_success_returns_to_the_interval` (pure + loop) | consecutive = 7 then `Ok` | the next pause is the configured interval |
| `the_pre_fix_error_text_produces_no_pace` (control) | `unavailable: discord: poll status 429` | `poll_pause(Some(that), 1, 1200) == 1200` |

## Exit Criteria

- [ ] `cargo test -p pii-channels --lib the_retry_after_token_is_read_from_a_real_refusal -- --exact 2>&1 | grep -q '1 passed'`
- [ ] `cargo test -p pii-server --test poll_backoff 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-channels 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `grep -q 'fn retry_after_ms' crates/pii-channels/src/lib.rs`

## Guardrails

- **Do NOT change the `Channel` trait.** Eight adapters and their mocks implement it; a result-type change is
  a different spec with its own criterion. The pace travels as the token R1 reads, the way the codebase
  already carries `auth rejected` (`is_auth_rejected`).
- **Do NOT re-purpose `channel_store.record_error`/`ChannelCursor.errors`.** It is the CUMULATIVE health
  report (`ChannelStatus.errors`); keep reporting it and keep calling it, unchanged.
- **Do NOT sleep longer than 300 s inside the poll loop**, and do not sleep inside `poll_once` (it runs on the
  blocking pool; the sleep belongs to the async loop, where the interval already lives).
- **Do NOT drop the reason or leak the token.** `refusal()` is the only formatter; its 300-char bound and
  `scrub()` stay.
- **A green `cargo test` is not a green gate**: `bash quality-gate.sh --routine` must pass on the tree this
  leaves (fmt, clippy, build), and `cargo test -p pii-server --lib` must stay green.
- **The criterion's test must drive the production path.** The integration test constructs the daemon's own
  `spawn_all` loop with a fake `Channel`; a test that re-states `poll_pause` in its own body verifies a copy
  of the logic (spec 191's measured failure mode), so the pure tests pin the arithmetic and the loop test
  pins the caller.
- **Mutation controls to record in the Closure**: (a) make `poll_once` drop the error before returning →
  the loop test must redden; (b) make the loop sleep only `interval` → the loop test must redden; (c) make
  `retry_after_ms` return a constant → the parser test and the pre-fix control must redden.

## Closure

(To be filled by the orchestrator after independent verification: the criteria's own output, the mutation's
red output, the byte-identical restore, and what this does and does not close in the E5 row.)

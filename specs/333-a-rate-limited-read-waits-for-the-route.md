# Spec: A rate-limited read waits for the route, not for our own timer

## Overview

pii polls the Discord channel for inbound messages and starts one graph run per message. That is the path that
makes the forum a conversation rather than a noticeboard.

Measured 2026-11-02 on production (`~/.local/state/pii/pii.log`): **173** `channel.poll.failed` events, every
one of them `429 … "retry_after": 0.3`, arriving about once a second — and **not one message read**. The
operator's report was *"i am not getting answers on the discord channel"*, and this is why: pii could write, and
could not hear.

Two causes, both ours:

1. `[[channel]] poll_interval_ms` defaulted to **500 ms**, i.e. two reads a second against a route that allows
   about five per five seconds. The default was below the provider's limit by construction.
2. `poll_pause` (spec 194) does honour `retry_after`, but as `interval.max(asked)` — so a 0.3 s answer from
   Discord was swallowed by our own 500 ms timer and the next read went out on schedule.

**Project**: <repo>
**Phase**: production hardening

## Requirements

**R1 — the default respects the route.** `default_poll_ms()` is **1500**, and its doc comment carries the
measurement that forced it.

**R2 — a discord channel cannot be configured below the limit.** `poll_interval_for("discord", configured)`
returns at least `DISCORD_MIN_POLL_MS` (1000). An operator asking for 5 s keeps 5 s; asking for 500 ms gets 1000.

**R3 — the floor belongs to Discord, not to polling.** Every other kind keeps its own floor
(`cli`, `mqtt`, `webhook` → 10 ms), so fast test channels stay fast. This is the control: the clamp must not
become a general slowdown.

**R4 — a `429` still wins over the timer.** Unchanged from spec 194: when the provider names `retry_after`, the
pause is at least that (and at least the interval). This spec does not weaken it.

**R5 — nothing else about polling changes.** One `poll_once` per tick, on a blocking thread, one graph run per
message, cursor semantics untouched.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `a_discord_poll_cannot_be_configured_below_the_provider_limit` | `("discord", 500)`, `("discord", 20)`, `("discord", 5000)` | `1000`, `1000`, `5000` |
| same test, CONTROL rows | `("cli", 20)`, `("mqtt", 100)`, `("webhook", 1)` | `20`, `100`, `10` — no general slowdown |
| `retry_after_wins_over_the_backoff_and_never_undercuts_the_interval` | 429 text with `retry_after: 132.176`, interval 1200 | `132_176` |
| `an_error_without_retry_after_uses_the_capped_backoff` | no `retry_after` | 1000·2ⁿ capped at 300 s |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --lib 2>&1 | grep -q "137 passed"` (or more, zero failed)
- [ ] `grep -q 'const fn default_poll_ms' crates/pii-server/src/daemon/config.rs && grep -A1 'const fn default_poll_ms' crates/pii-server/src/daemon/config.rs | grep -q '1_500'`
- [ ] `grep -q 'DISCORD_MIN_POLL_MS' crates/pii-server/src/daemon/router.rs`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q "fresh-gate: PASS"`

## Guardrails

- Do NOT raise the cap in `MAX_POLL_PAUSE_MS`, and do NOT remove the `retry_after` branch of `poll_pause`.
- Do NOT apply the 1000 ms floor to non-discord kinds: that would break the fast test channels and prove nothing.
- Do NOT silence `channel.poll.failed`: a poll that fails must stay visible.

## Closure

_(the implementing cycle fills this with what it measured)_

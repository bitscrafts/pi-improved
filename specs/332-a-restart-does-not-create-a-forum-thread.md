# Spec: A restart does not create a forum thread

## Overview

pii posts one line at startup — `pii <version> is up (revision <rev>)` — to the Discord channel named by
`PII_DISCORD_CHANNEL_ID` (spec 157). That was written when the destination was a text channel, where the line
is a harmless sentence.

Measured 2026-11-02: the destination (`1558589728671146105`) is **the `pii` FORUM, `type: 15`**. In a forum every
message is a *thread*, so each daemon start creates a **new forum post**. The daemon restarted several times
today, and the operator's own words were: *"the messages you are sending to the channel has no use!"* — the
digests would be buried under startup notices, in the one surface that is meant to be the primary way to talk
to pii.

**Project**: /projects/pi-improved
**Phase**: production hardening

## Requirements

**R1 — silent by default.** `Daemon::post_verification` writes nothing to the channel unless
`PII_VERIFY_POST` is `1` or `true`. It returns `None` (nothing attempted), and the attempt counters
(`channel_posts`, `channel_post_refusals`) stay at 0.

**R2 — readiness stays observable.** The silent path emits one event, `channel.ready`, carrying the revision and
a note naming the switch. An operator must be able to see that the channel came up without reading Discord.

**R3 — the opt-in still works.** With `PII_VERIFY_POST=1` the behaviour of spec 157 is unchanged: exactly one
post, naming the revision, the token never in the text, and a refusal is counted and not fatal.

**R4 — no other channel write is affected.** Graph posts (`post` node) and digests keep their own paths and
gating; only the startup notice changes.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `a_startup_notice_is_silent_unless_the_operator_asks_for_it` | credentials set, `PII_VERIFY_POST` unset | returns `None`, **zero** HTTP requests, `channel_posts=0`, `channel_post_refusals=0` |
| `credentials_present_one_post_naming_the_revision_is_recorded` | credentials + `PII_VERIFY_POST=1` | exactly one POST, body names the revision, token absent |
| `a_refused_verification_post_is_counted_not_fatal` | credentials + flag, no route | `channel_posts=0`, `channel_post_refusals=1`, daemon alive |
| `no_credentials_the_daemon_starts_with_no_channel_and_posts_nothing` | no credentials | no channel, no request |

The control is the first row: same credentials, same destination, and **not one request reaches the network**.
The third row is the near-miss that proves the switch still gates a real post.

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test channel_wiring 2>&1 | grep -q "4 passed"`
- [ ] `grep -q 'PII_VERIFY_POST' crates/pii-server/src/daemon/channel_wiring.rs`
- [ ] `grep -q 'CHANNEL_READY' crates/pii-observe/src/names.rs`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q "fresh-gate: PASS"`

## Guardrails

- Do NOT delete `post_verification` or spec 157's behaviour: it stays behind the flag.
- Do NOT touch graph posts, digests, or the poll loop in this spec.
- Do NOT write the flag into the unit's `EnvironmentFile`: silence is the default, and an operator opts in.

## Closure

### Measured 2026-11-02

**What the destination is.** `GET /channels/1558589728671146105` → `name: pii`, `type: 15` (forum),
`guild_id: 1484208204627054704`. In a forum a message *is* a thread, so spec 157's startup line was creating a
forum post on every daemon start. The operator's report — *"the messages you are sending to the channel has no
use!"* — was accurate in a way the code could not see, because the adapter already supported forums and did the
right thing with what it was given.

**What changed.** `post_verification` returns `None` and emits `channel.ready` unless `PII_VERIFY_POST` is `1`
or `true`. Nothing else in the channel path changed.

**Evidence.** `cargo test -p pii-server --test channel_wiring` → **4 passed, 0 failed**, including the control
`a_startup_notice_is_silent_unless_the_operator_asks_for_it` (credentials present, zero HTTP requests,
`channel_posts=0`, `channel_post_refusals=0`) and the near-miss that keeps the opt-in honest
(`credentials_present_one_post_naming_the_revision_is_recorded`). `fresh-gate --worktree`: **PASS**, gate rc 0.

**What this does NOT fix.** The poll loop is being rate-limited (`429 … retry_after: 0.3`) roughly every second,
because `[[channel]] poll_interval_ms` defaults to **500 ms** and the poll path does not honour `retry_after`
(the parser exists, `pii_channels::retry_after_ms`, and is wired into *posting* only). Reading messages is what
makes the forum the primary way to talk to pii, so that is the next spec.

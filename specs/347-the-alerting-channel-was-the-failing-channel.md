# Spec: The channel that carries the alerts was the channel that failed

## Overview

On the host, between 2026-11-02 02:08:49Z and 03:05:45Z, the daemon logged **732** `channel.poll.failed` events
for the `discord` channel — `unavailable: net: write: Connection reset by peer (os error 104)`. The incident reader
folds them into one honest line (`refusal-repeated:channel.poll.failed count=731`), and then nothing happens,
because the only place pii reports an incident **is that same channel**.

Measured after the 17:03 deploy: 0 occurrences, so the failure has stopped. That is not the point. The point is
that for an hour the alerting path was down and the only record of it was a log line nobody was watching, and that
a transient network fault produced 732 attempts in 57 minutes — about one every 4.7 seconds, which is a retry loop
without backoff hammering a channel that is already unwell.

**Project**: /projects/pi-improved
**Phase**: production hardening — the failure mode that hides all the others.

## Verified premises (host log, 2026-11-02)

- 732 `channel.poll.failed`, first 02:08:49Z, last 03:05:45Z; error
  `unavailable: net: write: Connection reset by peer (os error 104)`, `channel: discord`.
- Occurrences since the deployed binary started (17:03): **0**.
- The reader already aggregates them into one incident with `count=731`, so the aggregation works; the response does
  not exist.
- `pii health` is served over the daemon's socket, not over the channel, so it is a path that survives this failure.

## Requirements

### Functional

1. **A channel outage is visible on a path that does not use the channel.** While any channel is failing, `pii
   health` reports it (a `degraded` line naming the channel, the error, the count and the time of the last attempt),
   and the daemon's own stdout/`--log-file` carries one line per state change — not one per attempt.
2. **Poll failures back off.** The retry interval grows on consecutive failures (start 1 s, double to a ceiling of
   5 min), and a success resets it. Measured control: a channel that fails 100 times in a row must produce at most
   ~20 attempts in the first minute, not 100.
3. **The incident says whether it is still true.** `refusal-repeated` for a channel carries `last_seen` and a
   `resolved_at` once a poll succeeds, so a reader can tell "it failed and recovered" from "it is failing now".
4. **The count is the number of attempts, and it is capped in the message** (e.g. `count=731+`) so a long outage does
   not produce a line that grows without bound.

### Non-Functional

- No new dependency; a small backoff state per channel in the daemon.
- The behaviour is testable without a network: the channel's poll is already behind a trait, so a test channel that
  fails N times and then succeeds drives both the backoff and the recovery.

## TDD Contract

| Test (`crates/pii-channels/tests/poll_backoff.rs`) | Assertion |
|---|---|
| `a_failing_poll_backs_off` | the REAL poll loop attempts ≤ 20 times in 60 s while failing, not a standalone struct |
| `CONTROL_a_healthy_poll_keeps_its_cadence` | a poll that succeeds every time keeps the base interval |
| `a_success_after_failures_resets_the_interval` | after a success the next interval is the base again |
| `health_reports_a_degraded_channel` | the REAL `pii health` output names the channel, the error and the last attempt — run against a daemon whose channel fails, not against a helper |
| `the_state_change_is_logged_once_not_per_attempt` | one `channel.degraded` line for the outage, one `channel.recovered` |
| `the_incident_carries_resolved_at_after_recovery` | `last_seen` set, then `resolved_at` once a poll succeeds |

## Exit Criteria

- [ ] `cargo test -q -p pii-channels --test poll_backoff 2>&1 | grep -q '6 passed'`
- [ ] `[ "$(grep -rn 'PollBackoff' --include='*.rs' crates/pii-channels/src crates/pii-server/src | grep -vc '/poll.rs:')" -ge 1 ]`
- [ ] `bash scripts/checks/gates.sh`

## Guardrails

- Do NOT report the outage only through the channel that is down.
- Do NOT lower the retry ceiling below 5 min or raise the base above 1 s "for safety": a slow channel must not stall
  the daemon, and a recovered channel must not take a quarter of an hour to be noticed.
- Do NOT log one line per attempt: 732 lines is a log nobody reads, and the aggregation that already exists proves
  the right shape.
- Do NOT touch the incident rule's fingerprints — this spec adds a state and a `resolved_at`, it does not rename what
  is already reported.

## Files

- `crates/pii-channels/src/poll.rs`
- `crates/pii-channels/tests/poll_backoff.rs`
- `crates/pii-cli/src/cmd_health.rs`
- `crates/pii-server/src/daemon/channels.rs`
- `crates/pii-learn/src/incident_runtime.rs`

## Closure

**Not done — the first attempt built a library nothing calls (2026-11-02).** Measured: `PollBackoff` and
`health_line` were defined in `crates/pii-channels/src/poll.rs` with six passing tests, and `grep -rn PollBackoff
--include='*.rs' crates | grep -v poll_backoff.rs` returned only its own definition. Nothing in the poll loop used
the backoff, and `pii health` did not report the channel, so both requirements had zero effect on the product while
every test was green.

This is the defect AGENTS.md §0b names: a check that passes while the live path is broken. The attempt is kept at
`_trash/347-unwired-20261102T224315Z` as the starting point; the spec now requires each row of its contract to be
proven THROUGH the running path — the poller's own attempts and the daemon's own `health` answer.

The acceptance test must fail if the wiring is removed: that is the only way to tell a working backoff from a
well-tested struct.

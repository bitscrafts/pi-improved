# Spec: one cron entry, one armed run

## Overview
MEASURED 2026-10-18 (070 section 0): the daily digest produced ~1,846 runs at ONE occurrence — 1,836 failed, 10 succeeded, and the 10 successes have run ids spaced by 32, i.e. TEN DIFFERENT daemon processes. EVERY DAEMON START ARMS ANOTHER COPY of the same cron entry and nothing cancels the stale one. Production holds FOUR armed copies right now for cron_entries: 1, so the next occurrence executes all four.

Reproduced in scratch on the deployed binary: one armed run executes once per occurrence; three kill -9 restarts give suspended 1 -> 2 -> 3, and at the occurrence ALL THREE execute.

**Project**: /projects/pi-improved

## Requirements
A daemon start MUST NOT arm a second run for a cron entry that already has a suspended run for that entry. Deduplicate on the entry name in start_cron_entries, or cancel stale suspended runs at boot and log that it did — either way, after N restarts the armed count equals cron_entries.

## Exit Criteria
- [ ] cargo test -p pii-server --test cron_arming_is_idempotent
- [ ] grep -q 'cron.rearmed' crates/pii-observe/src/names.rs

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT test this on the live daemon: every restart arms another copy. The proof runs on a SCRATCH daemon on a scratch store.
- Do NOT delete a suspended run silently: cancelling one is an event.
- `start_cron_entries` is in crates/pii-server/src/daemon/mod.rs, which sits near the 450-line gate (spec 182): put the new logic in daemon/cron.rs and keep mod.rs's line count where it is.
- The commit takes only crates/pii-server/src/daemon, crates/pii-server/tests and crates/pii-observe/src/names.rs. If the fix needs a file outside those, STOP and say so — a file left out of the commit is a HEAD that does not build (cabf634).

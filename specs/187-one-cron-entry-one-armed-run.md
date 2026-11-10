# Spec: one cron entry, one armed run

## Overview
MEASURED 2026-10-18 (070 section 0): the daily digest produced ~1,846 runs at ONE occurrence — 1,836 failed, 10 succeeded, and the 10 successes have run ids spaced by 32, i.e. TEN DIFFERENT daemon processes. EVERY DAEMON START ARMS ANOTHER COPY of the same cron entry and nothing cancels the stale one. Production holds FOUR armed copies right now for cron_entries: 1, so the next occurrence executes all four.

Reproduced in scratch on the deployed binary: one armed run executes once per occurrence; three kill -9 restarts give suspended 1 -> 2 -> 3, and at the occurrence ALL THREE execute.

**Project**: <repo>

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

## Closure

VERIFIED 2026-10-18 by the orchestrator, independently of the worker's report, on the committed tree (`18691a2`):

- **Criterion 1** `cargo test -p pii-server --test cron_arming_is_idempotent` → `test result: ok. 1 passed; 0 failed`.
- **Criterion 2** `grep -q 'cron.rearmed' crates/pii-observe/src/names.rs` → rc=0.
- **Mutation (the orchestrator's own, then restored).** `arm_cron`'s dedupe search was replaced by
  `let existing: Option<String> = None;` — i.e. every start arms another copy, the defect this spec describes.
  The criterion's own test goes red: `assertion left == right failed: four starts armed more than one run`,
  `left: 4`, `right: 1`, `test result: FAILED. 0 passed; 1 failed`. The three touched files were restored
  byte-identical (`sha256sum -c`: `41dd24c4…` cron.rs, `fddc143f…` mod.rs, `0a25111f…` the test) and the
  criterion re-ran green (`1 passed`) on those exact bytes.
- **PITFALL, measured here.** Restoring with `cp -p` keeps the file's original mtime, which is older than the
  mutated build artifact: cargo then reuses the mutated test binary and reports a **false red** on a file whose
  `sha256` already matches. `touch` the restored file before the green re-run — the bytes, not the mtime, are
  the evidence.
- **Line gate:** `cargo run -q -p xtask -- lint-files` → 319 files, max effective 446 lines
  (`crates/pii-memory/src/entity/store.rs`); `daemon/mod.rs` stayed inside it.

**What this closes, and what it does not.** This is the in-tree half of 070 §1 step 1: a start no longer arms a
second run for an entry that already has a suspended one. The production half is a deploy measurement and stays
**open**: production still runs the pre-fix binary and holds 4 suspended digest runs for `cron_entries: 1`, so
2026-10-19 09:00 UTC is expected to execute 4. It closes only when the redeployed binary reads
`pii run ls | jq '[.[]|select(.status=="suspended")]|length'` equal to `cron_entries` after restarts, and the
occurrence itself produces exactly one run.


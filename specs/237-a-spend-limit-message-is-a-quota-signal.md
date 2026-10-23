# Spec: 237 — a spend-limit message from an agent is journalled as a quota signal

**Project**: /projects/pi-improved
**Phase**: filed 2026-10-23T03:00Z by the orchestrator. Premises read, not assumed: `coder.exited` is built in
`crates/pii-server/src/node_kinds/nodes/coder/report.rs` (field groups `criteria_fields`, `output_fields`,
`tree_fields`, `claude_fields`, `route_fields`); the agent's captured stdout and stderr are `RunReport.agent_output` and
`RunReport.agent_stderr`; the refusal shapes the loop already trusts are in `scripts/autonomy/window.sh` (`parse`).

## Overview

A claude run that hits the account's limit exits non-zero with the CLI's own words in its output, e.g.
`You've hit your session limit · resets 4:50pm (UTC)` and `"api_error_status":429` (measured 2026-10-13, recorded in
`window.sh`). Through the coder today that is a plain `worker exited 1`: nothing says it was quota, nothing carries the
reset time, so the run cannot be told from a code failure and a retry policy has nothing to read.

This spec only RECORDS the signal in `coder.exited`. It does not change any verdict, park anything, or retry.

## Requirements

- **R1** A new module `crates/pii-server/src/node_kinds/nodes/agent_quota.rs` exposes
  `pub fn quota_signal(output: &str, stderr: &str) -> Option<QuotaSignal>` and
  `pub struct QuotaSignal { pub reset_text: String }`, registered as `pub mod agent_quota;` in
  `crates/pii-server/src/node_kinds/nodes/mod.rs`.
- **R2** It returns `Some` only for the CLI's own shapes, matched case-insensitively in stdout or stderr:
  `hit your (session|weekly|monthly) limit`, or `"api_error_status":429`, or `"rate_limit_error"`.
  `reset_text` is the text `resets H:MMam|pm (UTC)` when present, else the empty string.
- **R3** It returns `None` for text that merely MENTIONS the words: a brief that says "quota" or "rate limit", a plain
  error, an empty output.
- **R4** `coder.exited` gains `quota_exhausted` (`"yes"`/`"no"`) and `quota_reset` (the `reset_text`, or empty), added as
  one more field-group function in `report.rs` and chained in `exited_event`.
- **R5** Nothing else changes: `stop_reason`, the verdict, the routing and every existing field stay as they are.

## Architecture

A pure function over two strings plus one field group. Rejected: classifying inside `stop_reason` (it takes only the exit
code and is pinned by `coder_stop_reason`), and acting on the signal (parking, retrying) — that is a policy the operator
has not chosen.

## TDD Contract

New test file `crates/pii-server/tests/coder_quota_signal.rs`. Fixture strings are the CLI's real ones, copied from
`scripts/autonomy/window.sh` and its tests, not invented.

| test | given | expects |
|---|---|---|
| `a_session_limit_is_a_quota_signal` | `You've hit your session limit · resets 4:50pm (UTC)` | `Some`, reset_text `resets 4:50pm (UTC)` |
| `a_weekly_and_a_monthly_limit_are_too` | `hit your weekly limit`, `hit your monthly limit` | `Some` each |
| `a_429_status_is_a_signal` | `{"api_error_status":429}` | `Some`, reset_text empty |
| `a_rate_limit_error_is_a_signal` | `{"type":"rate_limit_error"}` | `Some` |
| `the_signal_is_found_in_stderr_too` | the session-limit text only in stderr | `Some` |
| `a_brief_that_mentions_quota_is_not_a_signal` | `Reply about the quota and the rate limit policy` | `None` (CONTROL) |
| `a_plain_error_is_not_a_signal` | `Error: Provider error: HTTP 404 model not found` | `None` (CONTROL) |
| `empty_output_is_not_a_signal` | `""`, `""` | `None` (CONTROL) |
| `the_event_carries_the_fields` | build the `coder.exited` event for a run whose output holds the session-limit text | `quota_exhausted` = `yes`, `quota_reset` = the reset text |
| `a_clean_run_says_no` | a run with ordinary output | `quota_exhausted` = `no`, `quota_reset` empty |

The requirement a plausible implementation gets wrong is R3: a matcher on the WORD "quota" or "limit" fires on a brief that
quotes the operator and would label ordinary failures as quota. The three CONTROL rows catch it.

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test coder_quota_signal 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `grep -q "quota_exhausted" crates/pii-server/src/node_kinds/nodes/coder/report.rs`
- [ ] `grep -q "pub mod agent_quota" crates/pii-server/src/node_kinds/nodes/mod.rs`
- [ ] `cargo test -q -p pii-server --test coder_stop_reason 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`

## Guardrails

- `specs/` are READ-ONLY for the implementer. If the code's shape differs from the premises above (no `report.rs` field
  groups, another event builder), STOP and report the mismatch instead of forcing the change.
- Never weaken a test. Never change `stop_reason`, the verdict or the routing.
- The fixture strings come from `scripts/autonomy/window.sh`; do not invent others.
- Tests write under `_tmp/` (never `/tmp`). 450-effective-line file gate and `#![forbid(unsafe_code)]` apply.
- Do NOT commit; the orchestrator commits.

## Closure

_(empty — the implementer's readings go here)_

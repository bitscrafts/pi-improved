# Spec: Health answers in constant time

**Project**: <repo> · **Phase**: 0.2.1 · **Source**: Opus finding 5, split out of row 357.

## Why this exists

`pii health` replays **every journal of every run on every call**: `daemon/identity.rs:289` →
`pii-graph/src/agent_calls.rs:237-273`, then a second scan in `daemon/since_deploy.rs:42-98`. It runs inline, on one
of four async workers (`engine/dispatch.rs:43`; `is_heavy` in `conn.rs:213` does not cover it).

Measured on the host: **82 · 85 · 92 ms** for 1,895 runs and 41,150 events, linear in history. At 10× the history
that is ~0.9 s of a blocked worker per health call — and health is the thing every watchdog and every operator
reaches for first.

The fix is not a cache: it is that the answer is a **status**, and a status should be maintained, not recomputed.

**Corrected 2026-11-04 (orchestrator; row 370 had failed three attempts on it).** This spec's `## Files` and its third
exit criterion both named `crates/pii-server/src/daemon/conn.rs`, which does not exist: `is_heavy`, the heavy set R3
talks about, lives in `crates/pii-server/src/engine/conn.rs`. Criterion 3 could therefore never pass, and every
attempt was scored `crit=0/3` on work that had nothing to do with the failure. The criterion now names the item R3
requires, `Method::SysStatus`, rather than a lowercase string a comment would satisfy.

## Requirements

- **R1** The totals health reports are maintained in a `SysStatus` record updated when a run changes state, so a
  health call reads one record instead of N journals.
- **R2** If the record is absent (an older store), health rebuilds it once and says so, rather than reporting zeros.
- **R3** `sys.status` is in the heavy set so it cannot saturate the worker pool, and health is measured, not assumed.
- **R4** The numbers do not change: the same totals before and after, proven by a test that compares a rebuilt record
  against the replay on a fixture.

## Files

- `crates/pii-server/src/daemon/identity.rs`
- `crates/pii-server/src/daemon/since_deploy.rs`
- `crates/pii-server/src/engine/conn.rs`
- `crates/pii-server/tests/health_is_constant_time.rs`

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test health_is_constant_time 2>&1 | grep -q 'test result: ok'`
- [ ] `grep -q 'SysStatus' crates/pii-server/src/daemon/identity.rs`
- [ ] `grep -q 'Method::SysStatus' crates/pii-server/src/engine/conn.rs`

## Guardrails

- Do NOT change what health reports, only how it is obtained; R4's test is the proof.
- Do NOT skip the rebuild path: an upgraded store has no record.
- Do NOT add a cache with a TTL — a status that can be stale is worse than a slow read.

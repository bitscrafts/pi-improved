# Spec: Retention drops what the policy expires

**Project**: <repo> · **Phase**: 0.2.1 hardening · **Source**: Opus finding 19, split out of row 358.

## Why this exists

`pii-store` has no retention code at all: `grep -c 'retention\|prune' crates/pii-store/src/*.rs` is 0. Every run,
every journal event and every node call is kept for ever, so the store grows with uptime and a long-lived pii answers
"what happened" by scanning everything it has ever done.

That is the same shape as health replaying every journal (row 370) and `pii health` paying for the whole history: the
data is not wrong, it is unbounded.

## Requirements

- R1. Policy: a retention age for run journals and a retention age for node-call events, both from config with
  defaults that do not surprise (30 days and 7 days). A **sweep** removes rows older than the policy, and reports how
  many it removed.
- R2. A sweep is **bounded**: it removes at most N rows per call (default 1 000) so a first run on a large store does
  not hold a write transaction for minutes.
- R3. A sweep never removes a **live** run: a run whose status is `Running` or `Suspended` is kept regardless of age,
  because the daemon may still resume it.
- R4. `pii store prune` runs one sweep and prints the count; the daemon runs one sweep at boot.
- R5. The sweep writes one event naming the count, so a store that is being trimmed is observable rather than quietly
  shrinking.

## Files

- `crates/pii-store/src/retention.rs`
- `crates/pii-store/tests/retention_drops_what_the_policy_expires.rs`
- `crates/pii-cli/src/cmd_store.rs`
- `crates/pii-server/src/daemon/mod.rs`

## Exit Criteria

- [ ] `cargo test -q -p pii-store --test retention_drops_what_the_policy_expires 2>&1 | grep -qE 'test result: ok'`
- [ ] `grep -q 'retention' crates/pii-store/src/retention.rs`
- [ ] `grep -q 'prune' crates/pii-cli/src/cmd_store.rs`
- [ ] `cargo test -q -p pii-store --test retention_keeps_a_live_run 2>&1 | grep -qE 'test result: ok'`

## Guardrails

- Do NOT delete a row whose run is `Running` or `Suspended` (R3) — that is the difference between trimming and losing
  work in flight.
- Do NOT make the sweep unbounded (R2); the test must prove the cap is honoured, not just that old rows disappear.
- Do NOT touch `durable/` — the durable copies are the backup path and retention must not reach them.

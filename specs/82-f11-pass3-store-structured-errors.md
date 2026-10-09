# Spec: 82 — F-11 pass 3: `pii-store/src/lib.rs` uses the structured store error (64 sites)

**Project**: /projects/pi-improved
**Phase**: batch D — 043 §10 item 14 (carries 032's F-11 item forward).

## Context
`crates/pii-store/src/lib.rs` still builds string-typed errors by hand: 64 occurrences of
`PiiError::Internal(format!("store: <op>: {e}"))`. Spec 66 added the structured
constructor `PiiError::store_op(op, resource, detail)` (`crates/pii-abi/src/error.rs:96`)
and converted `heal.rs`/`session.rs`. This spec finishes the store's own `lib.rs`.

## Requirements
R1. Replace all 64 `PiiError::Internal(format!("store: <op>: {detail}"))` constructions in
    `crates/pii-store/src/lib.rs` with `PiiError::store_op(<op>, <resource>, <detail>)`,
    choosing `op`/`resource` from the real operation and target already named in the
    message (e.g. `store: begin_write: {e}` → `store_op("begin_write", "db", e)`; a
    per-table open like `store: open journal: {e}` → `store_op("open", "journal", e)`).
    Do NOT invent resources the message does not name.
R2. Nothing else changes: same failure conditions, same error semantics, no new variant,
    no message text concatenated by hand.
R3. `pii-store/src/testing.rs` keeps its synthetic `PiiError::Internal(String::from(..))`
    fault seams — those are deliberate test instrumentation, NOT store operations; the
    exit criterion below is scoped to `lib.rs`.
R4. Verify the error still renders usefully: at least one converted site's message must
    be asserted in a test (add a small test if none exists) so the Display shape is
    pinned.

## Exit Criteria
- [ ] `[ "$(grep -c 'PiiError::Internal(' crates/pii-store/src/lib.rs)" = 0 ]`
- [ ] `[ "$(grep -c 'PiiError::store_op(' crates/pii-store/src/lib.rs)" -ge 1 ]`
- [ ] `cargo test -p pii-store 2>&1 | grep -c 'test result: FAILED' | grep -q '^0$'`
- [ ] `cargo test -p pii-store 2>&1 | grep -c 'test result: ok' | grep -qv '^0$'`
- [ ] `cargo clippy -p pii-store --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --all --check` exits 0
- [ ] `cargo run -q -p xtask -- lint-files` exits 0 and reports NO warning for `crates/pii-store/src/lib.rs` (the file is near the 380 warn line; the shorter constructor form must not push it over)

## Guardrails
- Do NOT change redb tables, key/value encodings, transaction structure, or any public
  signature other than the error construction inside method bodies.
- Do NOT convert test-instrumentation seams (`testing.rs`, inline `#[cfg(test)]` blocks).
- Do NOT batch-edit with a blind regex that could touch `session.rs`/`mem/*.rs` — this
  spec's scope is `lib.rs` only.
- Do NOT commit; leave the tree for the orchestrator.
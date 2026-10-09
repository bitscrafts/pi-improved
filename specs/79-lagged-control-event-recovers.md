# Spec: 79 — a lagged control event is never silently dropped (D6)

**Project**: /projects/pi-improved
**Phase**: batch D — 043 §10 item 8 / §6 D6 (MED-HIGH).

## Context
`crates/pii-server/src/daemon/mod.rs:199` handles a broadcast lag as:
```rust
Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
```
The loop carries CONTROL events (e.g. `RunSuspended`, which arms the reactive timer
that resumes a suspended run). Dropping one means a suspended run never resumes —
silently, with no gap reported. The same file's subscription path already has the
required shape: `crates/pii-server/src/engine/subscribe.rs:134-143` reports the gap
(`Event::Lagged { dropped }`) with the comment "Never drop silently (F4)".

## Requirements
R1. `daemon/mod.rs` lagged arm: never a silent no-op. It must (a) record/report the
    gap the way the subscribe path does, and (b) RECOVER the control state a dropped
    event would have applied — concretely: rescan `store.runs_by_status(RunStatus::Suspended)`
    and re-arm the resume timer for every suspended run (idempotent; re-arming an
    already-armed run is a no-op).
R2. Keep the existing error/log channel style of that module (no new dependency, no
    panics; workspace lints deny unwrap/expect/panic).
R3 (TDD, RED first). A test that reproduces the drop: a run is registered as
    `Suspended`, a large burst of events (≥5,000) is published around it so the
    receiver lags, and the test asserts the resume timer IS armed afterwards. It must
    FAIL before the fix (observe and record the red) and pass after.

## Exit Criteria
- [ ] `[ "$(grep -c 'Lagged(_)) => {}' crates/pii-server/src/daemon/mod.rs)" = 0 ]` (the silent swallow is gone)
- [ ] `[ "$(grep -c 'runs_by_status' crates/pii-server/src/daemon/mod.rs)" -ge 1 ]` (the recovery rescan exists at the call site)
- [ ] `cargo test -p pii-server lagged 2>&1 | grep -q 'test result: ok'` (the new test passes)
- [ ] `cargo test -p pii-server 2>&1 | grep -c 'test result: FAILED' | grep -q '^0$'` (no failing suite)
- [ ] `cargo clippy -p pii-server --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --all --check` exits 0

## Guardrails
- Do NOT change the bus/broadcast channel sizes, the event wire shape, or the
  subscription protocol.
- Do NOT make the rescan O(all runs) on every event — it runs only on the lagged
  arm (an exceptional path), which is the point.
- Tests under `_tmp/`, never `/tmp`. Do NOT commit; leave the tree for the orchestrator.
- Record in the report: the red evidence for R3 (exact assertion + output before the fix).
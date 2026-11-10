# Spec: The idle daemon leaves no files and no busy loops

**Project**: <repo> · **Phase**: 0.2.1 · **Source**: Opus findings 13, 14, 15, 20 — the rest of row 357.

## Why this exists

Four costs an idle daemon pays, measured 2026-11-03:

- **`graph.lint` creates four redb files per call** (`engine/registry.rs:174,241,247,258` creates `lint-{approval,heal,
  memory,queue}.redb`), inline on an async worker. On the host, `/run/user/1000/pii/lint-*.redb` are four files of
  36,864 B each — about eight fsync transactions per lint RPC, for tables nothing persists.
- **the promotion sweep re-parses up to 10,000 files every 5 s** (`daemon/learn_loop.rs:19-29` →
  `pii-learn/src/promote/mod.rs:52-58,386`): each sweep truncates `.writer.lock`, flocks, `read_dir`s and JSON-parses
  every file. That is **17,280 sweeps a day on an idle host**.
- **`max_in_flight` is configured and never read** (`pii-worker/src/spec.rs:16`, passed at `daemon/mod.rs:376`,
  never consulted in `control.rs`), and the supervisor's `events: Vec` grows without bound (`supervisor.rs:100,180`).
- **two config claims are stale**: `poll.rs:175`'s doc still describes the old 16× backoff cap, and `daemon/mod.rs:441`
  reloads `poll_interval_ms` while `router.rs:40` captured the interval at spawn — so the reload cannot change pace.

## Requirements

- **R1** Lint uses `redb::backends::InMemoryBackend`: `pii graph lint` leaves no file behind, proven by a test that
  lints and then asserts no `lint-*.redb` exists anywhere under the state and runtime dirs.
- **R2** The sweep is skipped when the directory's mtime is unchanged since the last sweep, and the skip is counted
  in an event so an operator can see it working.
- **R3** `max_in_flight` is enforced: a second invoke beyond the limit is refused with a typed error, and the
  supervisor's event vector is capped (1,024) with the old ones dropped and counted.
- **R4** The two stale claims are made true or removed: the reload either changes the next sleep (proven by a test)
  or the field is dropped from the reload and the doc says a restart is required.

## Files

- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-learn/src/promote/mod.rs`
- `crates/pii-server/src/daemon/learn_loop.rs`
- `crates/pii-worker/src/control.rs`
- `crates/pii-worker/src/supervisor.rs`
- `crates/pii-server/src/daemon/router.rs`
- `crates/pii-server/tests/lint_leaves_no_files.rs`
- `crates/pii-server/tests/sweep_skips_an_unchanged_dir.rs`
- `crates/pii-worker/tests/in_flight_is_enforced.rs`

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test lint_leaves_no_files 2>&1 | grep -q 'test result: ok'`
- [ ] `grep -q 'InMemoryBackend' crates/pii-server/src/engine/registry.rs`
- [ ] `cargo test -q -p pii-server --test sweep_skips_an_unchanged_dir 2>&1 | grep -q 'test result: ok'`
- [ ] `grep -q 'modified()' crates/pii-learn/src/promote/mod.rs`
- [ ] `cargo test -q -p pii-worker --test in_flight_is_enforced 2>&1 | grep -q 'test result: ok'`

## Guardrails

- Do NOT weaken lint: the same node kinds must be checked, only the backend changes.
- Do NOT skip a sweep that is owed: the mtime gate must still sweep when the directory changed.
- Do NOT let a cap silently drop work; the drop is counted and reported.
- The near-miss control for R2: a changed directory must sweep.

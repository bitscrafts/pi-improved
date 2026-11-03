# Spec: The daemon idles cheaply

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening · **Source**: Opus optimisation review 2026-11-03,
workstream **W5** — closes findings 5, 13, 14, 15 and 20.

## Verified premises

- `pii health` replays every journal of every run on every call: `daemon/identity.rs:289` →
  `pii-graph/src/agent_calls.rs:237-273`, plus a second scan in `daemon/since_deploy.rs:42-98`; `runs` runs inline
  (`engine/dispatch.rs:43`) and is not in `is_heavy` (`engine/conn.rs:213`). Measured on the production host:
  **82, 85 and 92 ms** over 1,895 runs.
- `daemon/learn_loop.rs:19-29` sweeps every 5 s: a truncating `File::create(".writer.lock")`, a flock, a `read_dir`
  and a JSON parse of up to 10,000 files — **17,280 sweeps a day** on an idle host.
- `engine/registry.rs:174,241,247,258` calls `Database::create` for four `lint-*.redb` files on every lint RPC,
  inline on an async worker.
- `pii-worker/src/spec.rs:16` defines `max_in_flight`, `daemon/mod.rs:376` passes it, and `control.rs:28-75` never
  reads it; `supervisor.rs:100,180` grows `events: Vec` without a bound.
- `daemon/mod.rs:441-452` reloads `poll_interval_ms` but `router.rs:40` captured it at spawn; `poll.rs:175` still
  documents the removed 16× cap.

## Requirements

**R1.** `SysStatus`/health totals are O(1): cached and keyed by journal length, so a call with no new events does
no replay. `sys.status` joins `is_heavy` or runs on the blocking pool.

**R2.** The promotion sweep runs only when the directory's mtime changed since the last sweep.

**R3.** `graph.lint` opens its stores with `redb::backends::InMemoryBackend`; no `lint-*.redb` file is created.

**R4.** `max_in_flight` is enforced: a second invoke above the limit is refused with a typed error. The supervisor's
event log is capped (1,024 entries, oldest dropped).

**R5.** Either reload applies the new poll interval, or the field is removed from reload and the doc says "restart
required". `poll.rs:175` must describe the code that exists.

## The requirement a plausible implementation satisfies in appearance only

A worker can memoise health by run count and pass a synthetic benchmark while the host still replays, because the
key does not change when a journal grows. The measurement below is taken on the host with `time`, and the sweep test
counts `openat` with `strace`, not with a unit test of the cache.

## TDD contract

| file | test | assertion |
|---|---|---|
| `crates/pii-server/tests/health_is_constant_time.rs` | `a_second_call_with_no_new_events_does_not_replay` | 20,000 synthetic runs: the second `sys.status` is under 5 ms and the replay counter does not increase |
| `crates/pii-server/tests/sweep_skips_an_unchanged_dir.rs` | `an_unchanged_dir_is_not_parsed_again` | two sweeps, one file touched between them → the first parses N files, the second parses 0 |
| `crates/pii-server/tests/lint_creates_no_files.rs` | `a_lint_leaves_no_redb_behind` | after a lint RPC the temp state dir holds no `lint-*.redb` |
| `crates/pii-worker/tests/in_flight_is_enforced.rs` | `a_second_invoke_above_the_limit_is_refused` | `max_in_flight = 1`, two invokes → the second is a typed refusal |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test health_is_constant_time 2>&1 | grep -qE 'test result: ok. 1 passed'`
- [ ] `cargo test -q -p pii-server --test sweep_skips_an_unchanged_dir 2>&1 | grep -qE 'test result: ok. 1 passed'`
- [ ] `cargo test -q -p pii-worker --test in_flight_is_enforced 2>&1 | grep -qE 'test result: ok. 1 passed'`
- [ ] `grep -q 'InMemoryBackend' crates/pii-server/src/engine/registry.rs`
- [ ] `cargo test -q -p pii-channels --test poll_backoff 2>&1 | grep -qE 'test result: ok. 6 passed'`

## Guardrails

- Do NOT change what `pii health` reports, only how it is computed; the identity fields must stay identical.
- Do NOT weaken `poll_backoff` (6 tests) or `timer_no_refire`.
- Do NOT make the sweep skip when a file changed: mtime is the signal, and the control must show a changed dir IS swept.
- The worker does not commit; the orchestrator reviews and commits.

## Files

- `crates/pii-server/src/daemon/identity.rs`
- `crates/pii-server/src/daemon/since_deploy.rs`
- `crates/pii-server/src/daemon/learn_loop.rs`
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-server/src/engine/conn.rs`
- `crates/pii-server/src/daemon/router.rs`
- `crates/pii-worker/src/control.rs`
- `crates/pii-worker/src/supervisor.rs`
- `crates/pii-channels/src/poll.rs`
- `crates/pii-server/tests/health_is_constant_time.rs` (new)
- `crates/pii-server/tests/sweep_skips_an_unchanged_dir.rs` (new)
- `crates/pii-server/tests/lint_creates_no_files.rs` (new)
- `crates/pii-worker/tests/in_flight_is_enforced.rs` (new)

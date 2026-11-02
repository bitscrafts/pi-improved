# Spec 331: a test that panics leaves no daemon behind

## Overview

Two `pii daemon` processes started by a gate run on 2026-10-22 were still alive nine days later. They were found and
reaped on 2026-11-01T23:59:57Z, with their argv, age and socket count written to
`/projects/_tmp/reaped-test-daemons-20261101T235957Z.log`:

```
candidate 583964 age=9-21:17:18 rss=21492kB
  argv: _tmp/tools-gate/debug/pii daemon --foreground --store .../_tmp/fake-agent/store.redb ... --log-file .../pii.log
candidate 585460 age=9-21:16:08 rss=18644kB
  argv: _tmp/tools-gate/debug/pii daemon --foreground --store _tmp/local-agent-test/store.redb ...
sockets: 0 listening
```

Both were started by a test under the gate's own target directory, survived the gate that started them, survived every
gate since, and held a redb store and a daemon control socket the whole time. The tests that spawned these two are no
longer in the tree; the shape that leaked them is, which is why this is a spec and not a cleanup.

**Measured on HEAD `9e87635`, 2026-11-02T00:2xZ** (`grep -c '\.spawn()'`, `grep -c 'impl Drop'` over
`crates/pii-cli/tests/*.rs`):

- **11 files spawn a live `pii daemon`**: `control_plane_is_not_behind_runs`, `daemon_log_wiring`, `health_drift`,
  `memory_learn`, `project_config_layers_live`, `run_cancel`, `run_cancel_kill`,
  `run_start_resolves_like_graph_list`, `skill_cli_reads_through_the_daemon`, `trace_absent_run_names_the_store`,
  `trace_reads_through_the_daemon`.
- **15 `.kill()` sites across those 11 files.**
- **5 files declare a correct kill-on-drop guard**, each its own copy of the same six lines
  (`struct Daemon(Child); impl Drop … kill + wait`), plus `LiveDaemon` in `project_config_layers_live`.
- **5 files spawn a daemon with no guard at all** — `daemon_log_wiring`, `memory_learn`, `run_cancel`,
  `run_cancel_kill`, `run_start_resolves_like_graph_list` — and reach `child.kill()` only after every `unwrap()`
  and *before* the final `assert!`. `daemon_log_wiring.rs:39-57` is the shape: spawn, a 15 s polling loop with
  three `unwrap()`s, `kill`, `wait`, then `assert!(found, …)`.
- **And the guard is not enough where it is not used by every spawn.** `project_config_layers_live.rs:33-60` — a
  file that *does* have `LiveDaemon` — spawns its own daemon inside `start_and_read_log` and stops it with
  `child.kill()` after a 20 s polling loop full of `unwrap()`s.

Rust's `Child` has no `Drop` that kills: dropping a `Child` leaves the process running. So in every one of those
paths a panicking assertion — including the assertion the test exists for, which is the *failing* half of TDD —
turns a test failure into a daemon the gate inherits, holding a store the next gate may want.

The cost is not memory. These daemons hold a redb file and the daemon control socket, and the fixed-store class has
already cost this repo measured work (059 F4: 2 of 6 concurrent rounds died with `Database already open. Cannot
acquire lock`). A gate that leaves a daemon behind makes the next gate's store and socket reads nondeterministic —
the class of failure that is later blamed on the code under test.

## Requirements

**R1 — one guard, one implementation.** New `crates/pii-cli/tests/daemon_guard/mod.rs`:

- `pub struct DaemonGuard(Option<std::process::Child>)` with `pub fn new(child: Child) -> Self`,
  `pub fn pid(&self) -> u32`, `pub fn child_mut(&mut self) -> &mut Child` (so a polling loop can `try_wait`),
  and `pub fn kill_now(&mut self)`.
- `kill_now` is idempotent and tolerant: it kills, waits, sets the slot to `None`, and ignores the error a
  already-exited child returns. Calling it twice is not an error.
- `impl Drop for DaemonGuard` calls `kill_now`, so a panic, an `unwrap()` or an early `return` all reap the child.
- The module carries `#![allow(dead_code, clippy::unwrap_used, …)]` with its reason in the house style
  (`tests/lane_support/mod.rs` is the precedent), and is included with `#[path = "daemon_guard/mod.rs"] mod
  daemon_guard;` so it is never compiled as a test target of its own.

**R2 — every spawn site goes through it.** In each of the 11 files above, the child returned by `.spawn()` is owned
by a `DaemonGuard` at the point of spawn (or in the helper that returns it), the five local guard types
(`Daemon(Child)` ×5 and `LiveDaemon`) are deleted, and **no test file calls `.kill()` on a child by hand** — the
guard is the only killer. Polling (`try_wait`) and `wait_with_output` on a short-lived process are untouched.
`child.wait()` after an explicit `kill()` becomes `guard.kill_now()`. Every existing assertion, timeout and scratch
path stays exactly as it is; only the ownership of the child changes.

**R3 — the guard is proven against the real binary, and the defect is proven by a control.**
New `crates/pii-cli/tests/daemon_guard_holds.rs`, two tests:

- `a_panicking_test_leaves_no_daemon`: spawn the real `pii daemon` (scratch store, `--sandbox off`, `PII_RUNTIME_DIR`
  inside the test's own `_tmp` scratch), take its pid, wrap it in a guard inside
  `std::panic::catch_unwind(AssertUnwindSafe(…))`, `panic!` there, assert the closure returned `Err`, then poll
  `/proc/<pid>` and assert it is gone within 5 s.
- `a_dropped_child_survives_without_the_guard` (the control): spawn the same daemon, drop the raw `Child` without
  killing it, assert `/proc/<pid>` **still exists** — the defect, measured live, not asserted from the docs — then
  kill that pid explicitly (`/bin/kill -9`) and assert it is gone. The control must leave nothing behind.

**R4 — a file that spawns a daemon and kills by hand cannot land.** New
`scripts/checks/daemon-tests-are-guarded.sh`:

- For every `crates/pii-cli/tests/*.rs` that spawns the binary with `daemon` (a file that names `CARGO_BIN_EXE_pii`,
  calls `.spawn()`, and passes `"daemon"` as an argument), require (a) the text `DaemonGuard` and (b) **no** `.kill()`
  in the file. Print `RESULT: <n> ok, 0 FAIL` and exit 0, or one `FAIL <file>: <why>` line per offending file and
  exit 1.
- `--control` proves the checker in both directions, in a scratch tree it creates under `_tmp/`: an unguarded copy is
  refused (`CONTROL: the unguarded copy is refused and the guarded copy passes`), a copy that only adds the
  `DaemonGuard` reference passes, and `daemon_guard/mod.rs`'s own `.kill()` is **not** flagged (it is not a test
  target). A checker that cannot fail is not a check.

**R5 — the gate runs it.** One step in `gate-steps.txt`:
`daemon-guard|bash scripts/checks/daemon-tests-are-guarded.sh`.

## Files

- `crates/pii-cli/tests/daemon_guard/mod.rs`
- `crates/pii-cli/tests/daemon_guard_holds.rs`
- `crates/pii-cli/tests/control_plane_is_not_behind_runs.rs`
- `crates/pii-cli/tests/daemon_log_wiring.rs`
- `crates/pii-cli/tests/health_drift.rs`
- `crates/pii-cli/tests/memory_learn.rs`
- `crates/pii-cli/tests/project_config_layers_live.rs`
- `crates/pii-cli/tests/run_cancel.rs`
- `crates/pii-cli/tests/run_cancel_kill.rs`
- `crates/pii-cli/tests/run_start_resolves_like_graph_list.rs`
- `crates/pii-cli/tests/skill_cli_reads_through_the_daemon.rs`
- `crates/pii-cli/tests/trace_absent_run_names_the_store.rs`
- `crates/pii-cli/tests/trace_reads_through_the_daemon.rs`
- `scripts/checks/daemon-tests-are-guarded.sh`
- `gate-steps.txt`
- `specs/331-a-test-that-panics-leaves-no-daemon-behind.md`

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| a-panicking-test-leaves-no-daemon | real `pii daemon` in a scratch store, `panic!` inside `catch_unwind` with the guard alive | `/proc/<pid>` gone within 5 s |
| a-dropped-child-survives-without-the-guard | the same daemon spawned and the raw `Child` dropped, no guard | `/proc/<pid>` still present (the defect), then gone after `/bin/kill -9` |
| the-checker-sees-an-unguarded-file | `--control`: a copy of `daemon_log_wiring.rs` with no `DaemonGuard` and a `.kill()` | `REFUSED`, the file named in a `FAIL` line |
| the-checker-accepts-a-guarded-file | `--control`: a copy that names `DaemonGuard` and has no `.kill()` | no `FAIL` for it |
| the-checker-ignores-the-guard-module | `--control`: `daemon_guard/mod.rs` with its own `.kill()` | not scanned; 0 hits |
| every-spawning-file-is-guarded | the 11 real files | `RESULT: 11 ok, 0 FAIL` |

## Exit Criteria

- [ ] `bash scripts/checks/daemon-tests-are-guarded.sh 2>&1 | grep -qE "^RESULT: [1-9][0-9]* ok, 0 FAIL"`
- [ ] `bash scripts/checks/daemon-tests-are-guarded.sh --control 2>&1 | grep -q "^CONTROL: the unguarded copy is refused and the guarded copy passes"`
- [ ] `cargo test -q -p pii-cli --test daemon_guard_holds 2>&1 | grep -qE "test result: ok\. [2-9][0-9]* passed"`
- [ ] `cargo test -q -p pii-cli --test daemon_log_wiring 2>&1 | grep -qE "test result: ok\. [1-9][0-9]* passed"`
- [ ] `cargo test -q -p pii-cli --test run_cancel 2>&1 | grep -qE "test result: ok\. [1-9]" && cargo test -q -p pii-cli --test run_cancel_kill 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-cli --test memory_learn 2>&1 | grep -qE "test result: ok\. [1-9]" && cargo test -q -p pii-cli --test run_start_resolves_like_graph_list 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `test "$(grep -rl 'impl Drop for .*Daemon' crates/pii-cli/tests/*.rs | wc -l | tr -d ' ')" -eq 0`
- [ ] `grep -q "^daemon-guard|bash scripts/checks/daemon-tests-are-guarded.sh" gate-steps.txt`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q "fresh-gate: PASS"`

## Guardrails

- **STOP on a shape mismatch.** If a spawn site cannot be expressed as "the guard owns the child" without changing
  what the test asserts — e.g. it needs the child's exit status through `wait_with_output`, or it re-reads `pid()`
  after an explicit kill — write the site and why in the Closure and stop. Do not force it, and do not delete its
  assertion to fit the guard.
- **Do not weaken a test.** Every `assert!`, timeout and scratch path in the 11 files stays byte-identical in
  meaning; the only change is who owns the child. Killing earlier or later than today is a behaviour change and is
  out of scope.
- **Do not sleep to make the guard test pass.** Poll `/proc/<pid>` with a deadline; a fixed `sleep` on a loaded
  machine is the flake this repo has already paid for.
- **The control must clean up after itself.** The control test asserts the leak *and* leaves no daemon: it kills the
  pid it proved was alive and asserts it is gone. A test that proves a leak by leaking is the same defect.
- **The checker's input is the live tree**, not a hand-written list: it scans `crates/pii-cli/tests/*.rs`, and its
  own `--control` plants a copy under scratch. Do not make it match a constant instead.
- **Do not touch the production daemon** (pid 1744962 at `669f5803d90a`): its argv contains no `_tmp/` store and it
  is not a test's child. The guard only ever kills a child the test spawned.
- **`specs/` is read-only for the implementer.** If the spec is wrong, stop and say which line; do not edit it.

## Closure

Open — filled when the item is verified on the tree the worker leaves.

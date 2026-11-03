# Spec: The gate tells the truth about the host it runs on

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening · **Finding**: measured 2026-11-03, while the
autonomy cycle was verifying spec 355.

## Verified premises (measured 2026-11-03, on this tree)

- **A gate deletes a running gate's export.** `xtask/src/fresh_gate.rs:115-125` (`stale_exports`) returns *every*
  `fresh-gate-*` directory under `/projects/_tmp` except its own, with no age and no liveness test, and
  `:338-342` removes them all before creating its own export. Two gates on this host therefore cannot coexist.
  Measured: gate A (export `fresh-gate-11434`, started 12:22Z) had reached `test (lib/bins/tests)` when gate B
  (`/tmp/gate-355e.log`, export `fresh-gate-197287`, started 12:35:31Z) took its retention pass. A then failed
  **sixteen** steps with `scripts/gate-lib.sh: line 28: target/gate-steps/<step>.log: No such file or directory`
  and `mkdir: cannot create directory 'target': No such file or directory`, reporting
  `fresh-gate FAILED; failed steps: [...]; failing tests: []` — a red with **no failing test**, blaming the tree
  for a directory another process deleted. The same thing killed a second attempt at 12:47Z.
- **A daemon a test spawns claims the operator's real durable store.** `crates/pii-cli/tests/daemon_log_wiring.rs:36-44`
  spawns the real `pii daemon` with `PII_RUNTIME_DIR` set to its scratch dir but **not** `PII_STATE_DIR`; the
  daemon resolves the durable dir through `pii_core::state_dir()` (`crates/pii-cli/src/cmd_daemon.rs:144-148`),
  which is `PII_STATE_DIR` then `$HOME/.local/state/pii` (spec 351). Measured in the 12:08Z gate:
  `store /root/.local/state/pii/durable/channel.redb is locked: another daemon holds it`, and the test failed at
  `daemon_log_wiring.rs:54`. That gate was red for as long as any daemon ran — the production condition.
- **The same gap makes a guard test pass vacuously.** `crates/pii-cli/tests/daemon_guard_reaps.rs:26-36` has the
  same shape. A spawned daemon that dies at the durable claim (`ExitCode::from(3)`) is already gone, so the two
  reaping assertions hold for a process that never started. Measured at 12:08Z, while a daemon held the store:
  `daemon_guard_reaps` reported `test result: ok. 2 passed` with both daemons refused.
- Exactly two test files have the gap:
  `for f in $(grep -rln 'arg("daemon")' crates --include=*.rs); do grep -q PII_STATE_DIR "$f" || echo $f; done`
  printed `daemon_log_wiring.rs` and `daemon_guard_reaps.rs` and nothing else.

## Requirements

**R1.** Retention removes an export only when its owner cannot be alive. The owner is the pid in the directory
name; a `/proc/<pid>` entry means live, and a live export is never removed — not by age, not by count.

**R2.** A gate whose export disappears mid-run reports that as its own condition (`export lost: <path>`) instead of
`FAIL: <step>`, so a deleted directory can never read as a failing test.

**R3.** Every test that spawns `pii daemon` sets `PII_STATE_DIR` to its own scratch directory, so the daemon it
spawns claims a durable store the test owns.

**R4.** `daemon_guard_reaps` proves its daemon **started** before it asserts on reaping: the daemon's log contains
`daemon.starting`. A refused start is a failed test, not a passed one.

**R5.** The daemon's resolution rule is unchanged: `PII_STATE_DIR` first, then `$HOME/.local/state/pii` (spec 351).
This spec changes what the tests own, not what the daemon resolves.

## The requirement a plausible implementation satisfies in appearance only

A worker can delete the retention loop ("nothing is ever removed") and two gates then coexist — at the cost of
every failed export staying on disk until the volume fills (it is at 92 %). The rule is not "stop removing"; it is
"never remove one whose owner is alive", and the test plants both cases in one call: a live owner's directory
survives, a dead owner's directory is removed.

The same trap in the fixture: a worker can set `PII_STATE_DIR` and still pass while a daemon on the host holds the
same store, because the daemon it spawns may fail to start for another reason. The test must assert the daemon's
own start line, which is why R4 is a requirement and not a comment.

## TDD contract

`xtask/src/fresh_gate.rs` (unit tests; the file already carries a `stale_exports` test):

| test | setup | assertion |
|---|---|---|
| `a_running_gates_export_is_not_reaped` | `<base>/fresh-gate-<pid of a live child>` and `<base>/fresh-gate-999999` | `stale_exports` returns only the dead pid's directory |
| `CONTROL_a_dead_owners_export_is_reaped` | `<base>/fresh-gate-999999` only | it is returned (the rule still reclaims disk) |

`crates/pii-cli/tests/daemon_log_wiring.rs`, `crates/pii-cli/tests/daemon_guard_reaps.rs`:

| test | setup | assertion |
|---|---|---|
| `a_worker_state_change_reaches_the_daemon_log_file` | the scratch dir as BOTH `PII_RUNTIME_DIR` and `PII_STATE_DIR`, a daemon holding the operator's real store | the log contains `worker.state.changed` |
| `a_panicking_test_leaves_no_daemon` | same, plus the start assertion | the daemon's log contains `daemon.starting`, then its pid is gone within 5 s |

Both fixture tests must be RED on HEAD while a daemon holds the operator's store; paste that output into the
Closure, then the green run under the same holder.

## Exit Criteria

- [ ] `cargo test -q -p xtask --bins fresh_gate 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-cli --test daemon_log_wiring 2>&1 | grep -qE 'test result: ok\. 1 passed'`
- [ ] `cargo test -q -p pii-cli --test daemon_guard_reaps 2>&1 | grep -qE 'test result: ok\. 2 passed'`
- [ ] `grep -q PII_STATE_DIR crates/pii-cli/tests/daemon_log_wiring.rs && grep -q PII_STATE_DIR crates/pii-cli/tests/daemon_guard_reaps.rs`
- [ ] `grep -q 'daemon.starting' crates/pii-cli/tests/daemon_guard_reaps.rs`

## Guardrails

- Do NOT delete the retention pass: a failed export waits on disk for a person, and `/projects` is at 92 %.
- Do NOT weaken either daemon test; the reaping and log assertions stay exactly as they are.
- Do NOT change `pii_core::state_dir()` or `cmd_daemon.rs`'s resolution: the rule is right, the fixtures are not.
- If a test cannot be made hermetic without changing the daemon, STOP and say so; do not paper over it with a
  timeout or a retry.
- The worker does not commit; the orchestrator reviews and commits.

## Files

- `xtask/src/fresh_gate.rs`
- `crates/pii-cli/tests/daemon_log_wiring.rs`
- `crates/pii-cli/tests/daemon_guard_reaps.rs`

## Closure

Filled by the orchestrator when the item is verified.

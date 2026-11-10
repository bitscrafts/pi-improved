# Spec: Tests off the wall clock

**Project**: <repo> · **Phase**: 0.2.1 hardening · **Source**: Opus optimisation review 2026-11-03,
workstream **W8** — closes finding 11.

## Verified premises

- `crates/pii-server/tests/cron_runtime.rs:75` waits for a real `* * * * *` minute plus a 6 s settle;
  `cancelled_run_disarms.rs` runs a two-minute graph; `daemon/cron.rs:108-110` re-arms through
  `std::thread::sleep`.
- Measured in the 2026-11-03 gate log: those two binaries take **50.5 s** and **67.7 s** — about 110 s per gate.
- The clock is already injectable: `Daemon::build` takes `Arc<dyn Clock>`, and `pii-core/src/clock.rs:33` provides a
  fixed clock.

## Requirements

**R1.** The scheduler's re-arm path takes its delay from the injected clock, not from `std::thread::sleep`.

**R2.** `cron_runtime` and `cancelled_run_disarms` construct the daemon with a fixed clock and advance it explicitly.
Their **assertions** are unchanged: no `assert` line may be deleted or altered, which is what protects the behaviour
being tested. The setup lines around them are expected to change — rewriting the clock is the entire point.

> Corrected 2026-11-03 after the workhorse refused the first version of this criterion: it demanded zero deleted lines
> while R2 requires rewriting the setup, so no implementation could satisfy both. The criterion now forbids deleting an
> assertion instead of deleting a line — the property that actually matters.

**R3.** Both binaries finish in under 5 s.

## The requirement a plausible implementation satisfies in appearance only

A worker can add `#[ignore]` to the slow tests and the suite is fast — while the behaviour they guard is untested.
The criterion requires the assertions to be byte-identical and the binaries to still run their tests.

## TDD contract

| file | assertion |
|---|---|
| `crates/pii-server/tests/cron_runtime.rs` | same assertions, `finished in` < 5 s |
| `crates/pii-server/tests/cancelled_run_disarms.rs` | same assertions, `finished in` < 5 s |
| control | `git diff` of the `assert!(` lines in those two files is empty |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test cron_runtime 2>&1 | grep -qE 'test result: ok.*finished in [0-4]\.'`
- [ ] `cargo test -q -p pii-server --test cancelled_run_disarms 2>&1 | grep -qE 'test result: ok.*finished in [0-4]\.'`
- [ ] `[ "$(git diff -U0 -- crates/pii-server/tests/cron_runtime.rs crates/pii-server/tests/cancelled_run_disarms.rs | grep -c '^-.*assert')" = "0" ]`

## Guardrails

- Do NOT ignore, delete or rename a test.
- Do NOT change any assertion; only the clock supply.
- Do NOT make the daemon's real scheduling depend on a test-only path.
- The worker does not commit; the orchestrator reviews and commits.

## Files

- `crates/pii-server/src/daemon/cron.rs`
- `crates/pii-server/tests/cron_runtime.rs`
- `crates/pii-server/tests/cancelled_run_disarms.rs`

# Spec: a test that spawns a daemon owns its state directory
#: depends=423 status=pending block=-

## Overview
Measured 2026-11-13, and it cost row 423 eight landing attempts: `crates/pii-cli/tests/memory_learn.rs` spawns a daemon
with `PII_RUNTIME_DIR` and `PII_CONFIG` set but **not** `PII_STATE_DIR`, so the daemon resolves its durable stores to
`$HOME/.local/state/pii` — the operator's own. With a real daemon running on that host the test's daemon dies with

```
store /root/.local/state/pii/durable/channel.redb is locked: another daemon holds it
```

and the failure reads as `the daemon never accepted memory.learn`. The gate then fails at `test (lib/bins/tests)` and
takes `coverage-floor` with it, on a tree whose code is fine. Reproduced outside the gate, 2/2, by starting the same
binary by hand.

This is the same defect spec 418 fixed for `--store` from the operator's side: a process given its own store must own
every durable store beside it. Here the caller is a test, and the rule is that a test owns its state.

## Requirements
- R1 Every test that spawns `pii daemon` sets `PII_STATE_DIR` to its own scratch directory (or passes `--store` inside it),
  so no test can open the operator's `$HOME/.local/state/pii`.
- R2 `crates/pii-cli/tests/memory_learn.rs` is fixed by R1 and still passes with a live daemon running on the host.
- R3 A check enforces the rule over the whole tree: `scripts/checks/daemon-tests-isolate-state.sh` finds every test
  that spawns a daemon and reports the ones that do not isolate their state, naming file and line. It exits 0 only when
  none is left, and prints `PASS: <n> FAIL: <m>`.
- R4 The check has controls: a fixture that spawns a daemon without `PII_STATE_DIR` must be reported (FAIL), and a
  fixture that sets it must pass. Both run from `--control`.
- R5 The check is a gate step, so the class cannot come back.
- R6 No test is weakened or skipped to satisfy R3: the isolation is added, the assertions stay.

## Architecture
```
test -> Command::new(pii).arg("daemon")
          .env("PII_RUNTIME_DIR", d)      <- today
          .env("PII_CONFIG", cfg)         <- today
          .env("PII_STATE_DIR", d)        <- what R1 adds
        daemon -> state_dir() = $PII_STATE_DIR (pii-core) -> durable/channel.redb inside the test's dir
```

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `memory_learn_still_records_once` | the fixed test, a live daemon on the host | 2 passed |
| `CONTROL_a_daemon_test_without_state_is_reported` | `--control` fixture, no `PII_STATE_DIR` | the check FAILs and names the fixture line |
| `CONTROL_a_daemon_test_with_state_passes` | `--control` fixture, `PII_STATE_DIR` set | the check passes that fixture |

## Exit Criteria
### New in this row
Verified to FAIL before the change: the check does not exist, and with a daemon running `memory_learn` fails 2/2.
- [ ] `bash scripts/checks/daemon-tests-isolate-state.sh && env -u CARGO_TARGET_DIR -u GATE_STEPS -u PII_BUILD_REVISION cargo test -q -p pii-cli --test memory_learn 2>&1 | grep -qE 'test result: ok\. 2 passed'`

## Guardrails
- Do NOT skip or weaken `memory_learn`'s assertions (R6).
- Do NOT make the daemon refuse to start when its state is a busy directory: the bug is the test's (R1).
- Do NOT change spec 418's `--store` behaviour.
- Do NOT add the check to the gate without `--control` passing (R4).

## Files
- `scripts/checks/daemon-tests-isolate-state.sh`
- `gate-steps.txt`
- `crates/pii-cli/tests/memory_learn.rs`

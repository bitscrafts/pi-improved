# Spec: The gate stops paying for its own leak

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening · **Source**: Opus optimisation review 2026-11-03,
workstream **W1** (findings 1 and 10).

## Verified premises

- `crates/pii-cli/tests/daemon_guard/mod.rs:34-38,82-86` — `Slot::acquire` gives the file to the *first* acquirer;
  later acquirers get `path: None`, and the file is removed only when the dropping slot sees `left == 1`. If the owner
  drops while another test still holds the slot, the file is never removed and the next binary waits for the 180 s
  stale-steal.
- Measured waits in the 2026-11-03 gate log: `memory_learn` **152.89 s**, `skill_cli_reads_through_the_daemon`
  **178.78 s** — together about **330 s of a 634 s** test step.
- The leaked file exists right now: `crates/pii-cli/_tmp/daemon-slot.lock` (0 bytes).
- `crates/pii-channels/tests/poll_backoff.rs:16-18` uses `pii_server::daemon::router`, and
  `crates/pii-channels/Cargo.toml` dev-depends on `pii-server`, `pii-learn` and `pii-tools`, so testing channels
  compiles the 209-entry server tree.

## Requirements

**R1.** The slot's file is owned by a process-wide static owner count; the file is removed when that count reaches
zero, regardless of which test was the first acquirer.

**R2.** A test binary that finds a stale file whose owning process is gone steals it immediately rather than waiting
180 s.

**R3.** The channel backoff test moves to `crates/pii-server/tests/` and `pii-channels` drops the `pii-server`,
`pii-learn` and `pii-tools` dev-dependencies; its assertions are unchanged (diff of the `assert!` lines is empty).

**R4.** `cargo xtask test-inventory` prints the number of test binaries and test functions, and `--check` fails if the
count drops below the committed baseline — so the large deletions in W7 cannot silently delete tests.

## The requirement a plausible implementation satisfies in appearance only

A worker can shorten the timeout constant and the waits disappear — while the leak remains and the suite simply fails
faster when it hits it. The test below acquires, drops in the wrong order, and asserts the file is GONE.

## TDD contract

`crates/pii-cli/tests/daemon_guard_holds.rs`:

| test | assertion |
|---|---|
| `the_slot_file_is_removed_when_the_last_holder_drops` | acquire A, acquire B, drop A, drop B → the file does not exist |
| `CONTROL_a_second_acquire_while_held_waits` | acquire A, acquire B while A holds it → B blocks (the lock still means something) |

## Exit Criteria

- [ ] `cargo test -q -p pii-cli --test daemon_guard_holds 2>&1 | grep -qE 'test result: ok. 2 passed'`
- [ ] `test ! -e crates/pii-cli/_tmp/daemon-slot.lock`
- [ ] `grep -c 'pii-server' crates/pii-channels/Cargo.toml | grep -q '^0$'`
- [ ] `cargo run -q -p xtask -- test-inventory --check`

## Guardrails

- Do NOT weaken the slot: it exists so two daemon tests never share a runtime dir.
- Do NOT raise the 180 s stale-steal as a "fix".
- Do NOT delete the channel backoff test; move it and keep every assertion.
- Test counts must not drop (R4).
- The worker does not commit; the orchestrator reviews and commits.

## Files

- `crates/pii-cli/tests/daemon_guard/mod.rs`
- `crates/pii-cli/tests/daemon_guard_holds.rs` (new)
- `crates/pii-channels/Cargo.toml`
- `crates/pii-channels/tests/poll_backoff.rs`
- `crates/pii-server/tests/poll_backoff.rs`
- `xtask/src/main.rs`, `xtask/src/test_inventory.rs` (new)

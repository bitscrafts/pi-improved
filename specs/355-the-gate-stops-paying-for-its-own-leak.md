# Spec: The gate stops paying for its own leak

**Project**: <repo> · **Phase**: 0.2.1 hardening · **Source**: Opus optimisation review 2026-11-03,
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

## Closure

Verified 2026-11-03 by the orchestrator on the tree the worker left (session `514a1eff`, worker rc=0).

**Criteria, run by hand on that tree:**

- `cargo test -q -p pii-cli --test daemon_guard_holds` → `test result: ok. 2 passed; 0 failed`
- `test ! -e crates/pii-cli/_tmp/daemon-slot.lock` → the file was absent. It **existed** when the row was
  dispatched (a 0-byte leak), and the reworked `Drop` removed it.
- `grep -c 'pii-server' crates/pii-channels/Cargo.toml` → `0`
- `cargo run -q -p xtask -- test-inventory --check` → `PASS: test-inventory (285 binaries >= 284, 2073 functions >= 2071)`

**Nothing was deleted to make it pass.** The channel backoff test moved:
`diff <(git show HEAD:crates/pii-channels/tests/poll_backoff.rs) crates/pii-server/tests/poll_backoff_outage.rs`
printed nothing — byte-identical, so no assertion left with the move. Spec 331's two guard tests moved from
`daemon_guard_holds.rs` to `daemon_guard_reaps.rs` the same way (`diff` empty). The test count rose by 2 functions
and 1 binary.

**Gate:** `fresh-gate --worktree` on this tree, 12:35–12:47Z, `quality-gate: ALL PASS` / `fresh-gate: PASS`
(20 steps; log `/tmp/gate-355e.log`, export `fresh-gate-197287`). Committed as `1c6b886`.

**Three reds seen while getting there were not this item's — all three became work and one leaked:**

1. `the_queue_graph_loads_itself_as_a_called_graph` failed at HEAD: spec 362's `dispatch` node declares
   `fs_read,fs_write`, and `graph.call.widen` refuses a call that grants less, while the test's host still granted
   `spawn,proc`. `graphs/night.toml` already granted them. Repaired in `crates/pii-server/tests/queue_in_a_call.rs`
   (fixture only, assertion untouched) as `eb48bdb`; the test then passed 3/3.
2. `a_worker_state_change_reaches_the_daemon_log_file` failed at 12:08Z with
   `store ~/.local/state/pii/durable/channel.redb is locked: another daemon holds it`. The test spawns a real
   daemon with `PII_RUNTIME_DIR` but not `PII_STATE_DIR`, so it claimed the operator's live durable store — the
   gate was red for as long as any daemon ran. Filed as spec 365.
3. At 12:22Z **my own gate was killed by another gate's retention pass**: `stale_exports` removes every other
   `fresh-gate-*` directory with no liveness test, so a second gate deletes a running one's export. Sixteen steps
   then failed on `target/gate-steps/*.log: No such file or directory` with `failing tests: []` — a red that
   blames the tree for a directory another process removed. Filed as spec 365 (R1/R2).

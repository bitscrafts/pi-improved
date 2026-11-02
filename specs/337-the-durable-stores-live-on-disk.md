# Spec: The durable stores live on disk, not on tmpfs

## Overview

Two stores promise durability in their own documentation, and both live in the runtime directory:

```
crates/pii-store/src/channel.rs:1-4   "a crash cannot duplicate or lose a message"
crates/pii-store/src/timer.rs        "durable timer wheel"
crates/pii-server/src/daemon/mod.rs:201,204   both built from `runtime_dir`

$ ssh mvcorrea@lnx 'findmnt -no FSTYPE /run/user/1000'
tmpfs
```

So on the host a **reboot** — not a crash — empties the channel cursor (messages re-processed or lost) and the
timer wheel (suspended runs forgotten). Measured 2026-11-02: the host has been up since 2026-09-16, so no install
has ever been through one; the defect is invisible until the first reboot, which is the worst possible time.

**Project**: /projects/pi-improved
**Phase**: production hardening (Opus audit, M2's unexamined half)

## Requirements

**R1 — durable stores resolve under the state directory.** The channel and timer stores are built from the
**state** directory (`~/.local/state/pii` on the host), not from `runtime_dir` (`/run/user/1000/pii`). The runtime
directory keeps the control socket and anything else that is genuinely per-boot. The path resolution has one
implementation and both call sites use it.

**R2 — it is configurable, and the default is the safe one.** A new `[core] durable_dir` (optional) overrides the
location; when unset it is `<state_dir>/durable`. It is **not** defaulted to the runtime dir under any condition.

**R3 — a restart keeps the cursor.** A test starts a daemon-shaped process with the resolved configuration,
records a message id in the channel store, stops it, starts it again, and asserts the cursor is the same. This is
the property, and the path assertion in R1 is not a substitute for it.

**R4 — the path is observable.** `pii health` reports the two store paths, so the probe from spec 335 can assert
they are not under `/run/`.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `the channel store is not under the runtime dir` | resolved config with `runtime_dir=/run/user/1000/pii`, `state=…/state/pii` | path starts with the state dir, not `/run` |
| `the timer store is not under the runtime dir` | the same | the same |
| `a restart keeps the cursor` | write a cursor, stop, start | the same cursor; no message re-processed |
| `control: the old placement loses it` | the same with `runtime_dir` used as before, and the directory removed between runs | the cursor is gone (this is the defect, measured) |
| `[core] durable_dir` overrides | a config naming a directory | both stores there |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test durable_store_placement 2>&1 | grep -qE 'test result: ok\. [3-9] passed'`
- [ ] `grep -q 'durable_dir' crates/pii-server/src/daemon/config.rs`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q 'fresh-gate: PASS'`

## Guardrails

- Do NOT move the control socket: a socket on tmpfs is correct.
- Do NOT migrate an existing store in place; a new location is created empty, and the old file is left where it is
  for the operator to retire. No deletion.
- Do NOT change the store format or `format_version`.
- The dev daemon in the container must keep working unchanged (`pii health` after a restart, same paths reported).

## Files

- `specs/337-the-durable-stores-live-on-disk.md`
- `crates/pii-server/src/daemon/config.rs`
- `crates/pii-server/src/daemon/identity.rs`
- `crates/pii-server/src/daemon/mod.rs`
- `crates/pii-server/src/daemon/router.rs`
- `crates/pii-server/src/daemon/status.rs`
- `crates/pii-server/src/engine/inline_measure_tests.rs`
- `crates/pii-server/tests/cancel_disarms_the_timer.rs`
- `crates/pii-server/tests/cancel_kill.rs`
- `crates/pii-server/tests/cancelled_run_disarms.rs`
- `crates/pii-server/tests/channel_adapters.rs`
- `crates/pii-server/tests/cron_arming_is_idempotent.rs`
- `crates/pii-server/tests/cron_runtime.rs`
- `crates/pii-server/tests/daemon_graphs.rs`
- `crates/pii-server/tests/durable_store_placement.rs`
- `crates/pii-server/tests/entities_e2e.rs`
- `crates/pii-server/tests/hal_e2e.rs`
- `crates/pii-server/tests/health_identity.rs`
- `crates/pii-server/tests/log_wiring.rs`
- `crates/pii-server/tests/mqtt_e2e.rs`
- `crates/pii-server/tests/operator_surfaces.rs`
- `crates/pii-server/tests/poll_backoff.rs`
- `crates/pii-server/tests/post_e2e.rs`
- `crates/pii-server/tests/tiering_e2e.rs`
- `crates/pii-server/tests/timer_no_refire.rs`
- `crates/pii-server/tests/wasm_e2e.rs`

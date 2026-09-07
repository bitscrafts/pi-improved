# pi-improved — spec-18: phase4-service

- **Project**: `/projects/pi-improved`
- **Priority**: P4.5 — the difference between "survives a file-level recreate" and "survives a rebuild"; a day of work per the opus plan.
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
  on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
  commits and updates HANDOFF.md)*
- **Source**: Phase 4.5 - make it a service (2026-09-07)
- **Depends-On**: none.

---

## Verified Premises

- `crates/pii-cli/src/cmd_daemon.rs:17-60` — `pii daemon` parses `--workspace`, `--store`, `--config`, `--sandbox`. There is NO `--foreground` flag (the daemon already runs in the foreground; a supervisor needs an explicit signal).
- `crates/pii-cli/src/cmd_daemon.rs:282-299` — the daemon exits 0 on clean SIGTERM/SIGINT, 130 on a forced second signal. This is already supervisor-friendly.
- `crates/pii-cli/src/cmd_ctl.rs:60-71` — `print_result` returns exit 0 on success, 2 on error. `pii sys status` returns 0 when the daemon is reachable, 2 when not — a usable healthcheck exit code already.
- `/root/.pii/start-daemon.sh` — exists (installed 2026-09-05) but is NOT wired into the container's `entrypoint.sh` (its own header comment says so). A file-level container recreate survives; a full rebuild that re-runs the entrypoint does not.
- `crates/pii-cli/src/cmd_daemon.rs:64-81` — the daemon resolves `runtime_dir` and `store_path`; `--foreground` would be a no-op flag today (the daemon is already foreground).

---

## Overview

The opus Phase-4 review's **P4.5**: *"systemd user unit (or the container `entrypoint.sh` hook), `pii daemon --foreground` semantics for a supervisor, log rotation, meaningful exit codes from `pii sys status` for a healthcheck. The difference between 'survives a file-level recreate' and 'survives a rebuild'; a day of work."*

This spec makes Pii a proper service:
1. `pii daemon --foreground` — an explicit flag (currently a no-op since the daemon is already foreground, but it documents the contract and future-proofs a daemonize path).
2. `pii sys status` exit codes — already 0/2 (reachable/unreachable); document them as the healthcheck contract.
3. Wire `start-daemon.sh` into the container `entrypoint.sh` so the daemon survives a full rebuild.
4. Log rotation — the daemon writes to `daemon.log`; add a simple rotation (size-triggered) or document the supervisor's rotation.

---

## Requirements

### Functional

- **R1 (`--foreground` flag)**: `pii daemon` accepts `--foreground` (explicit no-op today — the daemon is already foreground; the flag documents the contract and future-proofs a daemonize path). Unknown-flag handling already errors (cmd_daemon.rs:55).
- **R2 (healthcheck exit codes)**: `pii sys status` returns exit 0 when the daemon is reachable, non-zero (2) when not. This is already true (cmd_ctl.rs:60-71); add a test asserting it and document it as the healthcheck contract.
- **R3 (entrypoint wiring)**: add a line to the container's `entrypoint.sh` that calls `start-daemon.sh` (backgrounded, with a health-check loop) so the daemon survives a full container rebuild. The script already exists and is idempotent.
- **R4 (log rotation)**: add a simple size-triggered rotation to `start-daemon.sh` (or document the supervisor's rotation) — the daemon appends to `daemon.log` unboundedly today.

### Non-Functional

- **N1**: no change to the daemon's runtime behavior — `--foreground` is a documented no-op, not a behavior change.
- **N2**: the quality gate stays green; the test count may only go up.
- **N3**: `xtask` gates stay green.

---

## Architecture

```
entrypoint.sh (container)
   |
   v
start-daemon.sh (idempotent, health-check loop)
   |
   v
pii daemon --foreground  (already foreground; flag documents the contract)
   |
   v
pii sys status  (exit 0 = reachable, 2 = unreachable)  -- healthcheck
```

**Key decision — `--foreground` is a documented no-op, not a daemonize change.** The daemon is already foreground (it must be, for a supervisor to manage it). Adding a real daemonize path (fork + setsid) would complicate signal handling for zero benefit in a container. The flag documents the contract and future-proofs.

**Key decision — the healthcheck is `pii sys status`'s existing exit code.** It already returns 0/2. No new endpoint needed; just test + document it.

**What this spec is not**: systemd units (the container is the supervisor), TLS, multi-user.

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R1 | `daemon_accepts_foreground_flag` (pii-cli unit) | `pii daemon --foreground` | parses without error (the flag is accepted) |
| R2 | `sys_status_exit_codes` (pii-cli integration) | a live daemon, then a dead one | `pii sys status` exits 0 when reachable, 2 when not |
| R3 | `start_daemon_is_idempotent` (shell check) | run `start-daemon.sh` twice | second run detects the running instance and does not double-start |
| R4 | `daemon_log_rotates` (shell check) | a log over the rotation size | the log is rotated (old content preserved, new file started) |

**R2 (`sys_status_exit_codes`) is the acceptance test.** The requirement a plausible implementation gets wrong: it adds a new healthcheck endpoint instead of using the existing `sys.status` exit code, or it changes the exit code semantics. The test pins the existing 0/2 contract.

---

## Exit Criteria

- [ ] `cargo build --workspace --all-targets` — compiles (R1)
- [ ] `cargo test -p pii-cli daemon_accepts_foreground` — flag accepted (R1)
- [ ] `cargo test -p pii-cli sys_status_exit` — healthcheck exit codes (R2)
- [ ] `cargo test --workspace` — 0 failures (N2)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N2)
- [ ] `cargo fmt --all --check` — clean (N2)
- [ ] `cargo xtask lint-files` — 0 hard errors (N2)
- [ ] Manual: `start-daemon.sh` runs idempotently; `pii sys status` exits 0; the entrypoint line is present

**Prose criteria**:

1. Test counts pasted raw, one line per binary, **unsummed**.
2. The `--foreground` flag must be accepted without changing daemon behavior (a no-op, not a daemonize).

---

## Guardrails

- **G1 — do NOT edit this spec.** If it is wrong, STOP and report it.
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT regenerate a pinned fixture.**
- **G5 — no hardcoded absolute paths.** Test artefacts under `_tmp/`.
- **G6 — report raw output, never summed totals.**
- **G7 — do NOT add a real daemonize path** (fork + setsid). The daemon stays foreground; `--foreground` is a documented no-op.
- **G8 — do NOT change the `sys.status` exit-code semantics** — 0 reachable / 2 unreachable is the contract.

### Error handling expectations

- `start-daemon.sh` must be idempotent — a second run detects the running instance and does not double-start.
- Log rotation must preserve the old log (rename, not truncate).

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/pii-cli/src/cmd_daemon.rs` | accept `--foreground` (R1) |
| `crates/pii-cli/src/cmd_sys.rs` | (test only) assert exit codes (R2) |
| `/root/.pii/start-daemon.sh` | log rotation (R4) |
| container `entrypoint.sh` | wire `start-daemon.sh` (R3) |

**Not modified**: `crates/pii-server/` (no daemon behavior change), `crates/pii-proto/` (no wire change).

# Spec: The daemon's default store is the state dir too

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening · **Found**: 2026-11-03 while closing 351
**Blocks**: the night run (a row seeded into the live store must be the store the daemon claims)

## Verified premises (measured 2026-11-03, on this tree)

- 351 moved every CLI **store** default onto `pii_core::state_dir()`; `pii backup` now reads
  `$HOME/.local/state/pii/store.redb` by default (`crates/pii-cli/src/cmd_backup.rs`, criteria green in `1501b8c`
  lineage).
- The **daemon's** own default did not move: `crates/pii-cli/src/cmd_daemon.rs:54` still reads
  `let store_path = store.unwrap_or_else(|| runtime_dir.join("store.redb"));`
  — command: `grep -n 'store.unwrap_or_else' crates/pii-cli/src/cmd_daemon.rs` → line 54, `runtime_dir.join`.
- Its help text says so out loud: `crates/pii-cli/src/cmd_daemon_flags.rs:17`
  `--store <path>   redb store file (default: <runtime dir>/store.redb)`.
- So after 351 the two halves disagree: a daemon started with no `--store` writes to the **boot-scoped runtime
  dir** (`/run/user/<uid>/pii/store.redb`, wiped on reboot) while `pii backup` reads the **state dir**
  (`~/.local/state/pii/store.redb`). A backup of a bare daemon therefore copies a file the daemon is not writing.
- Production is not affected *today* only because the install writes an explicit store; the DEFAULT is the defect,
  and it is the same class R3 of 351 fixed for the CLI.

## Requirements

**R1.** `pii daemon`'s default store is `pii_core::state_dir().join("store.redb")`. An explicit `--store <path>`
(or the config's key) is unchanged and still wins.

**R2.** Only the store default moves. The socket, the graphs dir, the sandbox's grant list and the log/trajectory
dirs keep `runtime_dir` exactly as they are — a socket is per-boot, a store is durable.

**R3.** The `--help` text states the new default and names the state dir, not the runtime dir.

**R4.** The test drives the live path, not the helper: it starts the real `pii daemon` with `PII_STATE_DIR=<scratch>`
and no `--store`, and asserts the store file appears under `<scratch>` and that no `store.redb` is created under
`PII_RUNTIME_DIR`. The daemon is owned by the repo's `DaemonGuard` (spec 331).

## The requirement a plausible implementation satisfies in appearance only

A worker can change line 54 and leave the help text saying `<runtime dir>`, or add a `default_store()` helper and
call it only from the sandbox grant list, and report success because "the default is centralised". The feature is
that **a bare `pii daemon` writes the store `pii backup` reads**. Criterion 3 checks the help text; criterion 1
checks the file the live daemon creates.

## TDD contract — `crates/pii-cli/tests/daemon_store_default.rs`

| test | setup | assertion |
|---|---|---|
| `a_bare_daemon_writes_the_store_in_the_state_dir` | `PII_STATE_DIR=<S>`, `PII_RUNTIME_DIR=<R>`, no `--store`; start `pii daemon` under `DaemonGuard` and wait for its socket | `<S>/store.redb` exists and is non-empty; `<R>/store.redb` does not exist |
| `CONTROL_an_explicit_store_still_wins` | same, plus `--store <T>/store.redb` | `<T>/store.redb` exists; `<S>/store.redb` does not |
| `CONTROL_the_socket_stays_in_the_runtime_dir` | same as the first | the socket the daemon answered on is under `<R>` |

All three must be RED (or the first two) before the change; paste the failing output into the Closure.

## Exit Criteria

- [ ] `cargo test -q -p pii-cli --test daemon_store_default 2>&1 | grep -q 'test result: ok\. 3 passed'`
- [ ] `[ "$(grep -c 'runtime_dir.join("store.redb")' crates/pii-cli/src/cmd_daemon.rs)" = "0" ]`
- [ ] `! grep -q 'default: <runtime dir>/store.redb' crates/pii-cli/src/cmd_daemon_flags.rs`
- [ ] `[ "$(grep -c 'pii_core::state_dir()' crates/pii-cli/src/cmd_daemon.rs)" -ge 2 ]`

## Guardrails

- Do NOT move the socket, the graphs dir or the sandbox grant list out of `runtime_dir`.
- Do NOT read config in `pii-core`; the precedence stays `--store` → config → `state_dir()`.
- Do NOT change the store file name or the durable layout.
- The gate is the cycle's step, not the implementer's: `cycle.sh` gates this commit with the full list and a
  measured coverage floor (specs 262/266/315), and a sandboxed implementer cannot write `/projects/_tmp` where
  `fresh-gate` exports. Do not add it as a criterion (measured 2026-11-03, row 351).
- The worker does not commit.

## Files

- `crates/pii-cli/src/cmd_daemon.rs` (R1, R2)
- `crates/pii-cli/src/cmd_daemon_flags.rs` (R3)
- `crates/pii-cli/tests/daemon_store_default.rs` (new, R4)

## Closure

_To be filled when the item is reviewed._

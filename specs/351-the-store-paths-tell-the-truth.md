# Spec: The store paths tell the truth

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening · **Finding**: Opus optimisation #2 (2026-11-03)
**Blocks**: the night run (a row seeded into the live store must be the row the daemon claims)

## Verified premises (measured 2026-11-03, on this tree)

- `pii_uds::runtime_dir("default")` is used as the **store** default in nine places across seven CLI files:
  `cmd_backup.rs` (1), `cmd_store.rs` (1), `cmd_skill.rs` (2), `cmd_entity.rs` (1), `cmd_learn.rs` (1),
  `graph_dirs.rs` (1), `cmd_graph_installed.rs` (2).
  Command: `for f in …; do grep -c runtime_dir $f; done` → `1 1 2 1 1 1 2`.
- The daemon resolves state differently: `PII_STATE_DIR` first, then the config (`daemon/mod.rs:207`).
- That rule is **copied three times**: `daemon/mod.rs:207`, `cmd_treat.rs:53`, `cmd_daemon.rs:145`.
- On the production host `/run/user/1000/pii/store.redb` is **561,152 B dated Oct 24** while the live store is
  `~/.local/state/pii/store.redb` at **18,812,928 B**. An ssh shell has no `PII_*` env, so `pii backup <p>`
  without `--dir` copies the STALE set. (Measured by `stat` on lnx.)
- In this container `/run/user/0/pii/` likewise holds a stale `channel.redb` (Nov 2) and no daemon answers.
- The socket is a different thing from state: it lives in `runtime_dir` and MUST stay there.

## Requirements

**R1.** One function owns the rule: `pii_core::state_dir() -> std::path::PathBuf` —
`PII_STATE_DIR` (via the crate's existing env accessor) if set and non-empty, else `$HOME/.local/state/pii`.
Empty string counts as unset. No other crate re-implements it.

**R2.** `daemon/mod.rs` uses `pii_core::state_dir()`; `cmd_treat.rs:51-55` and `cmd_daemon.rs:145` each call it
instead of their private copy. The three copies become one call site each.

**R3.** The nine CLI **store** defaults use `pii_core::state_dir()` (joined with the store's file name), not
`pii_uds::runtime_dir`. The socket paths keep `pii_uds::runtime_dir` exactly as they are.

**R4.** `pii store` and `pii backup` gain no new flags; their behaviour with `--dir`/`--store` given explicitly is
unchanged. This spec changes only the DEFAULT.

## The requirement a plausible implementation satisfies in appearance only

A worker can add `pii_core::state_dir()` and call it from one file, leave the nine CLI defaults untouched, and
report success because "the rule is centralised". The feature is that **a default backup reads the live store**.
The test below is written against the artefact (`pii backup` output manifest) and fails on HEAD; a green suite of
unit tests on the helper alone proves nothing.

## TDD contract — `crates/pii-cli/tests/store_dir_default.rs`

| test | setup | assertion |
|---|---|---|
| `a_backup_default_reads_the_state_dir_not_the_runtime_dir` | `PII_RUNTIME_DIR=A` (empty), `PII_STATE_DIR=B` holding a `store.redb` with known bytes; run `pii backup <out>` with no `--dir` | the manifest's `store.redb` hash equals the blake3 of `B/store.redb`; the exit code is 0 |
| `CONTROL_an_empty_state_dir_env_falls_back_to_home` | `PII_STATE_DIR=""`, `HOME=<C>` with `C/.local/state/pii/store.redb` | the backup reads `C/.local/state/pii/store.redb` |
| `CONTROL_the_socket_still_comes_from_the_runtime_dir` | `PII_RUNTIME_DIR=A`, `PII_STATE_DIR=B`, no daemon | `pii health` reports the socket it tried under `A`, not under `B` (the failure text names `A`) |

All three must be RED before the change. Measure and paste the failing output into the Closure.

## Exit Criteria

- [ ] `cargo test -q -p pii-cli --test store_dir_default 2>&1 | grep -q 'test result: ok\. 3 passed'`
- [ ] `[ "$(grep -rc 'runtime_dir' crates/pii-cli/src/cmd_backup.rs) " != "1" ]`
- [ ] `[ "$(grep -rn 'fn state_dir' crates/pii-core/src | wc -l)" = "1" ]`
- [ ] `[ "$(grep -rn 'PII_STATE_DIR' crates/pii-server/src/daemon/mod.rs crates/pii-cli/src/cmd_treat.rs crates/pii-cli/src/cmd_daemon.rs | grep -c 'env_var\|env(')" = "0" ]`
- [ ] `cargo xtask fresh-gate`

## Guardrails

- Do NOT move the socket out of `runtime_dir`; a per-boot socket is correct.
- Do NOT change the precedence order (`PII_STATE_DIR` wins) or introduce a config lookup into `pii-core`.
- Do NOT add a flag; the defect is the default.
- Do NOT touch the store file names or the durable layout.
- The worker does not commit.

## Files

- `crates/pii-core/src/lib.rs` (or a new `crates/pii-core/src/state.rs`, registered from `lib.rs`)
- `crates/pii-server/src/daemon/mod.rs` (R2)
- `crates/pii-cli/src/cmd_treat.rs`, `crates/pii-cli/src/cmd_daemon.rs` (R2)
- `crates/pii-cli/src/cmd_backup.rs`, `cmd_store.rs`, `cmd_skill.rs`, `cmd_entity.rs`, `cmd_learn.rs`,
  `graph_dirs.rs`, `cmd_graph_installed.rs` (R3, store defaults only)
- `crates/pii-cli/tests/store_dir_default.rs` (new)

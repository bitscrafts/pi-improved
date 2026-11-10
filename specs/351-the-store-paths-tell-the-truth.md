# Spec: The store paths tell the truth

**Project**: <repo> · **Phase**: 0.2.1 hardening · **Finding**: Opus optimisation #2 (2026-11-03)
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
  without `--dir` copies the STALE set. (Measured by `stat` on the host.)
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

## Closure

Closed 2026-11-03 by the orchestrator on the worker's tree. The worker's three attempts did not fail on the code:
they failed on criterion 5, which the implementer cannot run (below). The code criteria were re-run here, the
first full gate of the tree was RED on two consequences of the change, both were repaired, and the gate then passed.

**The four code criteria, run by the orchestrator (`PATH` with `cargo`):**

| criterion | result |
|---|---|
| `cargo test -q -p pii-cli --test store_dir_default` | `test result: ok. 3 passed` |
| `[ "$(grep -rc 'runtime_dir' crates/pii-cli/src/cmd_backup.rs) " != "1" ]` | rc 0 (count 0) |
| `[ "$(grep -rn 'fn state_dir' crates/pii-core/src \| wc -l)" = "1" ]` | rc 0 (count 1) |
| `PII_STATE_DIR` env reads in `daemon/mod.rs` + `cmd_treat.rs` + `cmd_daemon.rs` | rc 0 (count 0) |

**Why the worker parked (not a code defect).** Criterion 5 is `cargo xtask fresh-gate`. The implementer runs
inside the daemon's sandbox, whose read-write set is the workspace plus the store parent and runtime dir; the gate
exports its tree to `<scratch>`, outside all of them. The worker recorded it verbatim — "This process cannot
write there" — and all three runs refused on that criterion alone (4 of 5 green each time). The gate is the
cycle's step (specs 262/266/315 gate every queue commit with the full step list and a measured coverage floor);
it is no longer an exit criterion in 352/353, and the reason is written in their Guardrails.

**The first full gate of this tree was RED on two steps — both real, both consequences of the change:**

1. `test (lib/bins/tests)`: three `channel_wiring` tests failed together with
   `channel: open ~/.local/state/pii/durable/channel.redb: Database already open`.
   `daemon/mod.rs` resolved the state dir from the process-global `pii_core::state_dir()`, while `build_with_env`
   carries an INJECTED env: the tests' own scratch `PII_STATE_DIR` was ignored, so they opened the operator's live
   store and collided with the running daemon. Fixed by `pii_core::state_dir_from(env)`, with `state_dir()`
   delegating to it; `cargo test -p pii-server --test channel_wiring` → 4 passed.
2. `deps-check`: invariant 5 — `pii-cli` depends on `redb` outside `pii-store` (the new test opened its fixture
   with `redb::Database::create`). The test now opens it through `pii_store::Store` and the dev-dependency is gone;
   `cargo run -p xtask -- deps-check` → rc 0.

**The gate after the repair:** `cargo xtask fresh-gate --worktree` → `fresh-gate: PASS`, every step green
(fmt · clippy · clippy hal-gpiod · build · test lib/bins/tests · test doc · lint-files · lint-scratch ·
lint-observability · reachability-check · deps-check · net-check · doc-drift-check · autonomy shell tests ·
plan-verify · item-ids · devbox-check · install-config · daemon-guard). Then, on the committed HEAD `8c32624`,
the plain `cargo xtask fresh-gate` — criterion 5 exactly as written — → `fresh-gate: PASS`, 51 steps, rc 0.

**Coverage-floor was NOT measured in this pass**, stated rather than implied: `fresh-gate` skips it by default,
and the `FRESH_GATE_COVERAGE=1` run was killed at `/projects` 100% (2.9 GB free on a 594 GB volume) with 56 GB of
instrumented build behind it. The volume was reclaimed (`cargo clean`, and this run's own export removed) and the
reading is not recorded as a pass. The next queue commit measures it under the cycle's gate.

**Found while closing, queued as spec 354 / row 354:** `crates/pii-cli/src/cmd_daemon.rs:54` still defaults the
daemon's own store to `runtime_dir.join("store.redb")` (help text included), so after this change a bare
`pii daemon` writes the boot-scoped path while `pii backup` reads the state dir. That is the same class this spec
closed for the CLI, and it is not covered by R1–R4 here.

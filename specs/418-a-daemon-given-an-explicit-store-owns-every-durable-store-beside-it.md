# Spec: a daemon given an explicit --store owns every durable store beside it and never opens the operator's
#: depends=416 status=pending block=-

## Overview
Measured 2026-11-12 while running the first end-to-end test of the `github` channel: a throwaway daemon started with `--store "$D/store.redb" --workspace "$D"`
exited with `error: internal: channel: open /root/.local/state/pii/durable/channel.redb: I/O error: Permission denied (os error 13)`. `--store` moves
the run store; the durable `channel.redb` and `timer.redb` still came from `$HOME/.local/state/pii/durable` (`pii_core::state::state_dir_from`, spec 351),
so the test daemon reached for the operator's channel store. It only failed loudly because that file was busy: on a quiet machine the test instance would have
opened the operator's state and advanced its channel cursors. The test only worked after it also set `PII_STATE_DIR` — a variable a user of `--store` has no
reason to know about.

`Config::resolve_durable_dir(state_dir)` (`crates/pii-server/src/daemon/config.rs:608`) is the single rule: `[core].durable_dir` if set, else
`<state dir>/durable`, and the state dir is `PII_STATE_DIR` or `$HOME/.local/state/pii`. Nothing in that chain knows the daemon was given an explicit store.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 416

## Requirements
- R1 A daemon started with an **explicit** `--store <path>` and no `PII_STATE_DIR` keeps its durable stores beside that store: the durable directory is
  `<parent of <path>>/durable`. Both `channel.redb` and `timer.redb` land there, and nothing under `$HOME/.local/state/pii` is created, opened or modified.
- R2 `PII_STATE_DIR` still wins over the store flag, and `[core].durable_dir` still wins over both: the existing precedence (spec 337/351) is unchanged.
- R3 A daemon with **no** `--store` keeps today's behaviour exactly: the durable stores stay under the resolved state dir, never under the runtime dir
  (spec 337: the runtime dir is a tmpfs a reboot empties).
- R4 The rule lives in one function in `pii_core::state` (`state_dir_for_store(env, store: Option<&Path>) -> PathBuf`, or the same idea) so the daemon and
  every CLI command that reports on the stores resolve it identically; `pii daemon --help` and the `--store` line say where the durable stores go.
- R5 The store's own path is unchanged in all cases: `--store` still names the run store, and a missing parent directory is created as today.

## Architecture
```
--store Some(p) + PII_STATE_DIR unset   -> state dir = p.parent()      -> durable = p.parent()/durable
PII_STATE_DIR set                       -> state dir = PII_STATE_DIR   -> durable = that/durable
--store absent                          -> state dir = $HOME/.local/state/pii -> durable = that/durable
[core].durable_dir set                  -> durable = that (wins over all)
```
Key decision: an explicit `--store` means "a separate instance", so everything the instance must own moves with it. Rejected: making the default store path
imply the same (it lives in the runtime dir, and spec 337 forbids durable state there). Rejected: requiring `PII_STATE_DIR` (a second flag for one
intention, and the flag that the failure was hidden behind).

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `an_explicit_store_puts_the_durable_dir_beside_it` | env without `PII_STATE_DIR`, `--store /tmp/x/s.redb` | `<state dir for the stores>` = `/tmp/x`, so `durable` = `/tmp/x/durable` |
| `CONTROL_pii_state_dir_still_wins_over_the_store` | both set | the `PII_STATE_DIR` path |
| `CONTROL_no_store_uses_the_home_state_dir` | neither | `$HOME/.local/state/pii` |
| `CONTROL_a_configured_durable_dir_wins_over_both` | `[core].durable_dir` set | that path |
| `CONTROL_an_empty_pii_state_dir_is_not_a_value` | `PII_STATE_DIR=""`, store set | the store's parent (today's empty check is kept) |

Acceptance: the four pass; each of these mutations makes at least one fail (the mutation script is the orchestrator's): the store branch removed; the
`PII_STATE_DIR` branch ordered after the store branch; the empty-string guard dropped.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-core state:: 2>&1 | grep -qE 'test result: ok. [4-9] passed'`

## Guardrails
- Keep green, unedited: `cargo test -q -p pii-server --test channel_wiring` and `cargo test -q -p pii-cli --test daemon_start_mark`.
- Do NOT change where `--store` itself points, and do NOT make the runtime dir hold durable state.
- Do NOT require a new environment variable for the ordinary path: a daemon with no flags behaves exactly as today.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-core/src/state.rs`
- `crates/pii-server/src/daemon/mod.rs`
- `crates/pii-cli/src/cmd_daemon.rs`
- `crates/pii-cli/src/cmd_daemon_flags.rs`

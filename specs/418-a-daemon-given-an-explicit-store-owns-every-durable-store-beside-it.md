# Spec: a daemon given an explicit --store owns every durable store beside it and never opens the operator's
#: depends=417 status=pending block=-

## Overview
Measured 2026-11-12 while running the first end-to-end test of the `github` channel: a throwaway daemon started with `--store "$D/store.redb" --workspace "$D"`
exited with `error: internal: channel: open /root/.local/state/pii/durable/channel.redb: I/O error: Permission denied (os error 13)`. `--store` moves
the run store; the durable `channel.redb` and `timer.redb` still came from `$HOME/.local/state/pii/durable` (`pii_core::state::state_dir_from`, spec 351),
so the test daemon reached for the operator's channel store. It only failed loudly because that file was busy: on a quiet machine the second instance would
have opened the operator's state and advanced its channel cursors. The test only worked after it also set `PII_STATE_DIR` — a variable a user of `--store`
has no reason to know about.

`Config::resolve_durable_dir(state_dir)` is the one rule: `[core].durable_dir` if set, else `<state dir>/durable`, where the state dir is `PII_STATE_DIR`
or `$HOME/.local/state/pii`. The configuration ALREADY has the override this needs; nothing in the chain knows the daemon was given an explicit store.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 417

## Requirements
- R1 A daemon started with an **explicit** `--store <path>` and no `PII_STATE_DIR` keeps its durable stores beside that store: the effective
  `[core].durable_dir` becomes `<parent of <path>>/durable`, so `channel.redb` and `timer.redb` land there and nothing under `$HOME/.local/state/pii`
  is created, opened or modified.
- R2 `PII_STATE_DIR` still wins over the store flag, and a `[core].durable_dir` written in the configuration file still wins over both: the operator's
  explicit configuration is never overridden by a command-line default.
- R3 A daemon with **no** `--store` keeps today's behaviour exactly: the durable stores stay under the resolved state dir, never under the runtime dir
  (spec 337: the runtime dir is a tmpfs a reboot empties).
- R4 The translation happens ONCE, in the CLI, through the existing `[core].durable_dir` override — no new precedence rule inside the daemon, and no new
  environment variable. It is a pure function (flag + environment in, `Option<PathBuf>` out) so it is testable without starting anything, and `pii daemon
  --help`'s `--store` line says where the durable stores go.
- R5 The store's own path is unchanged in all cases: `--store` still names the run store, and its directory is created as today.

## Architecture
```
--store Some(p), PII_STATE_DIR unset, [core].durable_dir unset   -> cfg.core.durable_dir = Some(p.parent()/durable)
PII_STATE_DIR set                                               -> no override: the state dir already moves everything
[core].durable_dir set in the file                              -> no override: the operator's value stands
--store absent                                                  -> no override: today's path
then Config::resolve_durable_dir keeps its single rule
```
Key decision: an explicit `--store` means "a separate instance", and the configuration already has the field that moves the durable stores, so the CLI
fills it instead of the daemon learning a second rule. Rejected: a new `state_dir_for_store` precedence chain in `pii-core` (two rules that must agree
for ever). Rejected: setting `PII_STATE_DIR` from the CLI (process-global, and it would hide the decision from the config it writes).

## TDD Contract
New unit tests in `crates/pii-cli/src/daemon_durable.rs` — `pii-cli` is a binary crate, so an integration test cannot
import it; the pure function is tested where it lives (no daemon, no store file opened).
| Test Name | Input | Expected Output |
|---|---|---|
| `an_explicit_store_puts_the_durable_dir_beside_it` | `--store /tmp/x/s.redb`, no `PII_STATE_DIR` | `Some("/tmp/x/durable")` |
| `CONTROL_pii_state_dir_still_wins_over_the_store` | both set | `None` (no override: the state dir alone moves everything) |
| `CONTROL_a_configured_durable_dir_wins_over_the_store` | `[core].durable_dir` already set | `None` (the operator's value stands) |
| `CONTROL_no_store_means_no_override` | no flag | `None` |
| `CONTROL_an_empty_pii_state_dir_is_not_a_value` | `PII_STATE_DIR=""`, store set | `Some("/tmp/x/durable")` |
| `CONTROL_a_store_with_no_parent_changes_nothing` | `--store s.redb` | `None` |

Acceptance: the six pass; each of these mutations makes at least one fail (the mutation script is the orchestrator's): the `PII_STATE_DIR` branch removed;
the configured-override branch removed; the empty-string guard dropped; the parent joined with `join("durable")` replaced by the parent alone.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-cli --bin pii daemon_durable::tests 2>&1 | grep -qE 'test result: ok. [6-9] passed'`

## Guardrails
- Keep green, unedited: `cargo test -q -p pii-cli --test daemon_start_mark` and `cargo test -q -p pii-server --test channel_wiring`.
- Do NOT change where `--store` itself points, do NOT make the runtime dir hold durable state, and do NOT add an environment variable.
- Do NOT change `Config::resolve_durable_dir` or its precedence: the override is expressed in the field it already reads.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-cli/src/cmd_daemon.rs`
- `crates/pii-cli/src/cmd_daemon_flags.rs`
- `crates/pii-cli/tests/store_flag_owns_the_durable_dir.rs`

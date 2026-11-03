# Spec: Delete the copies

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening · **Source**: Opus optimisation review 2026-11-03,
workstream **W7** — closes findings 9, 16, 17 and 18.

## Verified premises

- `pii_graph::nodes::eval_ctx` is `pub(crate)` (`crates/pii-graph/src/nodes.rs:9-15`) and is copied in 11 places
  (`worker.rs:27`, `spawn.rs:35`, `tool.rs:59`, …); the deep `$.` resolve is copied three times
  (`worker.rs:93-112`, `spawn.rs:127-146`, `tool.rs:81-101`); required-string config extraction appears ~30 times;
  `ok`/`err` outcome code appears 21 times.
- A local `fail()` shadows `cli_util::fail` and **skips `CliObserver`** in `cmd_skill.rs:106`, `cmd_entity.rs:88`,
  `cmd_learn.rs:107`, `cmd_index.rs:38`; `usage()` has 9 copies; `print_result` is re-inlined 5 times.
- `pii-store` has three error styles (`store_op` 146 uses, `internal` 40, inline `Internal(format!)` 130) and copied
  get/put/open bodies (`channel.rs:72`, `approval.rs:143`, `heal.rs:193`, `queue.rs:210`).
- `engine/registry.rs:83,86` omits `debug_crash` and `spawn` from the lint registry, so
  `pii graph lint graphs/starter/crash.toml` reports an unknown kind.

## Requirements

**R1.** One `eval_ctx`, one deep-resolve helper, one required-string helper, one outcome helper, all public to the
node kinds that need them. No behaviour change.

**R2.** One `fail()` in `pii-cli`, and it goes through `CliObserver`; the four shadowing copies are deleted.

**R3.** One `usage()` and one `print_result` in `pii-cli`.

**R4.** One store error constructor and one `get_json`/`put_json`/`open_aux` trio; the three error styles collapse to
one.

**R5.** The lint registry lists every registered kind, including `debug_crash` and `spawn`, so a lint of
`graphs/starter/crash.toml` exits 0.

## The requirement a plausible implementation satisfies in appearance only

A worker can delete the copies and the diff looks right while a shadowing `fail()` survives in one file, keeping the
one CLI whose failures are invisible. The criterion `grep -rc 'fn fail(' == 1` is what catches it; the test count check
is what proves no test was deleted along with the code it covered.

## TDD contract

| check | assertion |
|---|---|
| existing suites | every test count identical to the inventory baseline (`test-inventory --check`) |
| `pii graph lint graphs/starter/crash.toml` | exit 0 |
| `grep -rc 'fn fail(' crates/pii-cli/src` | exactly 1 |
| `git diff --stat` | at least 1,000 net lines deleted |

## Exit Criteria

- [ ] `cargo run -q -p xtask -- test-inventory --check`
- [ ] `cargo run -q -p pii-cli --bin pii -- graph lint graphs/starter/crash.toml`
- [ ] `[ "$(grep -rc 'fn fail(' crates/pii-cli/src | sed 's/.*://' | paste -sd+ | bc)" = "1" ]`
- [ ] `grep -q 'debug_crash' crates/pii-server/src/engine/registry.rs`

## Guardrails

- Do NOT change observable behaviour: this is a deletion pass, and every existing test must keep passing with the same
  count.
- Do NOT delete a test to make a helper removable.
- Do NOT touch `graphs/*.toml` semantics.
- The worker does not commit; the orchestrator reviews and commits.

## Files

- `crates/pii-graph/src/nodes.rs`
- `crates/pii-graph/src/nodes/worker.rs`
- `crates/pii-graph/src/nodes/spawn.rs`
- `crates/pii-graph/src/nodes/tool.rs`
- `crates/pii-cli/src/cli_util.rs`
- `crates/pii-cli/src/cmd_skill.rs`
- `crates/pii-cli/src/cmd_entity.rs`
- `crates/pii-cli/src/cmd_learn.rs`
- `crates/pii-cli/src/cmd_index.rs`
- `crates/pii-store/src/lib.rs`
- `crates/pii-store/src/channel.rs`
- `crates/pii-store/src/approval.rs`
- `crates/pii-store/src/heal.rs`
- `crates/pii-store/src/queue.rs`
- `crates/pii-server/src/engine/registry.rs`

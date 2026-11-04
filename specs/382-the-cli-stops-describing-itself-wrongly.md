# Spec: the CLI stops describing itself wrongly

## Overview

Documenting the command reference meant running every command and quoting what it does. Five
places where the binary's own description disagrees with the binary were confirmed by running them:

- **F-6** `run ls --status` cannot name a status the store holds, so a real run is invisible to the
  filter that exists to find it.
- **F-7** a missing `--config` file is silently replaced by defaults: the operator is told nothing
  and the daemon runs on settings nobody wrote.
- **F-8** `pii store stats` is documented as safe while the daemon runs, but it opens the store.
- **F-9** `pii graph` usage strings disagree with each other and omit `check`.
- **F-11** `night`'s report needs an input key that nothing validates at start; a missing key fails
  at the report, after the work.

Found by documentation work, 2026-11-04 (`manual-audit-20261104.md`).

**Project**: /projects/pi-improved
**Phase**: CLI truthfulness

## Requirements

- **R1** `run ls --status <s>` accepts every status the store can hold and rejects an unknown one
  with a message naming the accepted set.
- **R2** A `--config` path that does not exist is refused, naming the path. A default is used only
  when the flag is absent.
- **R3** `store stats` either opens the store read-only or its help says it needs the daemon.
- **R4** `pii graph` usage lists every subcommand it dispatches, and `check` appears in it.
- **R5** `night`'s required input keys are validated before the run starts, not at the report.

## Exit Criteria

- [ ] `cargo test -q -p pii-cli --test cli_describes_itself 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `./target/debug/pii run ls --status nope >/dev/null 2>&1; [ $? -eq 2 ]`
- [ ] `./target/debug/pii --config /nonexistent.toml health >/dev/null 2>&1; [ $? -ne 0 ]`
- [ ] `./target/debug/pii graph --help 2>&1 | grep -q check`

## Guardrails

- Do NOT document the behaviour instead of fixing it: the manual quotes the binary, so the binary is
  what has to become true.
- Do NOT make `--config` create the file.

## Files

- `crates/pii-cli/src/cmd_run.rs`
- `crates/pii-cli/src/cmd_store.rs`
- `crates/pii-cli/src/cmd_graph.rs`
- `crates/pii-cli/src/main.rs`
- `crates/pii-cli/tests/cli_self_description.rs`

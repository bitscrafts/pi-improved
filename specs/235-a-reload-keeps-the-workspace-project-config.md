# Spec: 235 — a reload keeps the workspace's project config

**Project**: /projects/pi-improved
**Phase**: after 234. Filed 2026-10-22 by the orchestrator, from its own measurement while wiring the
project-config layer into `pii daemon`.

`pii daemon` now layers `<workspace>/.pii/pii.toml` over the global `~/.pii/pii.toml` at start (the CLAUDE.md
model: one global file, one project file, the project wins for the sections a project may set). The other
reader of that file — `sys.reload` (F5.12) — was left behind:

```rust
// crates/pii-server/src/daemon/mod.rs:430 — reload_config
pub fn reload_config(ctx: &DaemonContext) -> Result<u32, PiiError> {
    let cfg = config::Config::load(&ctx.config_path)?;   // the global file, and nothing else
```

So one daemon gives two answers to "which config is in force": at start the project's `[core].model`,
`[llama].host` and `[[model_tier]]` list apply; after any `pii sys reload` they silently revert to the global
ones, and a global-only setting that a repository put in its project file stops being refused. Nothing throws,
which is why the gap is only visible by reading the code.

## Requirements

### Functional

- **R1 — reload applies the same merge as start.** `reload_config` loads the global file, loads the project
  file when the daemon was started with one, and resolves the effective config with `Config::merge_project` —
  the one function that owns the rules, not a second copy of them.
- **R2 — a refusal stays a refusal.** A project file that sets a global-only setting (`[net]`,
  `grant_ceiling`, `[core].runtime_dir`, `log_file`, `audit`, `observer`) makes `sys.reload` fail with the same
  named error start gives, and nothing from that file is applied (no partial config).
- **R3 — the path is the daemon's own workspace.** The project path is the one start used
  (`<workspace>/.pii/pii.toml`); another workspace's file is never read. A file that was absent at start and
  appears later is picked up; a file that disappears is not an error (the global file applies alone).
- **R4 — start's signature stays usable.** `Daemon::build` keeps its current call shape for the ~20 existing
  call sites (no project layer); the CLI takes a layered constructor. No test is rewritten to buy this.
- **R5 — the operator's view does not change.** `sys.reload` still answers `{workers_reloaded: n}`; the
  tests below read the live RPC, not internals.
- **R6 — mutation control.** Replacing the merge inside `reload_config` with the bare global config reddens
  `a_global_only_setting_added_after_start_is_refused_by_reload`. The orchestrator pastes that red in
  `## Closure` and restores the file byte-identical (`sha256sum` before and after, then `touch`).

## TDD Contract

| Test Name | Input | Expected Output |
| --- | --- | --- |
| `a_global_only_setting_added_after_start_is_refused_by_reload` | a daemon started with a valid project file; the file is then edited to set `[core].runtime_dir` | `pii sys reload` fails and the error names `runtime_dir`; the daemon keeps serving |
| `a_valid_project_file_reloads_instead_of_erroring` | a daemon started with a valid project file, unchanged | `pii sys reload` answers with `workers_reloaded` |
| `an_absent_project_file_still_reloads_the_global` | a daemon started with no project file | `pii sys reload` succeeds (the near-miss control for the two rows above) |

## Exit Criteria

A file-level `cargo test` alone is not a criterion for this spec: it is green on HEAD **before** any of the three
rows above exist (measured 2026-10-22 — the file had three start-path rows and zero `reload` rows, so the item
could have been called done with `reload_config` untouched). Each contract row is therefore asserted by name, and
a row that does not exist fails its criterion (`grep -c '1 passed'` reads 0).

- [ ] `cargo test -p pii-cli --test project_config_layers_live`
- [ ] `[ "$(cargo test -p pii-cli --test project_config_layers_live -- --exact a_global_only_setting_added_after_start_is_refused_by_reload 2>&1 | grep -c '^test result: ok\. 1 passed')" = 1 ]`
- [ ] `[ "$(cargo test -p pii-cli --test project_config_layers_live -- --exact a_valid_project_file_reloads_instead_of_erroring 2>&1 | grep -c '^test result: ok\. 1 passed')" = 1 ]`
- [ ] `[ "$(cargo test -p pii-cli --test project_config_layers_live -- --exact an_absent_project_file_still_reloads_the_global 2>&1 | grep -c '^test result: ok\. 1 passed')" = 1 ]`
- [ ] `cargo test -p pii-server --lib`
- [ ] `bash quality-gate.sh`

## Guardrails

- Do NOT re-implement the merge, and do NOT add a second project-path resolution: R1 names the one function and
  the one path the start path already uses.
- Do NOT weaken the refusal to make a reload succeed, and do NOT apply part of a refused file.
- Do NOT make the project file mandatory, and do NOT read another workspace's file to fill a gap.
- Do NOT touch `specs/` (this file is the orchestrator's). If the daemon context cannot carry the project path
  without changing every `Daemon::build` call site, STOP and report the shape you would need — a 20-file test
  rewrite is not an implementation of R4.

## Closure

(to be filled by the orchestrator: the three rows run green, the mutation red quoted under a
`mutation red: ` line, and the sha256 of the restored file)

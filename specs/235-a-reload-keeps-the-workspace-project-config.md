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

Closed by the orchestrator 2026-10-22 (item A103). The reload half did not exist at HEAD `f48298f`: the
three contract rows were absent and `reload_config` read the global file alone.

- **RED first, on the tree HEAD had.** With the three `reload` rows written and nothing implemented,
  `a_global_only_setting_added_after_start_is_refused_by_reload` failed with
  `a refused project setting fails the reload; stdout: {"workers_reloaded":0}` — the reload never looked at
  the project file, so the `runtime_dir` a repository put there was silently accepted. The other two rows are
  the near-miss controls (an unchanged valid project file; no project file at all) and were green before the
  change for the right reason: neither needs the merge.
- **What was implemented.** `DaemonContext.project_path` holds the PATH, not the contents, so a file that
  appears or disappears after start is seen (R3); `config_layers::project_path` and
  `config_layers::load_project` are now the one resolution and the one existence rule, called by start
  (`cmd_daemon`) and by `reload_config` alike; `reload_config` resolves the effective config through
  `Config::merge_project`, the function that owns the rules (R1), and a refused file applies nothing (R2).
  `Daemon::build`/`build_with_env` keep their shapes (R4) — the CLI calls the new `Daemon::build_layered`.
  The only test edits are the mechanical `project_path: None` on three `DaemonContext` literals
  (`daemon/mod.rs` fixture, `daemon/router.rs` fixture, `tests/poll_backoff.rs`); no assertion was touched.
- **GREEN on the tree this commit carries.** `cargo test -p pii-cli --test project_config_layers_live`
  **6 passed, 0 failed**; each reload row by name **1 passed, 0 failed**; `cargo test -p pii-server --lib`
  **136 passed, 0 failed, 1 ignored**; `bash quality-gate.sh` **ALL PASS**
  (`_tmp/autonomy/logs/a103-gate4-20261022T124012Z.log`, gate_rc=0).
- sha256 `crates/pii-server/src/daemon/mod.rs` before the mutation:
  `6a992ca2431104fea361bfd360c743a978b3ed9a029c397647ba96b6627ab4f9`

  mutation red: `reload_config`'s body reduced to `let cfg = config::Config::load(&ctx.config_path)?;` (the
  bare global config — R6) reddens one row and only that row:
  `a_global_only_setting_added_after_start_is_refused_by_reload` →
  `assertion left == right failed … stdout: {"workers_reloaded":0}; left: Some(0) right: Some(2)`,
  the other five still green. Restored from the saved copy and `touch`ed; sha256 after:
  `6a992ca2431104fea361bfd360c743a978b3ed9a029c397647ba96b6627ab4f9` (identical), and the row re-ran green
  (`1 passed; 0 failed`) on the restored file, in that order.
- **Gate noise recorded, not hidden.** Two earlier gate runs of this same tree failed inside `xtask`'s
  `spec72_reachability` and not in this item: a second agent session was running
  `cargo llvm-cov --workspace` over the same tree at the same time, and that fixture mutates a shared
  `_tmp/spec72-reachability-integration/<tag>` export, so two concurrent suites collide
  (`expected exactly one non-test caller of pii_model::resolve_with_transport(: […queue.rs, …observed_provider.rs]`).
  With no code change in between, the run after the foreign coverage finished measured **ALL PASS**.
  Finding for a follow-up spec: that fixture's scratch is not concurrency-safe (its decoy files are shared by
  tag and its cleanup ignores errors) — not fixed here.


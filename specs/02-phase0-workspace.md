# Spec: Phase 0 - Workspace Skeleton

## Overview
**Project**: <repo>
**Phase**: 0 - Workspace skeleton & CI gate scaffold

Create the Cargo workspace skeleton and the CI/gate scaffold that every later phase compiles and lints against. This phase delivers **no business logic**: it delivers a workspace that builds clean, a kernel-ABI crate whose dependency purity is machine-enforced, an `xtask` gate runner, a `pii --version` binary, and a `quality-gate` entry point.

Source of truth: `_docs/06-research/002-pii-architecture-plan-pass2.md` §1.3 (lints), §1.4 (450-line gate), §1.6 (CI), §12.1–12.3 (crate layout + dependency DAG), §14 Phase 0; and `_docs/06-research/003-pii-architecture-plan-pass3.md` §4.1 (one harness).

**Environment facts (verified on this host — read these before writing any command):**
- Rust **1.95.0** stable is installed at `/usr/local/cargo/bin` (`CARGO_HOME=/usr/local/cargo`, `RUSTUP_HOME=/usr/local/rustup`). `rustfmt 1.9.0-stable` and `clippy 0.1.95` are present. Edition 2024 is supported.
- **`cargo` is NOT on `PATH`** in the shell that runs gates and exit criteria. Every shell step must begin with `export PATH=/usr/local/cargo/bin:$PATH`.
- The only installed toolchain is `1.95.0-x86_64-unknown-linux-gnu`. `rust-toolchain.toml` must pin exactly `1.95.0`; any other channel string triggers a toolchain download.
- crates.io is reachable (sparse index HTTP 200). Current versions: `serde 1.0.229`, `serde_json 1`, `thiserror 2.0.20`, `schemars 1.2.2`.
- `clippy::integer_arithmetic` **does not exist** in clippy 0.1.95 (verified against `-W help`). Pass-2 §1.3 lists it; it was renamed to `arithmetic_side_effects`. Using the old name makes `-D warnings` fail on an unknown lint.
- Note for the human operator (not a task for the implementer): `.pidag/config.toml` sets `quality_gate_script` to the global skill at `<agent-config>`, which invokes bare `cargo` and will fail with "command not found" in this container. Phase 0 ships a repo-local `quality-gate.sh` that fixes `PATH` itself; repoint `quality_gate_script` at it.

## Requirements

### Functional
- **F1** — Workspace root `Cargo.toml` at `<repo>`: virtual manifest with `members = ["crates/pii-abi", "crates/pii-cli", "xtask"]`, `resolver = "2"`, and a `[workspace.package]` block carrying `version = "0.1.0"`, `edition = "2024"`, `rust-version = "1.95"`, `license = "MIT OR Apache-2.0"`, `repository`, `description`. Every member inherits these with `.workspace = true`.
- **F2** — `[workspace.dependencies]` declares exactly four external crates, used by members via `{ workspace = true }`: `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `thiserror = "2"`, `schemars = { version = "1", features = ["derive"] }`.
- **F3** — `crates/pii-abi` — the kernel ABI. **Traits and types only.** Contents, all tiny:
  - `src/lib.rs` — `#![forbid(unsafe_code)]`, crate-level `//!` docs, `mod`/`pub use` re-exports.
  - `src/error.rs` — `PiiError` (`thiserror::Error`) with variants `Invalid(String)`, `Denied(String)`, `Cancelled`, `NotFound(String)`, `Internal(String)`.
  - `src/ids.rs` — newtypes `RunId`, `NodeId`, `PortName` over `String`, each deriving `Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema` with `Display` and `From<&str>`.
  - `src/effects.rs` — `Effects` enum `{ Pure, ReadsFs, WritesFs, MutatesExternal }` with `pub const fn is_parallel_safe(self) -> bool` (true for `Pure`/`ReadsFs`), per pass-2 §1.2 / pass-3 §4.3.
  - `src/node.rs` — `NodeInput { pub payload: serde_json::Value }`, `Outcome` enum `{ Next { port: PortName }, Done, Failed { error: String } }`, and a **synchronous** trait `NodeMeta { fn kind(&self) -> &str; fn effects(&self) -> Effects; fn ports(&self) -> &[PortName]; }`.
  - **No async trait in Phase 0.** The async `Node::run` contract (pass-3 §4.1) needs `async-trait`/`futures-core`, which are outside this phase's allowlist; it lands in Phase 1 together with an explicit widening of the `xtask` allowlist. Do not add it now.
- **F4** — `crates/pii-cli` — binary crate producing an executable named `pii` (`[[bin]] name = "pii"`, `path = "src/main.rs"`). Zero dependencies (no `clap`) to protect the Phase-1 cold-start budget. Behaviour:
  - `pii --version` / `pii -V` → prints exactly `pii <version>` on stdout, exit 0, where `<version>` is `env!("CARGO_PKG_VERSION")`.
  - any other argv → prints a one-line usage string, exit code 2.
  - The version string is produced by a testable `fn version_line() -> String`; `main` returns `std::process::ExitCode` (never call `std::process::exit`, which `clippy::exit` denies).
- **F5** — `xtask` — dev-tools runner at `xtask`, reachable as `cargo xtask <cmd>` via `.cargo/config.toml` containing `[alias] xtask = "run --package xtask --"`. Subcommands: `deps-check`, `lint-files`, `ci` (runs both in order). Unknown subcommand → usage + exit 2. Only dependency: `serde_json` (workspace).
- **F6** — `xtask deps-check` enforces ABI purity. It shells out to `cargo metadata --format-version 1` and hands the parsed JSON to a **pure, unit-testable** function:
  ```rust
  pub(crate) fn check_abi_purity(metadata: &serde_json::Value) -> Result<(), Vec<String>>
  ```
  with these constants:
  ```rust
  const ABI_CRATE: &str = "pii-abi";
  const ABI_ALLOWED_DIRECT: &[&str] = &["serde", "serde_json", "thiserror", "schemars"];
  const FORBIDDEN_TRANSITIVE: &[&str] =
      &["tokio", "reqwest", "redb", "wasmtime", "axum", "hyper", "rustls"];
  ```
  Two rules, both violations reported (never `panic!`, never `unwrap`):
  1. **Direct allowlist** — every entry in `pii-abi`'s `dependencies` array whose `kind` is not `"dev"` must have a `name` in `ABI_ALLOWED_DIRECT`.
  2. **Transitive denylist** — BFS `resolve.nodes` from `pii-abi`'s package id following `deps[].pkg`; resolve ids to names via `packages[]`; no reached name may appear in `FORBIDDEN_TRANSITIVE`.
  Exit 0 clean, exit 1 with one line per violation otherwise.
- **F7** — `xtask lint-files` enforces the 450-line gate (pass-2 §1.4, simplified for Phase 0). Pure function `pub(crate) fn count_effective_lines(src: &str) -> usize` counting lines that are non-empty after trimming and do not start with `//`. Walks `crates/*/src/**/*.rs` and `xtask/src/**/*.rs`, skipping `target/`. **Fails at > 450**, prints a warning line at > 380. The richer pass-2 semantics (`#[cfg(test)]` exclusion, `xtask/file-size-allow.toml`) are deliberately deferred to Phase 1.
- **F8** — `[workspace.lints]` in the root `Cargo.toml`, inherited by **every** member via a `[lints] workspace = true` table in its own `Cargo.toml`:
  ```toml
  [workspace.lints.rust]
  unsafe_code           = "forbid"
  missing_docs          = "deny"
  unreachable_pub       = "deny"
  unused_qualifications = "warn"
  rust_2018_idioms      = { level = "deny", priority = -1 }

  [workspace.lints.clippy]
  pedantic = { level = "warn", priority = -1 }
  nursery  = { level = "warn", priority = -1 }
  cargo    = { level = "warn", priority = -1 }

  unwrap_used                = "deny"
  expect_used                = "deny"
  panic                      = "deny"
  indexing_slicing           = "deny"
  await_holding_lock         = "deny"
  await_holding_refcell_ref  = "deny"
  undocumented_unsafe_blocks = "deny"
  mem_forget                 = "deny"
  exit                       = "deny"
  print_stdout               = "deny"
  print_stderr               = "deny"
  dbg_macro                  = "deny"
  todo                       = "deny"
  unimplemented              = "deny"
  large_futures              = "deny"
  large_enum_variant         = "deny"
  missing_panics_doc         = "warn"
  missing_errors_doc         = "warn"

  arithmetic_side_effects = "allow"
  module_name_repetitions = "allow"
  must_use_candidate      = "allow"
  missing_const_for_fn    = "allow"
  multiple_crate_versions = "allow"
  cargo_common_metadata   = "allow"
  ```
  `unsafe_code = "forbid"` is workspace-wide in Phase 0; pass-2 §1.3's override for `pii-hw`/`pii-memory` arrives with those crates.
- **F9** — Because `print_stdout`/`print_stderr` are denied but `pii-cli` and `xtask` must write to the terminal, each of those two crates carries a crate-level allow **in source** with a one-line reason comment: `#![allow(clippy::print_stdout, clippy::print_stderr)]`. Cargo rejects mixing `[lints] workspace = true` with per-crate lint tables, so these overrides must be inner attributes in `main.rs`, not manifest entries. `pii-abi` gets no such allow.
- **F10** — `rust-toolchain.toml`: `[toolchain] channel = "1.95.0"`, `components = ["rustfmt", "clippy"]`, `profile = "minimal"`.
- **F11** — `rustfmt.toml`, **stable-only options only**: `edition = "2024"`, `max_width = 100`, `newline_style = "Unix"`. Do not add `imports_granularity` or `group_imports` (nightly-only; they emit warnings on this stable toolchain).
- **F12** — `clippy.toml` restricted to paths that actually resolve in this workspace, so no `disallowed_*` entry is dead:
  ```toml
  disallowed-methods = [
    { path = "std::path::Path::canonicalize",  reason = "use dunce::canonicalize (Phase 1)" },
    { path = "std::process::Command::spawn",   reason = "use Ctx::proc() (Phase 1)" },
    { path = "std::time::SystemTime::now",     reason = "use Ctx::now() - deterministic in replay" },
    { path = "std::env::var",                  reason = "use pii_core::config (Phase 1)" },
  ]
  disallowed-types = [
    { path = "std::sync::Mutex", reason = "use parking_lot::Mutex or tokio::sync::Mutex" },
  ]
  too-many-lines-threshold  = 80
  type-complexity-threshold = 250
  ```
  The `reqwest`/`dirs`/`tokio`/`tracing` entries from pass-2 §1.3 land in Phase 1 with the crates they reference.
- **F13** — Release profiles in the root manifest, per pass-2 §1.3:
  ```toml
  [profile.release]
  opt-level = "z"; lto = "fat"; codegen-units = 1; panic = "abort"; strip = true
  overflow-checks = true
  [profile.perf]
  inherits = "release"; opt-level = 3; lto = "thin"; debug = 1; strip = false; panic = "unwind"
  ```
- **F14** — `quality-gate.sh`, executable (`chmod +x`), `set -uo pipefail`, first line of body `export PATH=/usr/local/cargo/bin:$PATH`, `cd` to its own directory, then run in order: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. Print a one-line PASS/FAIL per step; exit 0 only if all three pass.
- **F15** — `.gitignore` containing at least: `target/`, `.pidag/`, `_tmp/`, `**/*.rs.bk`, `*.profraw`, `_repos/`, `.DS_Store`.
- **F16** — Tests. Unit tests live **inside the crates** (`#[cfg(test)] mod tests` in the relevant `.rs` file); only test *artifacts* (temp dirs a test creates) go under `_tmp/`. Fixtures for `deps-check` are embedded `const &str` JSON literals — no filesystem, no network. Test module bodies open with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` so `--all-targets` clippy stays clean. Required test names, referenced verbatim by the exit criteria:
  - `deps_check_accepts_clean_abi`
  - `deps_check_rejects_tokio_in_pii_abi`
  - `deps_check_rejects_forbidden_transitive_dep`
  - `lint_files_rejects_451_lines`
  - `lint_files_accepts_450_lines`
  - `version_line_matches_cargo_pkg_version`
  - `effects_parallel_safety` (in `pii-abi`)
  - `outcome_serde_roundtrip` (in `pii-abi`)

### Non-Functional
- **N1** — Clean checkout → `cargo check`, `cargo clippy -D warnings`, `cargo fmt --check`, and `cargo test` all succeed with no manual steps beyond the `PATH` export.
- **N2** — No `unsafe` anywhere. Every crate root carries `#![forbid(unsafe_code)]` in addition to the workspace lint.
- **N3** — No file exceeds 450 effective lines; every file in this phase should be well under 150.
- **N4** — `pii-abi` links no transport, runtime, storage, or engine crate — directly or transitively. This is pass-2's load-bearing structural rule and `deps-check` must provably fail when it is broken.
- **N5** — Total external dependency surface for the whole workspace in Phase 0 is the four crates in F2 plus their own transitive trees. Adding any other external crate is out of scope.
- **N6** — `pii-cli` has zero non-workspace dependencies, so the Phase-1 cold-start gate (`pii --version` p95 < 20 ms) starts from a clean baseline.

## Architecture

```

├── Cargo.toml              # virtual workspace: members, resolver="2", workspace.package,
│                           #   workspace.dependencies, [workspace.lints], profiles
├── rust-toolchain.toml     # channel = "1.95.0"  (the only installed toolchain)
├── rustfmt.toml            # stable options only
├── clippy.toml             # disallowed-{methods,types} limited to resolvable std paths
├── .cargo/config.toml      # [alias] xtask = "run --package xtask --"
├── .gitignore              # target/ .pidag/ _tmp/ _repos/ ...
├── quality-gate.sh         # fmt --check && clippy -D warnings && test   (fixes PATH itself)
├── crates/
│   ├── pii-abi/            # KERNEL ABI - traits + types only
│   │   ├── Cargo.toml      #   serde, serde_json, thiserror, schemars   <- and nothing else
│   │   └── src/{lib,error,ids,effects,node}.rs
│   └── pii-cli/            # bin "pii"  - zero deps
│       ├── Cargo.toml
│       └── src/main.rs
└── xtask/                  # dev-tools runner  - serde_json only
    ├── Cargo.toml
    └── src/{main,deps,lint_files}.rs

Dependency DAG enforced this phase (pass-2 §12.2):

    pii-cli ──► (nothing yet; pii-abi wired in Phase 1)
    xtask   ──► serde_json
    pii-abi ──► serde, serde_json, thiserror, schemars        [HARD CEILING]
                and NOT tokio | reqwest | redb | wasmtime | axum | hyper | rustls

Gate flow:

    quality-gate.sh ─┬─► cargo fmt --all --check
                     ├─► cargo clippy --workspace --all-targets -- -D warnings
                     └─► cargo test --workspace
                                │
    cargo xtask ci ──┬─► deps-check ──► cargo metadata ──► check_abi_purity(json)
                     │                                       ├─ direct allowlist
                     │                                       └─ transitive denylist (BFS)
                     └─► lint-files ──► count_effective_lines(src) > 450 ⇒ fail
```

`check_abi_purity` and `count_effective_lines` are pure functions over already-parsed input. That is what makes the negative cases (`deps_check_rejects_tokio_in_pii_abi`, `lint_files_rejects_451_lines`) testable from an embedded fixture without ever adding `tokio` to the real manifest or writing a 451-line file.

## TDD Contract

| Test Name | Input | Expected Output |
|-----------|-------|-----------------|
| `deps_check_accepts_clean_abi` | embedded `cargo metadata` JSON where `pii-abi` has exactly `serde`, `serde_json`, `thiserror`, `schemars` | `Ok(())` |
| `deps_check_rejects_tokio_in_pii_abi` | same fixture with a direct `tokio` dependency (`kind: null`) added to `pii-abi` | `Err(v)` where some element of `v` contains `tokio` |
| `deps_check_rejects_forbidden_transitive_dep` | fixture where `pii-abi` → allowed crate → `redb` in `resolve.nodes` | `Err(v)` where some element of `v` contains `redb` |
| `deps_check_ignores_dev_dependencies` | fixture with a `kind: "dev"` entry naming a non-allowlisted crate | `Ok(())` |
| `lint_files_accepts_450_lines` | 450 code lines + 60 blank lines + 40 `//` comment lines | `count_effective_lines == 450`, no violation |
| `lint_files_rejects_451_lines` | 451 code lines | violation reported naming the file and the count `451` |
| `version_line_matches_cargo_pkg_version` | `version_line()` | equals `format!("pii {}", env!("CARGO_PKG_VERSION"))` |
| `effects_parallel_safety` | each `Effects` variant | `Pure`/`ReadsFs` → `true`; `WritesFs`/`MutatesExternal` → `false` |
| `outcome_serde_roundtrip` | `Outcome::Next { port: PortName::from("ok") }` | `serde_json` round-trip is equal to the original |
| `pii_error_display` | `PiiError::Denied("fs_write".into())` | `to_string()` is non-empty and contains `fs_write` |

## Exit Criteria
- [ ] `cd /projects/pi-improved && grep -q 'resolver = "2"' Cargo.toml && grep -q 'edition = "2024"' Cargo.toml && grep -q 'crates/pii-abi' Cargo.toml && grep -q 'crates/pii-cli' Cargo.toml && grep -q 'xtask' Cargo.toml`
- [ ] `cd /projects/pi-improved && for f in crates/pii-abi/Cargo.toml crates/pii-cli/Cargo.toml xtask/Cargo.toml; do grep -q "^\[lints\]" "$f" || exit 1; done`
- [ ] `cd /projects/pi-improved && grep -q "unsafe_code" Cargo.toml && grep -q "missing_docs" Cargo.toml && grep -q "unreachable_pub" Cargo.toml && grep -q "rust_2018_idioms" Cargo.toml && grep -q "unwrap_used" Cargo.toml && grep -q "indexing_slicing" Cargo.toml && grep -q "await_holding_lock" Cargo.toml && grep -q "pedantic" Cargo.toml && grep -q "nursery" Cargo.toml`
- [ ] `cd /projects/pi-improved && ! grep -qE "^(tokio|reqwest|redb|wasmtime|axum|hyper|rustls)" crates/pii-abi/Cargo.toml`
- [ ] `cd /projects/pi-improved && test "$(grep -rl "forbid(unsafe_code)" --include=*.rs crates xtask | wc -l)" -ge 3`
- [ ] `cd /projects/pi-improved && grep -q "^target/" .gitignore && grep -q "_tmp/" .gitignore && grep -q ".pidag/" .gitignore`
- [ ] `cd /projects/pi-improved && grep -q 'channel = "1.95.0"' rust-toolchain.toml && ! grep -q "imports_granularity" rustfmt.toml`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo fmt --all --check`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo check --workspace --all-targets 2>&1 | grep -q "Finished"`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | grep -q "Finished"`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test --workspace --quiet`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p xtask -- --list 2>&1 | grep -q "deps_check_rejects_tokio_in_pii_abi"`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p xtask -- --list 2>&1 | grep -q "deps_check_rejects_forbidden_transitive_dep"`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p xtask -- --list 2>&1 | grep -q "lint_files_rejects_451_lines"`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask deps-check`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask lint-files`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask ci`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo build -q -p pii-cli && ./target/debug/pii --version | grep -qE "^pii [0-9]+\.[0-9]+\.[0-9]+$"`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && ./target/debug/pii --bogus-flag; test $? -eq 2`
- [ ] `cd /projects/pi-improved && test -x quality-gate.sh && ./quality-gate.sh`

## Guardrails
- Do NOT implement business logic. No scheduler, no journal, no graph engine, no `Ctx`, no async runtime. Phase 0 is scaffolding; every file should be under 150 lines.
- Do NOT add any external dependency beyond the four in F2 (`serde`, `serde_json`, `thiserror`, `schemars`). No `clap`, no `anyhow`, no `cargo_metadata`, no `walkdir`, no `tokio`, no `async-trait`. Use `std` for argv, directory walking, and process spawning.
- Do NOT add `tokio`, `reqwest`, `redb`, `wasmtime`, `axum`, `hyper`, or `rustls` to `pii-abi` — not as a dependency, not as a dev-dependency, not to "make the test realistic". The negative tests use embedded JSON fixtures only.
- Do NOT write `clippy::integer_arithmetic` in the lint table. It does not exist in clippy 0.1.95 and an unknown lint under `-D warnings` fails the whole gate. Use `arithmetic_side_effects = "allow"`.
- Do NOT put nightly-only options (`imports_granularity`, `group_imports`, `use_field_init_shorthand`) in `rustfmt.toml`.
- Do NOT add `disallowed-methods` / `disallowed-macros` entries in `clippy.toml` whose paths cannot resolve in this workspace (`reqwest::*`, `tokio::*`, `dirs::*`, `tracing::*`, `println`). They arrive in Phase 1 with the crates they name.
- Do NOT pin `rust-toolchain.toml` to `stable`, `nightly`, or any channel other than `1.95.0` — it is the only installed toolchain and anything else forces a download.
- Do NOT rely on `cargo` being on `PATH`. Every script and every shell step starts with `export PATH=/usr/local/cargo/bin:$PATH`.
- Do NOT mix `[lints] workspace = true` with per-crate `[lints.rust]` / `[lints.clippy]` tables in the same `Cargo.toml` — Cargo rejects it. Per-crate lint relaxations go in source as inner attributes (`#![allow(...)]`) with a one-line reason comment.
- Do NOT silence lints beyond the allow-list in F8 and the two crate-level allows in F9 plus the test-module allows in F16. If a `pedantic` or `nursery` lint fires on skeleton code, restructure the code. If a lint is a demonstrable false positive, add `#[allow(...)]` at the narrowest possible scope with a `// reason:` comment.
- Do NOT use `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `dbg!`, or `std::process::exit` in non-test code. Return `Result` and let `main` return `std::process::ExitCode`.
- Do NOT create a git repository, run `git init`, stage, or commit. File creation only.
- Do NOT write anything to `/tmp`. Test artifacts go under `_tmp/`. Rust test *sources* stay inside their crates.
- Do NOT run `rm -rf` or delete/overwrite anything that already existed before this run. In particular do NOT touch `_repos/`, `_docs/`, `.pidag/`, `specs/`, `AGENTS.md`, or `HANDOFF.md`.
- Do NOT create crates beyond `pii-abi`, `pii-cli`, and `xtask`. The remaining crates from pass-2 §12.1 arrive in later phases; pass-2 §12.3 is explicit that starting with 18 crates is the wrong move.
- Do NOT add an async `Node` trait, `async-trait`, `futures-core`, `bitflags`, or `smol_str` to `pii-abi`. Widening the `ABI_ALLOWED_DIRECT` list is a Phase 1 decision and must be made in the spec, not in the implementation.

# pi-improved — spec-58: pii-cli test coverage for cli_util.rs/ndjson.rs (F-20 tidy)

- **Project**: `/projects/pi-improved`
- **Priority**: LOW-MEDIUM — Phase 5.3 tail. `pii-cli` has 3/13 files
  with inline tests. This spec deliberately targets ONLY the two
  smallest, purest files (`cli_util.rs`, 49 lines; `ndjson.rs`, 36
  lines) — NOT the big `cmd_*.rs` command files, which mostly wire a
  real daemon/UDS connection or spawn real subprocesses and would need
  much heavier test infrastructure (out of scope here; a future spec
  if pursued). This is a small, safe, genuinely useful increment, not
  an attempt to close all of F-20's `pii-cli` gap in one pass.

## Overview

Ground truth already confirmed by the orchestrator (verify yourself
from the actual current source before writing tests):

- `cli_util.rs` (49 lines): three `pub(crate)` functions —
  `usage_error(msg, usage) -> ExitCode` (prints to stderr, returns
  `ExitCode::from(2)` — the F5.3 usage exit code), `next_value(args,
  flag, usage) -> Result<String, ExitCode>` (pulls the next arg from a
  `std::env::ArgsOs` iterator, converts to a `String`, or returns the
  usage exit code via `usage_error` if there's no next arg or it isn't
  valid UTF-8), `open_store(db: &Path) -> Option<Store>` (creates the
  parent dir if needed, opens a `pii_store::Store`, returns `None`
  after printing an error on either failure).
- `ndjson.rs` (36 lines): `NdjsonObserver` implements `pii_core::
  Observer` — `event()` prints a `{"seq":N,"event":...}` line for
  `ObsEvent::Journal` (ignores other `ObsEvent` variants — check what
  other variants exist and confirm this silent-ignore is correct
  behavior, not an oversight, before asserting on it), with a fallback
  hand-written JSON line if `serde_json::to_string(event)` fails, and a
  `metric()` method that prints `{"metric":name,"value":N}` by hand
  (comment says `Metric` isn't `Serialize`).
- Both files' functions/structs are currently `pub(crate)` (with a
  `#[allow(clippy::redundant_pub_crate)]` — the whole binary crate's
  established pattern, matching `pii-tui`'s `input.rs`/spec-50's
  precedent). Tests living in the SAME file (inline `#[cfg(test)] mod
  tests`) can reach `pub(crate)` items directly with no visibility
  changes needed — confirm this is true before assuming it, but it
  should be, since `pub(crate)` is visible anywhere in the same crate.

## Requirements

### Functional

- R1. `cli_util.rs`: test `usage_error` (captures/asserts on the
  `ExitCode` return; stderr output capture is optional/best-effort
  since Rust's std doesn't make capturing another process's stderr
  from within the same test easy — if you can't cleanly assert on the
  printed text, assert on the `ExitCode` alone, that's fine). Test
  `next_value`'s two paths: a real value present → `Ok(String)`; args
  exhausted → `Err(ExitCode::from(2))`. Test `open_store`: a valid,
  writable temp path → `Some(Store)`; an invalid path (e.g. a path
  component that's actually a file, not a directory, so `create_dir_all`
  fails) → `None`.
- R2. `ndjson.rs`: `NdjsonObserver::event()` is the main thing worth
  testing, but it prints directly to stdout via `println!` — capturing
  stdout from within a test is the same class of problem as R1's
  stderr note. Two options, pick whichever is the smaller, cleaner
  diff after reading the file: (a) if `Observer::event`'s signature
  allows it, refactor NOTHING and instead just call the method and
  confirm it doesn't panic on both a `Journal` event and a
  serialization-failure case (weaker but honest coverage of the
  fallback path); or (b) if genuinely valuable and low-risk, extract
  the line-formatting logic into a small private helper that RETURNS a
  `String` instead of printing directly, call `println!` on the result
  at the call site — this makes the formatting itself fully testable.
  If you choose (b), this is a NON-test code change and must be called
  out explicitly in your summary (it's a legitimate minimal refactor
  in service of testability, not forbidden, but it's not "test-only"
  either — be honest about it in your report).
- R3. Whichever path R2 takes, cover: a `Journal` event round-trips to
  the expected `{"seq":N,"event":...}` shape; the metric-line shape is
  correct; and (if choosing R2 option (b)) the serialize-failure
  fallback path is exercised somehow — even a comment explaining why
  it's hard to trigger `serde_json::to_string` failure on a real
  `RunEvent` (probably impossible without a contrived value) is an
  acceptable, honest outcome — don't force a test that doesn't
  actually prove anything.

### Non-Functional

- N1. Do NOT modify `cli_util.rs`'s or `ndjson.rs`'s LOGIC except for
  the narrow, called-out exception in R2 option (b) if chosen.
- N2. Do NOT touch any other file in `crates/pii-cli/src/`.
- N3. Standard test-module clippy allows apply.

## Architecture

Appends `#[cfg(test)] mod tests { ... }` to the end of both files
(possibly with a small helper extraction in `ndjson.rs`, see R2).

## TDD Contract

| Test Name (illustrative) | Input | Expected Output |
|---|---|---|
| `usage_error_returns_exit_code_2` | any msg/usage strings | `ExitCode::from(2)` |
| `next_value_returns_present_arg` | args with a following value | `Ok(String)` matching it |
| `next_value_errors_when_exhausted` | args with nothing left | `Err(ExitCode::from(2))` |
| `open_store_succeeds_on_valid_path` | a writable temp path | `Some(Store)` |
| `open_store_fails_on_invalid_path` | a path whose parent can't be created | `None` |
| `ndjson_journal_event_shape` | a `Journal` event | correct `{"seq":...,"event":...}` shape (via whichever R2 approach) |
| `ndjson_metric_shape` | a `Metric` | correct `{"metric":...,"value":...}` shape |

## Exit Criteria

- [ ] `cargo test -p pii-cli`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `grep -q "cfg(test)" crates/pii-cli/src/cli_util.rs`
- [ ] `grep -q "cfg(test)" crates/pii-cli/src/ndjson.rs`

## Guardrails

- G1. Do NOT touch any `cmd_*.rs` file or `main.rs`.
- G2. Any non-test-only change (the R2 option (b) exception) must be
  explicitly called out in the implementer's final summary — do not
  silently bundle it in.
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).

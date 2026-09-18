# Gate Hygiene: Wire lint-files In, Fix clippy.toml, Fix Line Counter

- **Project**: `/projects/pi-improved`
- **Crate**: `xtask` (mostly), plus `quality-gate.sh`, `clippy.toml`
- **Priority**: MEDIUM — closes a real enforcement gap plus two small
  correctness bugs in the gate tooling itself. Sourced from fable
  review findings/candidates 2, 3, 6, 7 (both passes),
  `_docs/06-research/032-fable-pii-review-2026-09-18.md`. Bundled into
  one spec because all four items are small, touch the same area
  (build/lint tooling), and share verification (`quality-gate.sh`
  itself, run at the end, proves all four).

## Overview

Four small, related fixes.

### Fix 1: `xtask lint-files` is not part of the quality gate

`.pidag/config.toml` names `quality-gate.sh` as `quality_gate_script`.
That script runs fmt, clippy, build, and test — it never runs `cargo
run -q -p xtask -- lint-files` (the 450-effective-line-per-file gate).
The line gate is enforced only by manual `cargo xtask ci` invocation,
`scripts/mvp-verify.sh`, or per-spec exit criteria when a spec author
remembered to list it (54 of 77 existing spec files do; 23 don't). A
pidag SDD run whose spec's exit criteria omit `lint-files` can push a
file over 450 effective lines and the loop will still report success.

### Fix 2: `xtask ci` and `mvp-verify.sh` risk the pre-F-01 cargo-target race

`quality-gate.sh` isolates clippy and doctests into their own
`CARGO_TARGET_DIR` and runs `cargo build` before `cargo test`
specifically to avoid a race on `schemars`/`serde_json` rlib hashes
(the F-01 fix — search git log/HANDOFF for "F-01" if you want the
original incident). `xtask ci` (`xtask/src/main.rs`) and
`scripts/mvp-verify.sh` still run `clippy --all-targets` followed by
`cargo test --workspace` in the SAME shared `target/debug`, the exact
shape that raced before. Neither has hit it recently (this is
latent, not currently failing), but whoever next runs `cargo xtask ci`
cold is at risk of hitting a bug `quality-gate.sh` already fixed
elsewhere.

### Fix 3: `clippy.toml`'s disallowed-macros `println` entry never fires

`clippy.toml` has `{ path = "println", reason = "use
pii_core::Observer::event/metric, or in pii-cli only, the UI writer" }`.
Clippy's `disallowed-macros` lint requires the FULLY QUALIFIED path
(`std::println`), not the bare macro name — as written, the rule
silently never matches anything, and it prints a `does not refer to a
reachable macro` warning once per compiled target (88 lines of noise
across the workspace's ~221 targets, per fable's review). The rule is
currently harmless only because `[workspace.lints.clippy] print_stdout
= "deny"` in the root `Cargo.toml` already covers the same ground
independently — but a broken rule that happens to be redundant with a
working one is still worth fixing, since a future reader may reasonably
believe `disallowed-macros` is what enforces this.

### Fix 4: `count_effective_lines` mis-parses one attribute shape

`xtask/src/lint_files.rs`'s `count_effective_lines` sets `in_cfg_test
= true` on seeing `#[cfg(test)] `, then starts brace-depth tracking
from the NEXT line. If that next line is ANOTHER attribute with no
braces (e.g. `#[rustfmt::skip]`), `opens - closes` is `0 - 0 = 0`, the
`cfg_test_depth <= 0` check fires immediately, `in_cfg_test` resets to
`false`, and the entire test module that follows gets counted as
production code. Confirmed real instance:
`crates/pii-abi/src/failover.rs` has `#[cfg(test)]` then
`#[rustfmt::skip]` then `mod tests { ... }` — the ~163-effective-line
test body is currently counted as production code, inflating that
file's reported size by that amount (verify the exact current
effective-line delta yourself; it may have shifted since fable's
review measured it).

Ground truth (verify yourself, re-read every file — line numbers will
have moved since this spec was drafted):
- `quality-gate.sh` — read the whole script; find where the fmt/
  clippy/build/test steps are, to know where to insert a lint-files
  step and in what target-dir context.
- `xtask/src/lint_files.rs` — `count_effective_lines`, the full
  function (documented above; the fix is to keep consuming lines that
  start with `#[` while `in_cfg_test` is true and brace depth is still
  `0`, before starting brace-depth tracking for real).
- `xtask/src/main.rs` — the `ci` subcommand's cargo invocations.
- `scripts/mvp-verify.sh` — its cargo invocations.
- `clippy.toml` — the `disallowed-macros` `println` entry.
- `crates/pii-abi/src/failover.rs` — the confirmed miscounted
  instance, to verify Fix 4 against a real before/after measurement.

## Requirements

### Functional

- R1 (Fix 1). Add a lint-files step to `quality-gate.sh`: `cargo run
  -q -p xtask -- lint-files`, placed after the existing build/test
  steps (it needs no special target-dir isolation — it's a pure text
  scan, not a compile). On non-zero exit, set the script's overall
  `status=1` the same way the existing fmt/clippy/build/test steps do
  — follow the script's own established pattern exactly (search for
  how `status` is set on the existing steps' failure branches).
- R2 (Fix 1, part 2). Make `lint-files` print a one-line success
  summary on success (e.g. `lint-files: N files checked, max effective
  M lines (file: <path>)`) instead of printing nothing — this closes
  the observability gap fable's review noted (a gate log currently
  cannot distinguish "lint-files ran and passed" from "did not run
  at all"). Keep the existing failure-path output format unchanged.
- R3 (Fix 2). Change `xtask ci` (`xtask/src/main.rs`) and
  `scripts/mvp-verify.sh` to invoke `quality-gate.sh` for their
  cargo-related steps (fmt/clippy/build/test) instead of running their
  own separate cargo invocations in the shared target dir. If a clean
  swap to calling the script directly isn't straightforward (e.g. `ci`
  needs to do something quality-gate.sh doesn't), instead copy
  quality-gate.sh's exact target-dir isolation pattern into both
  callers rather than leaving the shared-target-dir race in place.
  Choose whichever is the smaller, cleaner diff and explain the choice
  in your report.
- R4 (Fix 3). Change `clippy.toml`'s `println` entry's `path` field
  to `"std::println"`. Re-run `cargo clippy --workspace --all-targets
  -- -D warnings` and confirm the `does not refer to a reachable
  macro` warning is gone and no NEW disallowed-macros violations
  surface (if any bare `println!` calls exist outside the crates
  already covered by `print_stdout = "deny"`'s existing `#[allow]`
  sites, they would now be caught — if this happens, report it, do
  not silently `#[allow]` your way past a newly-surfaced real
  violation).
- R5 (Fix 4). Fix `count_effective_lines` in `xtask/src/lint_files.rs`:
  while `in_cfg_test` is true and the accumulated brace depth is still
  `0` (i.e. no `mod tests {` or similar opening brace has been seen
  yet), continue consuming lines that start with `#[` as part of the
  attribute stack before beginning real brace-depth tracking. Add two
  new tests to whatever test module already covers this function (or
  create one if none exists) that FAIL on today's code: (a) a
  `#[cfg(test)] #[rustfmt::skip] mod tests { <N lines> }` fixture
  correctly excludes the N lines, (b) a bare `#[cfg(test)] fn helper()
  {}` (single-item form, no `mod`) is still handled correctly (this
  form already works per fable's review, but add a regression test
  for it anyway since it's the same code path).

### Non-Functional

- N1. None of these four fixes should change any file's actual
  effective-line COUNT except via Fix 4 (which corrects a
  miscounted file's reported number back down to its true value) —
  Fixes 1-3 are tooling/config only.
- N2. Fix 1's lint-files step must not slow down `quality-gate.sh`
  meaningfully — it's a text scan over already-checked-out files, not
  a build; confirm this holds after adding it (the whole gate should
  still run in roughly the same wall-clock time).
- N3. Do NOT change `MAX_EFFECTIVE_LINES`/`WARN_EFFECTIVE_LINES` (450/
  380) — this spec fixes the counting mechanism's correctness, not the
  thresholds.

## Architecture

Described inline in Overview above.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `count_effective_lines_handles_attribute_stack_before_cfg_test_body` | `#[cfg(test)]\n#[rustfmt::skip]\nmod tests {\n    fn a() {}\n}` | the 3 lines inside the module body are excluded (only counts toward 0, not toward production) |
| `count_effective_lines_handles_bare_cfg_test_fn` | `#[cfg(test)]\nfn helper() {}\nfn production() {}` | `helper`'s line is excluded, `production`'s line is counted |
| `failover_rs_effective_count_drops_after_fix` | `cargo run -q -p xtask -- lint-files` output/exit for `crates/pii-abi/src/failover.rs` specifically | reported effective count matches the file's true production-only size (verify the exact number after the fix; report it) |
| `clippy_toml_println_path_is_qualified` | `grep '"std::println"' clippy.toml` | present |
| `quality_gate_runs_lint_files` | `grep -c "xtask -- lint-files" quality-gate.sh` | >= 1 |

## Exit Criteria

- [ ] `grep -q "lint-files" quality-gate.sh`
- [ ] `grep -q 'path = "std::println"' clippy.toml`
- [ ] `cargo test -p xtask`
- [ ] `cargo run -q -p xtask -- lint-files` (must exit 0, confirming the whole workspace still passes the corrected counter)
- [ ] `bash quality-gate.sh` (the full gate, now including lint-files, must pass end to end)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` (confirms Fix 3's println change didn't surface a new real violation, and Fix 1's addition didn't break anything)
- [ ] `cargo fmt --check`

## Guardrails

- Do NOT change `MAX_EFFECTIVE_LINES`/`WARN_EFFECTIVE_LINES` thresholds.
- Do NOT modify any file under `specs/`.
- Do NOT silently `#[allow]` any NEW clippy violation Fix 3 (the
  `println` path fix) surfaces — if one appears, report it explicitly
  in your final summary rather than suppressing it.
- No emojis, no AI-tool mentions in any comment or commit message.
- If `xtask ci`/`mvp-verify.sh` have functionality `quality-gate.sh`
  genuinely doesn't cover (not just a different invocation shape of
  the same cargo commands), STOP and report the discrepancy rather
  than dropping that functionality to force a clean swap.

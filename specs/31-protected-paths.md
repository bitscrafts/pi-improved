# pi-improved — spec-31: protected-paths

- **Project**: `<repo>`
- **Priority**: HIGH — the single highest-risk gap in the boss loop. A worktree
  is a full checkout, so `<worktree>/xtask/`, `<worktree>/quality-gate.sh`,
  `<worktree>/specs/` and every `#[test]` are INSIDE the heal agent's fs_write
  scope (`crates/pii-server/src/runs.rs:232-255`). The cheapest way for any
  model to make `cargo test` green is to delete the assertion. Must land
  before spec 33 (Slice E live).

## Verified Premises

- `PathScope` (`crates/pii-core/src/cap.rs:13-57`) already has deny-wins
  semantics: `permits` returns false if the path is inside a deny prefix even
  when an allow prefix covers it. But `run_caps` (`runs.rs:255-283`) builds
  `fs_write` with ONLY an allow prefix (the worktree/workspace) and NO deny —
  so protected paths inside the worktree are writable.
- `FsCap::write`/`append` (`crates/pii-core/src/fscap.rs:97-151`) enforce
  `caps.fs_write.permits(&canonical)` — a deny entry would be enforced here.
- `xtask heal-collect` (`xtask/src/heal_collect.rs:1-40`) writes the worktree
  diff to `_tmp/heal-reviews/<sig>.patch` and applies nothing. It does NOT
  currently refuse a patch that touches protected paths or removes a test.
- `HealAttempt` (`crates/pii-store/src/heal.rs:14-28, 225-259`) has a
  `HealState` with `Escalated` and a `detail` field.
- `silent-noop-check` (`xtask/src/silent_noop.rs:139`) scans methods, events,
  config keys and graph-node keys — it can gain a "protected-path entry with
  no enforcement site" class.

## Overview

Add a protected-path deny list to the heal run's fs_write scope so the boss
cannot patch its own oracle (the gates that prove it worked), and make
`heal-collect` refuse a patch that touches a protected path or removes a test.
This is a capability (deny-wins in `PathScope`), not a policy an agent can
talk its way past.

## Requirements

### Functional

- **R1 (deny list in fs_write scope)**: a protected-path deny list applied at
  `FsCap::write`/`append` (`crates/pii-core/src/fscap.rs:97-151`), expressed
  through `PathScope`'s existing deny-wins semantics (`cap.rs:52-57`) so it is
  a capability, not a policy. Default set, relative to the run's `scope_root`:
  `specs/`, `xtask/`, `quality-gate.sh`, `graphs/`, `clippy.toml`,
  `rust-toolchain.toml`, `.github/`.
- **R2 (heal-collect refuses)**: `xtask heal-collect` refuses a patch that
  touches a protected path OR whose diff net-removes a `#[test]`,
  `assert!`/`assert_eq!` line, or adds `#[ignore]`. Exit 1 with the offending
  hunk.
- **R3 (refusal recorded)**: the refusal is recorded — the `HealAttempt`
  moves to `HealState::Escalated` with the reason in `detail`
  (`crates/pii-store/src/heal.rs:14-28, 225-259`).
- **R4 (silent-noop-check class)**: `silent-noop-check`
  (`xtask/src/silent_noop.rs:139`) gains a class: a protected-path entry with
  no enforcement site fails the gate.

### Non-Functional

- **N1**: the quality gate stays green; the test count only goes up.
- **N2**: no new third-party dependencies.
- **N3**: `silent-noop-check` (spec 24) stays green; new fields/types must have
  a producer.
- **N4**: the deny list is a default, overridable per-run (a future spec may
  need to allow a protected path deliberately).

## Architecture

The deny list is added to the `fs_write` `PathScope` in `run_caps`
(`runs.rs:255-283`) for scoped (heal) runs. `PathScope` already has deny-wins
semantics, so a path in the deny list is refused even though the worktree allow
prefix covers it. `FsCap::write`/`append` already call
`caps.fs_write.permits(&canonical)` — no change needed there beyond the deny
entries being present.

`heal-collect` gains a pre-write check: parse the diff, refuse (exit 1) if it
touches a protected path or net-removes a test/assert line.

## TDD Contract

| Test | Input | Expected Output |
|---|---|---|
| `fs_write_denies_protected_path_in_scope` (T1, acceptance) | ctx scoped to a temp worktree; write `<wt>/specs/foo.md` | `PiiError::Denied`; the file does not exist on disk afterwards |
| `heal_collect_refuses_test_deletion` (T2) | worktree diff that removes one `assert_eq!` line | exit 1, stderr names the file and the removed assertion; no `.patch` written |
| `fs_write_still_allows_source` (T3) | write `<wt>/crates/pii-core/src/x.rs` | Ok |

T1 is the acceptance test: a plausible implementation checks only the path
list and lets a test deletion through under `crates/`.

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-core protected_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — deny-list tests (R1)
- [ ] `cargo test -p pii-server protected_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — run_caps deny wiring (R1)
- [ ] `cargo test -p pii-store heal_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — Escalated recording (R3)
- [ ] `cargo test --workspace` — 0 failures (N1)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N1)
- [ ] `cargo fmt --all --check` — clean (N1)
- [ ] `cargo xtask silent-noop-check` — green (N4)

## Guardrails

- **G1 — do NOT edit this spec.**
- **G2 — do NOT commit.** Leave work in the working tree for orchestrator review.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — the deny list is a capability (deny-wins in `PathScope`), NOT a
  prompt-level policy.**
- **G5 — do NOT touch `specs/`** (read-only for implementers).

## Files to Modify

- `crates/pii-core/src/cap.rs` — add a `with_deny`/`deny` builder to
  `PathScope` if not present.
- `crates/pii-server/src/runs.rs` — `run_caps` adds the protected-path deny
  list to the `fs_write` scope for scoped runs.
- `xtask/src/heal_collect.rs` — refuse protected-path/test-deletion patches.
- `crates/pii-store/src/heal.rs` — record the refusal as `Escalated` (R3).
- `xtask/src/silent_noop.rs` — add the protected-path enforcement class (R4).
- Tests: `crates/pii-core/tests/`, `crates/pii-server/tests/`,
  `crates/pii-store/tests/` for the TDD contract.

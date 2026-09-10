# pi-improved — spec-34: slice-f-dev-prod-promotion

- **Project**: `/projects/pi-improved`
- **Priority**: MEDIUM — Slice F: dev/prod split and a gated, collectable
  promotion artifact. Deterministic, no model.

## Overview

008 §6 says *"the boss may build and test its own patch, but must never
replace the running daemon binary. Deploy stays human, permanently."* So
Slice F is **not** "the boss deploys." It is: build the patched version in
the worktree, run it as a second daemon against a separate runtime dir,
soak it, gate it, and stop at a signed, human-applicable artifact.

## Requirements

### Functional

- **R1 (dev runtime dir isolation)**: `[core] runtime_dir` already exists
  (`config.rs:43`). A dev daemon launched with
  `runtime_dir = /root/.pii-dev/runtime` gets its own `control.sock`,
  `store.redb`, `heal.redb`, `approval.redb`, `memory.redb`. Prove
  isolation: a run started on dev never appears in prod's `run.list`.
- **R2 (promote-build xtask)**: a `promote-build` xtask step:
  `cargo build --workspace` **inside the worktree**, then start the built
  binary against the dev runtime dir, run a fixed soak graph
  (`graphs/starter/soak.toml`), and record pass/fail. Never touches
  `/root/.pii/`.
- **R3 (runtime-cycle promotion gate)**: the promotion gate is a `gate`
  node with a **runtime** cycle (`gate.rs:83-87` reads
  `input.payload.cycle`; spec 26's mechanism), so re-entering the loop
  mints a fresh `approval_id` (`gate.rs:110-126`) and requires a fresh
  human decision.
- **R4 (collect on allow)**: on `allow`, the terminal action is
  `heal-collect` (a `.patch` under `_tmp/heal-reviews/`) plus a
  `HealAttempt.decision = {allow, approval_id, by, ts_ms}` bound to the
  gate's id. **Nothing writes the live tree, restarts prod, or merges.**
- **R5 (provenance)**: the worktree branch is `heal/<sig>` (already so,
  `heal_seed.rs:81-101`) and the collected patch carries a
  machine-identifiable trailer, so a human can revert a class of boss
  changes at once (008 §6).

### Non-Functional

- **N1**: the quality gate stays green; the test count only goes up.
- **N2**: no new third-party dependencies.
- **N3**: `silent-noop-check` (spec 24) stays green.
- **N4**: the live tree is never modified by a promotion cycle.

## Architecture

A dev daemon is a second `pii daemon` instance with a distinct
`[core] runtime_dir`. The `promote-build` xtask builds the worktree,
launches the dev daemon, runs the soak graph, and records pass/fail. The
promotion gate (a `gate` node with a runtime cycle) mints a fresh
`approval_id` per cycle; on `allow`, `heal-collect` writes the `.patch`
and the ledger records the decision.

## TDD Contract

| Test | Given | Expects |
|---|---|---|
| `dev_runtime_dir_is_isolated` (T1) | two engines, distinct runtime dirs; start a run on dev | prod's `run.list` returns 0 rows; dev's returns 1 |
| `promotion_gate_requires_fresh_approval_each_cycle` (T2) | two-cycle loop through the promotion gate, cycle 1 approved | cycle 2 **suspends** with a *different* `approval_id` |
| `deny_produces_no_patch_artifact` (T3) | promotion gate denied | `_tmp/heal-reviews/<sig>.patch` does not exist; ledger `decision.allow == false` |
| `promotion_never_writes_live_tree` (T4) | full promotion cycle in a temp workspace | `git status --porcelain` on the live tree is empty; assert no path outside `_tmp/` was modified |

T3 is the acceptance test — a plausible implementation collects the patch
*before* the gate resolves.

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-server dev_runtime_dir_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — isolation (R1)
- [ ] `cargo test -p pii-server promotion_gate_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — fresh approval (R3)
- [ ] `cargo test -p pii-server deny_produces_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — no patch on deny (R4)
- [ ] `cargo test -p pii-server promotion_never_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — live tree untouched (N4)
- [ ] `cargo test --workspace` — 0 failures (N1)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N1)
- [ ] `cargo fmt --all --check` — clean (N1)
- [ ] `cargo xtask silent-noop-check` — green (N3)

## Guardrails

- **G1 — do NOT edit this spec.**
- **G2 — do NOT commit.** Leave work in the working tree for orchestrator review.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — the live tree is never modified by a promotion cycle** (008 §6).
- **G5 — do NOT touch `specs/`** (read-only for implementers).
- **G6 — a denied promotion must NOT produce a patch artifact** (T3).

## Files to Modify

- `xtask/src/` — the `promote-build` step (R2).
- `crates/pii-server/src/nodes/gate.rs` — runtime-cycle promotion gate (R3).
- `crates/pii-store/src/heal.rs` — `HealAttempt.decision` (R4).
- `crates/pii-server/tests/` — the TDD contract tests.

# pi-improved — spec-33: slice-e-live

- **Project**: `<repo>`
- **Priority**: HIGH — the first spec that lets a model patch with write
  capability (Slice E live). `graphs/starter/heal-fix.toml` is already a
  complete localize→patch→validate→escalate loop (spec 08); what it lacks is
  model routing, structural reproduction/validation enforcement, ledger
  population, and an end-to-end script. Must land AFTER spec 31 (protected
  paths) — that is the one ordering where getting it wrong lets a model delete
  the test that proves it worked.

> **Amendment (planner, 2026-09-16, Phase 5.0.7 / F-03,F-04):** R5 and its
> exit criterion below were edited to reflect ground truth —
> `scripts/phase4.7/slice-e2e.sh` was never shipped, yet the spec was
> committed DONE at `29625cf` with an unverifiable exit criterion pointing at
> a nonexistent script (026 report F-04). This edit is adopted here as a
> planner-authored change per AGENTS.md §"specs/ are READ-ONLY for the
> workhorse" — the prior working-tree diff was factually correct but had no
> owner (026 report F-03); it now does. No further edits to R1–R4 or the TDD
> Contract were made.

## Verified Premises

- `graphs/starter/heal-fix.toml` exists and is complete: `probe` (bash, runs
  `$.input.test_command` in `$.input.worktree`) → `route_probe` on exit code →
  `fix` (agent, `tools = [fs_read, fs_write, edit, multi_edit]`, no bash) →
  `retest` → `route_retest` → `summarize` (git diff) → `escalate` (gate,
  `heal-escalation`) → `done`, with a `retry` back-edge and `loop_budget = 2`.
- `route_step` is wired into `AgentNode` (spec 28): `resolve_model`
  (`crates/pii-server/src/agent.rs:77-96`) is loud (`NotFound`) on an unassigned
  step. `KNOWN_STEPS` (`config.rs:108-109`) already contains
  `localize`/`patch`/`review`/`research`/`orchestrator`.
- `HealAttempt` (`crates/pii-store/src/heal.rs:66-116`) already carries
  `gate_before`/`gate_after`/`patch_hash`/`model`/`tokens_in`/`tokens_out` —
  today nothing populates them.
- `heal_detect` (`crates/pii-server/src/nodes/heal.rs:123-165`) already
  reproduces by running the gate twice before recording a ledger row.
- `route_probe` already routes exit-0 to `unexpected_ok` (a gate) — a break
  that doesn't reproduce escalates.

## Overview

Make Slice E live: the `fix` agent runs on a routed model (`route_step =
"patch"`), reproduction-before-patching and validation-before-promotion are
enforced structurally (not by prompt), the attempt is written to the ledger
with all its fields, and an end-to-end script drives the whole
seed→detect→spawn→heal-fix→approve→collect→prune cycle with the main tree
byte-identical before and after.

## Requirements

### Functional

- **R1 (routed model)**: `heal-fix.toml`'s `fix` agent declares
  `route_step = "patch"`; a new `localize` agent (or the existing `probe`
  output feeding `fix`) declares `route_step = "localize"`. Resolution goes
  through `resolve_model` (`crates/pii-server/src/agent.rs:77-96`), which is
  loud (`NotFound`) on an unassigned step.
- **R2 (reproduction enforced)**: reproduction before patching is enforced
  structurally, not by prompt: `route_probe` already routes exit-0 to
  `unexpected_ok` (a gate), so a break that doesn't reproduce escalates. Add a
  test that proves it.
- **R3 (validation before promotion)**: `retest` runs the SAME `test_command`
  in the SAME worktree; only `route_retest.ok` reaches `summarize`/`escalate`.
  A still-broken retest after `loop_budget` must NOT reach the human gate.
- **R4 (ledger populated)**: the attempt is written to the ledger with
  `gate_before`/`gate_after`/`patch_hash`/`model`/`tokens_in`/`tokens_out`
  (all fields already exist, `crates/pii-store/src/heal.rs:66-116`) — today
  nothing populates them.
- **R5 (e2e behaviour, script NOT yet shipped)**: spec 33 landed with structural
  reproduction/validation/ledger via the `slice_e_live.rs` test battery (R1–R4),
  but the standalone driver `scripts/phase4.7/slice-e2e.sh` (seed → detect →
  spawn → heal-fix → approve → collect → prune, main tree byte-identical to
  HEAD before and after) **does not exist** — tracked as a known gap, not a
  landed requirement. The e2e cycle it was to drive is only partially asserted
  by the in-crate tests; a full tree-byte-identical cycle remains unproven.

### Non-Functional

- **N1**: the quality gate stays green; the test count only goes up.
- **N2**: no new third-party dependencies.
- **N3**: `silent-noop-check` (spec 24) stays green; new fields/types must have
  a producer.
- **N4**: the main tree is never modified by a heal attempt (worktree-only).

## Architecture

`heal-fix.toml` gains `route_step = "patch"` on the `fix` agent (and a
`localize` step if a separate agent is added). The `[evolve.routing]` config
maps `patch`/`localize` to model specs. The ledger population happens in the
`fix`/`retest`/`escalate` nodes (or a small helper) writing the attempt's
`gate_before`/`gate_after`/`patch_hash`/`model`/`tokens` fields.

## TDD Contract

| Test | Input | Expected Output |
|---|---|---|
| `heal_fix_resolves_patch_model` (T1) | `[evolve.routing] patch = "X"`, load real `heal-fix.toml` | the `fix` node's resolved model == `"X"`; a `RouteDecision{step:"patch", model:"X"}` event on the bus |
| `unreproduced_break_escalates` (T2) | scripted probe exiting 0 | run suspends on `unexpected_ok`, NO agent node ever starts (assert the provider call count == 0) |
| `still_broken_after_loop_budget_does_not_reach_gate` (T3) | scripted model that never fixes; `loop_budget = 2` | run terminates without `escalate` ever starting, and the ledger row is `Escalated`, not `Fixed` |
| `ledger_records_gate_before_and_after` (T4) | scripted successful repair | `HealAttempt{gate_before:"fail", gate_after:"pass", patch_hash: Some(_)}` |

T3 is the one a plausible implementation gets wrong: it will let the retry
loop fall through to the human gate with a still-red test, which would put an
unvalidated patch in front of an approver — a direct 008 §6 violation.

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-server heal_fix_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — routed model + reproduction (R1, R2)
- [ ] `cargo test -p pii-server still_broken_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — loop-budget gate (R3)
- [ ] `cargo test -p pii-server ledger_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — ledger population (R4)
- [ ] `bash scripts/phase4.7/slice-e2e.sh` — e2e cycle (R5) — **NOT SHIPPED**: script absent; replace with the in-crate structural proof that landed: `cargo test -p pii-server --test slice_e_live 2>&1 | grep -qE "test result: ok\. [1-9]"` (R1–R4)
- [ ] `cargo test --workspace` — 0 failures (N1)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N1)
- [ ] `cargo fmt --all --check` — clean (N1)
- [ ] `cargo xtask silent-noop-check` — green (N3)

## Guardrails

- **G1 — do NOT edit this spec.**
- **G2 — do NOT commit.** Leave work in the working tree for orchestrator review.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — the main tree is never modified by a heal attempt** (worktree-only).
- **G5 — do NOT touch `specs/`** (read-only for implementers).
- **G6 — a still-broken retest after `loop_budget` must NOT reach the human
  gate** (008 §6).

## Files to Modify

- `graphs/starter/heal-fix.toml` — `route_step = "patch"` on `fix` (R1).
- `crates/pii-server/src/nodes/heal.rs` — ledger population (R4).
- `crates/pii-server/tests/` — the TDD contract tests.
- `scripts/phase4.7/slice-e2e.sh` — the e2e script (R5). **NOT YET SHIPPED** (does not exist; the R1–R4 structural proof lives in `crates/pii-server/tests/slice_e_live.rs`).

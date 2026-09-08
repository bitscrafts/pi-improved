# pi-improved — spec-20: phase4-boss-approval-gate-fix

- **Project**: `/projects/pi-improved`
- **Priority**: P4.7 boss prerequisite — the opus review's blocking finding: the human approval gate is NOT a real gate once a run resumes, which would void the user's "human approves promotions" decision for all of Boss Slice F.
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
  on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
  commits and updates HANDOFF.md)*
- **Source**: opus review `_docs/06-research/009-pii-boss-opus-review-pass1.md` items 1-3 (2026-09-08)
- **Depends-On**: none — this is the prerequisite fix that Slice F (promotion) and Slice B (escalation) depend on.

---

## Verified Premises

- `crates/pii-server/src/runs.rs:111-146` — `spawn_resume` builds the resumed `Ctx` with `run_caps(&engine.cfg.workspace, None, engine.ollama_net_host.as_deref())` — **`scope_root` is hardcoded `None` on resume**, so a scoped (heal/promotion) run resumes with WORKSPACE-scoped fs_write, widening write authority to the whole repo. Confirmed.
- `crates/pii-server/src/runs.rs:176-199` — `run_caps` takes `scope_root: Option<&Path>`; when `Some`, `fs_write` is confined to exactly `scope_root`. So restoring `scope_root` on resume is a matter of passing it, not new machinery.
- `crates/pii-server/src/runs.rs:37-44` — `spawn_run` reads `scope_root` from `extra_input["scope_root"]` and validates it. The RESUMED run's input (including `scope_root`) is recoverable from the journal's `RunEvent::Started { input_hash }` at `crates/pii-graph/src/run.rs:140-151` (resume re-reads it).
- `crates/pii-server/src/nodes/gate.rs:63-82` — `GateNode::run` computes `approval_id(ctx)` (`blake3(run_id, node_id)[..16]`, gate.rs:88-103), then if the store already has a `decision` for that id, AUTO-ROUTES to `allow`/`deny` WITHOUT suspending. On a resume of the same run, the id matches the earlier approval → auto-allows, never re-asks the human.
- `crates/pii-server/src/nodes/gate.rs:75-81` — when no decision exists, it `Suspend`s with `ResumeKey(format!("approval:{id}"))`. This is the deterministic-replay property (§8.8).
- `graphs/starter/heal-watch.toml:23` — `gate = "quality-gate"` inside a `heal_detect` node. `heal_detect` runs `cargo xtask <gate>` (`crates/pii-server/src/nodes/heal.rs:50`), but `quality-gate` is NOT an xtask subcommand — `xtask/src/main.rs:172-175` falls through to `USAGE` and exits 2. **The shipped watch graph records a phantom permanent failure.** Confirmed.

---

## Overview

The opus adversarial review found that the boss's entire safety model — "a human approves every promotion" — is silently voided by two pre-existing bugs:

1. **A scoped run does not resume scoped.** `spawn_resume` drops `scope_root`, so a heal/promotion run whose write authority was deliberately confined to a disposable worktree resumes with workspace-wide `fs_write`. Every Boss Slice that writes after an approval (all of Slice F) would be unconfined.
2. **An approved gate auto-allows forever.** `approval_id = blake3(run,node)` + a cached `decision` means a gate re-entered on a later cycle reads its OLD approval and routes `allow` without ever asking a human again. The "approval" is effectively one-time-and-sticky, which is the opposite of what promotion needs (every promotion must be a fresh human decision).

Plus a third, independent bug: `heal-watch.toml` names a non-existent xtask gate, so the shipped detection graph reports a permanent phantom failure.

This spec fixes all three so the boss's human-approval contract actually holds.

---

## Requirements

### Functional

#### R1 — Resumed scoped runs stay scoped
`spawn_resume` must recover the run's `scope_root` (from the resume-reconstructed run input) and pass it to `run_caps`, so a run that originally started with a confined `fs_write` resumes with the SAME confinement. A run that never had a `scope_root` resumes workspace-scoped exactly as today (byte-identical behavior for non-scoped runs).

Implementation shape: `pii_graph::run::resume` already reconstructs the run input (incl. `scope_root`) at `run.rs:140-151`. `spawn_resume` can either (a) recover `scope_root` from that same journal->input path before building the `Ctx`, or (b) thread it out of `resume` if a cleaner seam exists. Either is acceptable; the REQUIRED contract is: **a run whose `Started` input carried `scope_root` resumes with that same `scope_root`**. `validate_scope_root` must still reject a resumed `scope_root` outside the workspace (defense in depth).

#### R2 — Promotion gates ask a human every cycle
The `gate` node must not auto-route on a STALE approval across separate decision cycles. The exact rule:
- A genuine in-run resume of the SAME approval (deterministic replay: `suspend` → `approve` → `resume` → gate reads the just-made decision) MUST keep working — that is the `§8.8` determinism property and must NOT be broken.
- But a NEW decision request — a second attempt/cycle at a `gate` whose prior decision came from an EARLIER attempt of a DIFFERENT run, or a re-run of the same graph where a fresh human decision is required — MUST suspend again rather than reuse the old `allow`.

The clean fix: **salt `approval_id` with a per-attempt/cycle counter** so each new decision attempt gets a fresh id (no cached decision → suspends), while an in-flight suspend→resume within the same attempt keeps the same id (idempotent replay). The salt source: the run's current journal seq at the time the gate is first entered (a counter that advances per attempt), or an explicit per-cycle value passed in the graph input. The implementation MUST keep the invariant that an in-progress approval resume re-reads its own decision; it MUST break the invariant that a NEW attempt reuses an old run-node's decision.

#### R3 — `heal-watch.toml` names a real gate
Change `graphs/starter/heal-watch.toml:23` from `gate = "quality-gate"` (NOT an xtask subcommand — verified: `xtask/src/main.rs:101-172` has `deps-check`,`net-check`,`reachability-check`,`doc-drift-check`,`rss-gate`,`lint-files`,`bench-gate`,`ci`, but NO `quality-gate`, so `cargo xtask quality-gate` falls to `USAGE` exit 2) to a REAL one. Use `reachability-check` (the deterministic self-improvement gate, main.rs:103 — the opus review's recommended first slice). Do NOT ship a graph that records a phantom failure.

### Non-Functional

- **N1**: A run with NO `scope_root` must resume byte-identical to today — R1 is purely additive confinement restoration, no behavior change for non-scoped runs.
- **N2**: The gate's `§8.8` deterministic-replay property (suspend → approve → resume → same decision) is preserved. R2 only breaks STALE decision reuse across new attempts.
- **N3**: quality gate stays green; test count only goes up.
- **N4**: `xtask` gates stay green.

---

## Architecture

```
spawn_run (new run)
  input = { scope_root: "/projects/pi-improved/_tmp/heal-wt-1", ... }
     |
     v
  ctx = run_caps(workspace, scope_root, net_host)   // fs_write = scope_root only
     |
     v
  ... gate suspends with approval_id = blake3(run, node, attempt)   [R2]
     |
     v  human approves
  suspend -> resume
     |
     v
spawn_resume  -- MUST recover scope_root from the run input      [R1]
  ctx = run_caps(workspace, recovered_scope_root, net_host)       // stays scoped
     |
     v
  gate: id matches the just-made decision (same attempt) -> allow  // replay OK
  gate: NEW attempt, salt differs -> STALE id absent -> SUSPEND again [R2]
```

**Key decision — recover `scope_root` from the journal, not a new header field.** The resume path already reconstructs the full run input (incl. `scope_root`) from `RunEvent::Started { input_hash }` (`run.rs:140-151`). Reusing that avoids a `RunHeader` schema migration, keeps the store unchanged, and guarantees the resumed scope matches the ORIGINAL run (source of truth is the journal, the same place replay gets everything else). Rejected: adding `scope_root` to `RunHeader` (schema bump, migration, and risks drift from the actual started input).

**Key decision — salt `approval_id` with an attempt counter, not the run id alone.** The current `blake3(run,node)` is why a later cycle reuses an old decision. Adding a per-attempt salt means: the FIRST entry of an attempt makes a fresh id (no cached decision → suspend); the in-flight resume (`approve` → `resume`) re-derives the SAME id from the SAME attempt counter → sees the decision → correct replay; the NEXT attempt's different counter → fresh id → asks again. Rejected: removing the cached-decision lookup entirely (breaks the §8.8 determinism the whole suspension/resume design is built on).

**What this spec is not**: the boss loop, model routing, dev/prod promotion, research, GitHub. Those are later Boss slices.

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R1 | `resume_restores_scope_root` (pii-server integration) | a run started with `scope_root` confined to a worktree, suspended, then resumed | the resumed `Ctx` has `fs_write` confined to the same worktree (a write to a path OUTSIDE the worktree but inside the workspace is `Denied`) |
| R1 | `resume_unscoped_stays_unscoped` | a run with NO `scope_root`, suspended, resumed | resumes workspace-scoped exactly as a fresh `spawn_run` (no confinement added) |
| R1 | `resume_rejects_scope_outside_workspace` | a run whose journal input claims a `scope_root` outside the workspace | resume fails with `Invalid`, not a silently permissive scoped run |
| R2 | `gate_suspends_on_new_attempt` | a gate `allow`'d in attempt 1 (cycle/salt N), run again as a new attempt (salt N+1) | the second attempt SUSPENDS again (does not auto-allow from the stale decision) |
| R2 | `gate_replays_same_attempt_decision` | suspend→approve→resume within ONE attempt (same salt) | gate routes `allow` from the just-made decision (no re-suspend) — §8.8 preserved |
| R2 | `approval_id_changes_with_attempt` (pii-server unit) | `approval_id(ctx)` at two different attempt counters | distinct ids (they must differ) |
| R3 | `heal_watch_gate_is_real_xtask` (shell/static) | `graphs/starter/heal-watch.toml`'s `gate` value | the value is a subcommand in `xtask/src/main.rs` (passes `cargo xtask <value> --help` or is in the match arms); the phantom `quality-gate` is gone |

**R2 (`gate_suspends_on_new_attempt`) is the acceptance test.** The requirement a plausible implementation gets wrong: it removes the cached-decision lookup entirely (breaking §8.8 replay) OR it keeps `blake3(run,node)` with no salt (so new attempts still auto-allow — the bug doesn't actually get fixed). The test asserts BOTH: a new attempt suspends, AND the same-attempt replay still routes.

---

## Exit Criteria

- [ ] `cargo build --workspace --all-targets` — compiles (R1-R3)
- [ ] `cargo test -p pii-server resume_restores_scope_root` — scoped resume (R1)
- [ ] `cargo test -p pii-server gate_suspends_on_new_attempt` — fresh approval per attempt (R2)
- [ ] `cargo test -p pii-server gate_replays_same_attempt` — §8.8 preserved (R2)
- [ ] `cargo test -p pii-server approval_id_changes_with_attempt` — salt works (R2)
- [ ] `cargo test --workspace` — 0 failures (N3)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N3)
- [ ] `cargo fmt --all --check` — clean (N3)
- [ ] `cargo xtask lint-files` — 0 hard errors (N3)
- [ ] `cargo xtask deps-check` — pass (N4)
- [ ] `cargo xtask net-check` — pass (N4)
- [ ] `cargo xtask reachability-check` — pass (the R1 change touches a call path)

**Prose criteria**:

1. Test counts pasted raw, one line per binary, **unsummed**.
2. The R2 test must prove the fix is real: a NEW attempt suspends (the old `blake3(run,node)` behavior would auto-allow — that test would FAIL on the pre-fix code). Require the wrong-version-first proof: write the "no salt" version, watch `gate_suspends_on_new_attempt` fail, then add the salt.

---

## Guardrails

- **G1 — do NOT edit this spec.** If it is wrong, STOP and report it.
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT regenerate a pinned fixture.**
- **G5 — no hardcoded absolute paths.** Test artefacts under `_tmp/`.
- **G6 — report raw output, never summed totals.**
- **G7 — do NOT break the §8.8 deterministic-replay property.** An in-progress approval resume must re-read its own decision. R2 is about STALE decisions across NEW attempts, not about removing decision caching.
- **G8 — do NOT add `scope_root` to `RunHeader`.** Recover it from the journal input (`run.rs:140-151` is the seam). No store schema change.
- **G9 — do NOT change non-scoped resume behavior.** A run without `scope_root` resumes byte-identical (N1).

### Error handling expectations

- A resumed `scope_root` outside the workspace must fail with `Invalid`, never a silently permissive scoped run.
- The heal-watch gate value must be a REAL xtask subcommand — shipping a graph that records a phantom failure is a bug, not an acceptable default.
- The attempt salt must be monotonic per attempt so two attempts never collide on the same `approval_id`.

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/pii-server/src/runs.rs` | `spawn_resume` recovers `scope_root` from the run input and passes it to `run_caps` (R1) |
| `crates/pii-server/src/nodes/gate.rs` | salt `approval_id` with a per-attempt counter (R2) |
| `graphs/starter/heal-watch.toml` | `gate` → a real xtask subcommand (R3) |

**Not modified**: `crates/pii-abi/src/journal.rs` (no `RunHeader` change), `crates/pii-graph/src/run.rs` (resume already reconstructs the input; expose the seam if needed without changing replay semantics), `crates/pii-store/` (no schema change).

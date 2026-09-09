# pi-improved — spec-23: phase4-pii-evolve

- **Project**: `/projects/pi-improved`
- **Priority**: P4.7 — the self-improving "boss" module. Capstone of Phase 4.
  The user directed: "a pii sub module", "crate!", "pii-evolve", "make it
  pluggable! allowing the installer to include it or not".
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN
  PROGRESS on dispatch → IMPLEMENTED — awaiting review → DONE after the
  orchestrator commits and updates HANDOFF.md)*
- **Source**: `_docs/06-research/008-pii-evolve.md` (deep design) +
  `_docs/06-research/009-pii-evolve-opus-review-pass1.md` (opus review +
  USER OVERRIDE: dedicated feature-gated crate)
- **Depends-On**: spec 20 (resume scope-root + per-cycle approval gate),
  spec 21 (pii-mcp), spec 22 (agent tool loop) — all landed.

---

## Verified Premises

- `_docs/06-research/009-pii-evolve-opus-review-pass1.md:79-102` — USER
  OVERRIDE: `pii-evolve` is a FIRST-CLASS **dedicated crate**, feature-gated,
  default OFF. It owns the boss loop, state.md management, context compaction,
  and model routing. It builds on pii-graph + pii-core/pii-server (journal,
  gates, approvals, budget, memory) — same substrate rule as every other crate.
  The opus prerequisite bugs (resume scope-root, approval-gate stale reuse,
  heal-watch phantom gate) already land in `pii-server`/`graphs` (specs 20, 22).
- `_docs/06-research/008-pii-evolve.md:160-168` — build order: Slice C
  (context mgmt: state.md writer, compaction-chain, external-memory), Slice D
  (model routing), Slice E/G/H (patch/validate, dev/prod promotion, research).
- `_docs/02-guides/feature-gating.md` — convention for opt-in crates. The
  working gate in this repo: the crate is a workspace member with its own bin,
  but is NOT a dependency of `pii-cli`, so the default build links none of it.
  Examples: `pii-web` (axum), `pii-mcp` (rmcp). `pii-evolve` follows the same
  shape.
- `crates/pii-server/src/compaction.rs` — `estimate_tokens`, `needs_compaction`,
  `compact` already exist (spec 14). `pii-evolve` REUSES this, doesn't
  reimplement.
- `crates/pii-server/src/agent.rs` — `AgentNode::run`, `session.prompt`, the
  turn loop. `pii-evolve` drives these to *be* the boss.
- `crates/pii-memory/src/search.rs` — `MemorySearcher::search` (P4.3). The
  boss's "external memory as source of truth".
- `crates/pii-core` — `Ctx`, `Budget`, `CapSet`, `CapKind`. The confine-and-budget
  substrate every boss action runs under.

---

## Overview

`pii-evolve` is the **self-improving boss**: a cohesive subsystem that checks
logs, detects problems, localizes + patches + validates them in a disposable
worktree, maintains a parallel dev/prod split, manages its OWN context
(durable `state.md` + compaction chains + external memory as source of truth),
routes each step to a specific model tier, and periodically researches new
tech. It runs on Pii's OWN substrate (a pii graph) — dogfooding.

Per the USER OVERRIDE, it is a **dedicated, feature-gated crate** (default
off) so an installer that doesn't want self-improvement builds none of it — no
code, no tokens, no background loop. The risk surface (writes code, spends
tokens) is opt-in by construction.

This spec delivers the **crate foundation + the three novel mechanisms** that
graphs cannot express: state.md management, context compaction, and model
routing. The boss LOOP itself is a pii graph (already in the substrate);
`pii-evolve` provides the logic the graph's agent nodes delegate to.

---

## Requirements

### Functional

- **R1 (pii-evolve crate)**: a new `crates/pii-evolve` crate — a workspace
  member with its OWN bin (`pii-evolve`), NOT a dependency of `pii-cli` (so
  the default build links none of it — the established opt-in shape of
  `pii-web`/`pii-mcp`). It declares no new third-party deps not already in the
  workspace. Crates it may build on: `pii-abi`, `pii-core`, `pii-graph`,
  `pii-server`, `pii-memory`, `pii-proto`, `pii-store`, `pii-tools`.
- **R2 (state.md management)**: a `StateDoc` type that reads/writes a
  structured `state.md` in a given state dir. Fields (the "where am I"
  source of truth, NOT inferred from history): `goal`, `phase`, `last_step`,
  `next_step`, `properties` (a small key/value map for decisions + model-routing
  learnings), `updated_at`. `StateDoc::load(dir)` (defaults when absent),
  `.save()`, `.touch()` (bump updated_at), and a `.summarize()` producing the
  compact system-prompt fragment ANTHROPIC/Zylos recommend for compaction.
  Round-trip test: write → re-read → field equality, and monotonic updated_at.
- **R3 (context compaction)**: a `context` module that, given the current
  transcript size and a `Compaction` policy (threshold fraction, default
  ~0.70-0.75 per Zylos, NOT 0.95), decides whether to compact and produces a
  compaction SUMMARY that captures **decisions and outcomes, not dialogue**.
  Reuses `pii-server::compaction::{estimate_tokens, needs_compaction}` where
  possible; supplies the decision→outcome summary writer. Test: at 0.40 of a
  window it does NOT compact; at 0.80 it does; the summary omits dialogue
  verbatim and lists the decision.
- **R4 (model routing)**: a `routing` module with an explicit, auditable
  `RoutePolicy` — a per-step→model-tier table. Tiers: `{localize, patch,
  review, research, orchestrator}`. Each maps to a model spec (the doc's
  hierarchy: localize=scout fast/cheap, patch=strong worker, review=DIFFERENT
  strong model for diversity, research=planner, orchestrator=boss). A `route`
  function resolves a step → model spec, and a `RouteLog` records every
  decision (step, tier, model, ts) so a misroute is auditable. Test: explicit
  table resolution, review != patch model when configured, and the log
  captures the decision.
- **R5 (boss driver)**: a `driver` module with the boss OUTER loop skeleton:
  `run_cycle(state_dir, policy)` that (1) loads state.md, (2) runs the gate
  checks (reachability-check + doc-drift-check via xtask — the deterministic
  self-improvement gates from spec 22), (3) if a gate is red, records the
  failure signature + `needs_attention` in the state doc and returns a
  `Result::PendingHuman` (the boss SUSPENDS for human acknowledgement — never
  auto-promotes), (4) always persists state.md + bumps a cycle counter. The
  driver does NOT patch or write code in this spec — that is the graph/agent
  slice. It proves the dogfooding skeleton (check → record → suspend) and gives
  the boss a real, deterministic thing to detect. Test: a green fixture returns
  Ok; a fixture with a deliberately failing check returns PendingHuman and
  persists `needs_attention`.
- **R6 (feature gate invariants)**: `pii-evolve` is NOT a dependency of
  `pii-cli`, `pii-server`, or any non-evolve crate. `deps-check` must accept
  it as an isolated standalone crate (like pii-mcp/pii-web). The default
  `cargo build --workspace` still links none of `pii-evolve` into `pii`.

### Non-Functional

- **N1**: the quality gate stays green; the test count only goes up.
- **N2**: `xtask` gates stay green (deps-check/net-check/lint-files).
- **N3**: 450-line file cap, traits-first, `#![forbid(unsafe_code)]` (matches
  every crate except pii-hw/pii-memory).
- **N4**: NO action in this spec writes to `specs/`, `xtask/`, `quality-gate.sh`,
  `graphs/`, `.github/`, or weakens a `#[test]` — that's the
  anti-reward-hacking rule (opus item 5). The boss SUSPENDS for human at every
  promotion/attention point; it does not self-approve.
- **N5**: no new third-party dependencies. Reuse the workspace's existing
  serde/serde_json/tokio/thiserror/parking_lot.

---

## Architecture

```
crates/pii-evolve/            (workspace member, standalone bin, NOT in pii-cli)
├── Cargo.toml                # deps: pii-core, pii-server, pii-memory (all path)
├── src/
│   ├── main.rs               # `pii-evolve <state-dir> [--policy FILE]` — run_cycle
│   ├── lib.rs                # pub mod state, context, routing, driver
│   ├── state.rs              # StateDoc (R2)
│   ├── context.rs            # CompactionPolicy + decision→outcome summary (R3)
│   ├── routing.rs            # RoutePolicy + RouteLog (R4)
│   └── driver.rs             # run_cycle skeleton (R5)
└── tests/                    # integration tests (R2-R5 round trips)
```

The boss LOOP (check logs → detect → localize → patch → validate → deploy →
learn → research) that runs on a schedule is a **pii graph** (substrate) whose
agent nodes delegate their context/routing decisions to this crate. This spec
builds the crate + the state/context/routing machinery + the outer-loop
skeleton that any future graph wires to.

**Key decisions:**
- **state.md = source of truth, memory = durable store.** The boss's "where am
  I" lives in `state.md` (written at every milestone); durable facts (lessons,
  resolutions, model learnings) go to `pii-memory`, NEVER to the compaction
  summary (ANTHROPIC/Zylos).
- **Compaction at ~70-75%, not 95%** (Zylos), designed for 10+ compactions over
  a long run; the summary keeps decisions+outcomes, drops dialogue.
- **Model diversity catches mistakes** (KissAPI): patch and review use
  DIFFERENT models; escalate to human after 2 failed attempts on one signature.
- **Deterministic gates first**: the boss's first job is to run
  reachability-check + doc-drift-check — the no-model gates that convert
  regressions into machine-checkable facts (the strongest self-improvement
  machinery per opus). Patching/validation comes in the graph slice.

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R2 | `state_doc_round_trips` | write a StateDoc to a temp dir, re-read | fields equal, updated_at monotonic |
| R2 | `state_doc_summarizes_to_fragment` | a populated StateDoc | `.summarize()` starts with a goal line and includes `next_step` |
| R3 | `compaction_small_window_does_not_fire` | transcript tokens at 0.40 of window | `should_compact` == false |
| R3 | `compaction_large_window_fires` | transcript tokens at 0.80 of window | `should_compact` == true (threshold is ~0.70-0.75) |
| R3 | `summary_keeps_decisions_omits_dialogue` | a transcript of dialogue + one decision | summary contains the decision, not verbatim dialogue |
| R4 | `route_resolves_step_to_tier` | a RoutePolicy | each step resolves to its configured tier/model |
| R4 | `review_uses_different_model_than_patch` | policy configures patch=X, review=Y | `route(review) != route(patch)` |
| R4 | `route_log_captures_decision` | perform N routed steps | RouteLog has N entries with step/tier/model/ts |
| R5 | `green_fixture_returns_ok` | a state dir whose gate checks pass | `run_cycle` Ok, cycle counter bumped, state persisted |
| R5 | `red_fixture_suspends_for_human` | a state dir whose gate check fails | returns `PendingHuman`, `needs_attention` persisted |

**Exit criteria** (each runs >=1 test — non-vacuity verified by count):

- [ ] `cargo build -p pii-evolve` — compiles (R1-R6)
- [ ] `cargo test -p pii-evolve` — all 10 tests above pass (R2-R5)
- [ ] `cargo build --workspace` — the default build still links none of pii-evolve into `pii` (R6)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N1)
- [ ] `cargo fmt --all --check` — clean (N1)
- [ ] `cargo xtask deps-check` — accepts pii-evolve as isolated (R6/N2)
- [ ] `cargo xtask net-check` — pass (N2)
- [ ] `cargo test --workspace` — 0 failures (N1)

---

## Guardrails

- **G1 — do NOT edit this spec.** If it is wrong, STOP and report it.
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip, or delete an existing test.**
- **G4 — do NOT touch `specs/`, `xtask/`, `quality-gate.sh`, `graphs/`, or a
  `#[test]` assertion** — anti-reward-hacking (opus item 5). The boss suspends
  for human; it does NOT self-approve or widen its own gates.
- **G5 — do NOT add third-party dependencies.** Reuse workspace deps only.
- **G6 — no hardcoded absolute paths.** Test artifacts under `_tmp/` or
  `tempfile::tempdir`.
- **G7 — `pii-evolve` must NOT be a dependency of `pii-cli` / `pii-server`** —
  it stays an isolated, opt-in standalone crate (R6).

---

### One-paragraph honest summary
`pii-evolve` is the self-improving boss as a DEDICATED, feature-gated (default
off) crate. It owns state.md management (the boss's "where am I" source of
truth), context compaction (at ~70-75%, decisions+outcomes not dialogue, with
pii-memory as the durable store), and auditable per-step model routing (model
diversity: review != patch). The outer `run_cycle` skeleton runs the
deterministic self-improvement gates (reachability + doc-drift) and SUSPENDS
for human acknowledgement when one is red — never self-approves. The scheduled
boss LOOP is a pii graph wired to this crate in a later slice; this spec
delivers the crate + the three novel mechanisms + the dogfooding skeleton,
opt-in and byte-identical to today's default build.
# Pii — Session Handoff

**Status: GREEN — PHASE 1 IMPLEMENTATION IN PROGRESS (autonomous run)**

## What exists
- 3-pass architecture research in `_docs/06-research/` (passes 1-3, ~48k words total).
- 5 reference repos cloned in `_repos/` (read-only).
- pidag initialized with `ollama-cloud/glm-5.3-flash:cloud` workhorse.
- **Phase 0 COMPLETE + committed** (`1ae1b32`): workspace skeleton + CI gates, 20/20 exit criteria pass.
  - `pii-abi` kernel ABI (traits only, no transport deps), `pii-cli --version`, `xtask deps-check/lint-files/ci`, workspace lints, `quality-gate.sh`.
- **2-parallel-workers VALIDATED**: pidag `--run <dag> --concurrency 2`, two glm-5.3-flash:cloud pi processes ran concurrently, both files written, validator passed.

## pidag bugs FIXED (2026-09-02, commit `6d69f4c`)
1. **Session:** `pi --session <file>` needs pre-existing file. Fixed `src/worker/pi_print.rs`
   → `--session-dir <per-node-dir>` (auto-creates + `-c` continues, parallel-workers isolated).
2. **Model split:** `split_provider_model` (TWO copies: `src/lib.rs` + `src/backend/pi.rs`)
   mis-split `provider/model:cloud`. Both fixed to split on earliest `:`/`/`.
Build + installed to both `/root/.local/bin/pidag` and `/projects/.local/bin/pidag`.

## CURRENT TASK — Phase 1: core engine
- **In-flight:** opus writing `specs/03-phase1-core-engine.md` (spec prompt at `_tmp/phase1_spec_prompt.md`).
- Deliverables: `pii-core` (Ctx, CapSet, CancelToken, Budget, Principal, TraceCtx, Observer),
  `pii-graph` (GraphSpec + validate, scheduler = EdgeState/JoinPolicy/readiness/dead-propagation/drain-all,
  event-sourced journal), `pii-store` (redb run_journal), resume (`kill -9` then resume without
  re-invoking done nodes), cold-start `--version` p95 < 20 ms gate.
- Then: `pidag sdd specs/03-phase1-core-engine.md --run --concurrency 2`, verify exit criteria, commit, HANDOFF, agent-memory.
- Design source of truth: pass-3 §4.2–4.7 (scheduler contract), pass-1 §2.4/§3.1/§3.2/§3.4 (Ctx/graph/validate/journal).

## NEXT after Phase 1 — Phase 2: client/server split + coding harness
- Prompt ready at `_tmp/phase2_spec_prompt.md`. Load-bearing gate: `cargo tree -p pii-tui` does NOT contain `pii-graph`.
- `pii-proto` (wire types), `pii-server` (UDS + loopback), `pii-cli -p`, `pii-tui` thin client,
  `starter/react` + `starter/verify-fix`, bash mediation + nono Landlock sandbox.

## Key files
- `.pidag/config.toml` — model chain (slash-first form required).
- `AGENTS.md` — SDD/TDD workflow + hard rules.
- `specs/` — numbered specs (01 smoke, 02 phase0, 03-parallel, 03-phase1-*).
- `crates/` — pii-abi (done), + pii-core/pii-graph/pii-store (Phase 1).

## Memory keys
- `pii/architecture/plan`, `pii/architecture/plan-pass2`, `pii/architecture/plan-pass3`
- `pii/research/repo-findings`, `pii/research/pass2-findings`
- `pii/implementation/pidag-fixes`, `pii/implementation/phase0-complete`, `pii/implementation/parallel-workers-validated`

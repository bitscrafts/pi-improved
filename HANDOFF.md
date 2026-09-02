# Pii — Session Handoff

**Status: GREEN — PHASE 1 CORE ENGINE + LINT SURFACE COMPLETE (autonomous run, 2026-09-02)**

## What exists
- 3-pass architecture research in `_docs/06-research/` (passes 1-3, ~48k words total).
- 5 reference repos cloned in `_repos/` (read-only).
- pidag initialized + BUG-FIXED (session-dir isolation, model split — commit `6d69f4c`).
- **Phase 0 COMPLETE**: workspace skeleton + CI gates, 20/20 exit criteria (commit `1ae1b32`).
- **2-parallel-workers VALIDATED**: pidag `--concurrency 2`.

## PHASE 1 — done so far (all authored directly; flaky ollama-cloud worker)
Spec `specs/03-phase1-core-engine.md` written by opus, split into 4 parts (24 exit criteria).
Committed milestones:
- **pii-core** (`7ce9402`...): Ctx, CapSet, Budget, CancelToken, Principal, TraceCtx, Observer, FsCap. 8 compile errors + 8 clippy fixed. 18 tests.
- **pii-store** (`*`): redb persistence, transactional append_event (one write txn), snapshots, resume index. 5 tests. redb-4.2 trait imports; can't impl external RunStatus.
- **pii-graph model+validator** (`*`): GraphSpec/JoinPolicy/Edge, load_toml, tiny `$.` expr, NodeFactory/NodeRegistry, validate() V0-V3/V5/V9.
- **pii-graph sched state** (`*`): EdgeState/Readiness/RunState, readiness (All/Any/Quorum/AllSettled, entry-node-zero-inbound explicit), dead propagation fixpoint.
- **pii-graph blackboard + built-in nodes** (`*`): transform/route/emit/wait runtimes.
- **pii-graph run engine + CLI** (`21c3f6a`): tick loop draining all ready, UnsatisfiableJoin, limits; `pii --version` + `pii graph lint` (exit 3 on V-code).
- `pii-abi` ids now derive Ord/PartialOrd.

**Verified:** 46 tests, `quality-gate ALL PASS`; all 9 example TOMLs lint correctly
(5 functional OK, 4 negatives V1/V2/V3/V9).

## KEY LEARNINGS (see agent-memory `implementation/*`)
- **ollama-cloud large-prompt instability**: intermittent empty 0-token responses on >15KB implement prompts (both glm-5.3-flash:cloud AND deepseek-v4-flash:cloud). Small probes always work. This made pidag worker runs fail fast regardless of timeout — the model, not pidag. Worked around by authoring deterministic code directly.
- **Background pi loses OLLAMA_API_KEY**: foreground works (key in shell); spawned subshells report "No API key found".
- petgraph tarjan_scc single-node cycle = `find_edge(n,n)` self-loop, NOT `edges_directed().count()>0`.
- GraphSpec.caps / NodeSpec.caps are `u32` bitflags (CapSet scopes empty at spec; CLI applies allow-scopes at runtime).

## REMAINING in Phase 1
- **redb-backed `pii graph run/resume/trace`** wiring (F5.3/5.4/5.5, F4.11 resume journal replay). CLI `graph run/resume/trace` currently reports "not yet wired".
- **xtask bench-gate** (F6.2) + `cargo xtask ci` (F6.3).
- Then verify against part2-4 exit criteria via `pidag validate` / run.

## NEXT after Phase 1 — Phase 2: client/server split + coding harness
- Prompt ready at `_tmp/phase2_spec_prompt.md`. **Cron `pii-phase2-opus-spec` fires 2026-09-02 14:55 UTC** (opus quota reset 14:50 UTC).
- Load-bearing gate: `cargo tree -p pii-tui` does NOT contain `pii-graph`.
- `pii-proto` (wire types), `pii-server` (UDS + loopback), `pii-cli -p`, `pii-tui` thin client, `starter/react` + `starter/verify-fix`, bash mediation + nono Landlock sandbox.

## Key files
- `specs/03-phase1-core-engine-part{1,2,3,4}.md` — split Phase 1 spec.
- `examples/*.toml` — 9 functional + lint-negative example graphs.
- `crates/pii-graph/src/{spec,toml_load,expr,node,nodes,validate,sched/state,blackboard,run}.rs`
- `crates/pii-store/src/{lib,tables,codec}.rs`, `crates/pii-cli/src/main.rs`

## Memory keys
`pii/implementation/phase1-pii-core-complete`, `.../phase1-pii-store-complete`,
`.../phase1-pii-graph-lint-complete`, `.../phase1-scheduler-state-complete`,
`.../phase1-blackboard-nodes-complete`, `.../phase1-run-cli-lint-complete`,
`.../ollama-cloud-large-prompt-instability`

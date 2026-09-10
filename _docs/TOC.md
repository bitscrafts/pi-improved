# Pii Docs — Table of Contents

## 01-overview
- `001-pii-vision.md` — the unified vision and architecture overview

## 02-guides
- `pidag-way-of-work.md` — the SDD pipeline: static-graph orchestration, how pidag drives pi, the free→account→topup model ladder, the implement→quality-gate→validate loop, hard rules, and operational traps
- `feature-gating.md` — how to make the monolith compilable-down: Cargo features gate surfaces (tui/web/evolve/channels/tls/memory/sandbox/github), never the engine substrate; trims vs service-izes
- `pii-mcp-bridge.md` — the Hermes↔pii integration: pii-mcp crate (MCP server + client, feature-gated, rmcp quarantine); MCP = integration surface, UDS JSON-RPC = control plane

## 03-reference
- (empty)

## 04-development
- (empty)

## 05-operations
- (empty)

## 06-research
- `001-pii-architecture-plan.md` — Claude Code (opus) analysis of the 5 reference repos + full Pii architecture plan (pass 1)
- `002-pii-architecture-plan-pass2.md` — pass 2: 12 new directions (observability, client-server, networking, layered memory, plugins, sync, callbacks, MCU, WASM first-class, critical review)
- `003-pii-architecture-plan-pass3.md` — pass 3: nano-model Tier-0 as CPU assistant, harness/DAG/pipelines (orki diamond fix, fanout/convergence), reframes pass 2 §9
- `004-granite-nano-tier-probe.md` — granite-4.2 tier-1 probe (llama.cpp `--reasoning off` required) + opus nano-tier recommendation + flexible model-coupling directive
- `005-dsh-vs-pii-harness-analysis.md` — DeepSeek Harness (dsh) comparative analysis: convergent architecture validation, 5 upgrade paths, journal-hardening spec 11 extracted
- `006-pii-graph-engine.md` — the graph-based automation engine: architecture, node kinds, route/if-then-else, loop_budget, capabilities/effects, event-sourced journal + resume, git-log-graph research & `pii graph show` implementation
- `007-pii-phase4-plan.md` — opus MVP-complete review + Phase 4 "daily driver" plan: turn-loop hardening, session durability, memory v2, TLS, service packaging, web viewer, self-improvement
- `008-pii-evolve.md` — P4.7 deep design: a self-aware "evolve" loop (new `pii-evolve` crate, per user override) that manages its own context (state.md + external memory + compaction), routes each step to a specific model tier, checks logs, patches in disposable worktrees, maintains a parallel dev/prod split with gated promotion, and keeps improving via web research
- `009-pii-evolve-opus-review-pass1.md` — opus adversarial review (APPROVE WITH CHANGES): blocking resume+approval-gate bug, heal-watch phantom failure, anti-reward-hacking refusal; user override = dedicated `pii-evolve` crate
- `010-pii-full-review-pass1.md` — FULL-codebase opus review (VERDICT: NEEDS WORK): P4.1c/4.1b broke the agent tool loop (chat_streaming drops tool_calls; call_tool_bounded substitutes empty CapSet; request_streaming doesn't dechunk) — every autonomous run reports success while doing nothing. Plus TLS lease leak, compaction-never-fires, O(n²) session writes, budget dimensions never charged.
- `011-pii-architecture-conformance-review-pass1.md` — opus architecture-conformance review (VERDICT: NEEDS WORK): A14-A17 findings, ordered fix list specs 24-29, cut list
- `012-p47-remaining-opus-plan.md` — opus P4.7-remaining plan: timer-resume blocker, heal-fix is 70% built, specs 30-36
- `013-dsh-context-slice-c-reference.md` — dsh-context plugin as the reference design for Slice C (context management): context insight = projection over the existing journal, read the harness's own token accounting, delta-mode diagnostic for the compaction no-op

## 99-archive
- (empty)

# Pii — Pi Improved

**Pii** ("Pi Improved") is a single static Rust binary that unifies a terminal-native
AI coding agent, a personal assistant daemon, a reactive IoT/automation engine, and a
self-improving cognitive layer. It aims to replace Claude Code, Home Assistant, and a
messaging gateway with one performant, local-first, modular system.

## Project Layout

```
/projects/pi-improved/
├── _repos/     # read-only clones of the 5 reference repos (analysis only)
├── _tmp/       # scratch / temporary files (research prompts, logs)
├── _docs/      # SSOT documentation (this tree)
└── (future)    # src/ workspace when implementation begins
```

## Reference Repos (read-only, in `_repos/`)

| Repo | Role in Pii |
|------|-------------|
| `pi_agent_rust/` | Coding-agent core: sub-100ms cold start, structured concurrency, capability-gated Cx, JSONL sessions |
| `zeroclaw/` | Assistant/gateway core: 30+ channels, ~20 LLM providers, tools, SOP engine, WASM plugin host, hardware HAL |
| `yantrikclaw/` | Cognitive memory (YantrikDB) + tier-aware tool selection for model sizes |
| `grok-build/` | TUI + Agent Client Protocol (ACP) + headless/CI modes |
| `hermes-agent/` | Cognitive self-improvement loop (skill extraction, procedural templates) |

## Docs Index

- [01-overview/](01-overview/) — high-level vision and architecture
  - `001-pii-vision.md` — the unified vision and architecture overview
- [02-guides/](02-guides/) — how-to guides
  - `pidag-way-of-work.md` — the SDD pipeline: static-graph orchestration, how pidag drives pi, the free→account→topup model ladder, the implement→quality-gate→validate loop, hard rules, and operational traps
  - `feature-gating.md` — how to make the monolith compilable-down: Cargo features gate surfaces (tui/web/evolve/channels/tls/memory/sandbox/github), never the engine substrate; trims vs service-izes
  - `pii-mcp-bridge.md` — the Hermes↔pii integration: pii-mcp crate (MCP server + client, feature-gated, rmcp quarantine); MCP = integration surface, UDS JSON-RPC = control plane
  - `pi-local-llamacpp.md` — how to call pi with local llama.cpp models (granite/laguna): the built-in `llamacpp` provider, router port 8082, and the critical `--provider`/`--model`-before-`-p` flag order
- `03-reference/` — technical reference  *(section not created in this project)*
- `04-development/` — developer docs / analysis  *(section not created in this project)*
- [05-operations/](05-operations/) — operational docs, including incident records
- [06-research/](06-research/) — research write-ups (numbered)
- `000-CANDIDATES-INDEX.md` — **the ten "researched, not scheduled" candidates**, one entry point (index, deliberately not merged)
- `000-TOPIC-MAP.md` — **start here for the research tree**: the SSOT topic map for all 42 research docs (canonical / evidence / superseded + the condensation actions).
  - `030-OPUS-START-HERE.md` — compact entry point for an opus review
    pass: the 4 open architecture decisions, linked to their full
    reasoning, nothing else required reading first.
  - `002-pii-architecture-plan-pass2.md` — pass 2: 12 new directions (observability, client-server, networking, layered memory, plugins, sync, callbacks, MCU, WASM first-class, critical review)
  - `005-dsh-vs-pii-harness-analysis.md` — DeepSeek Harness (dsh) comparative analysis: convergent architecture validation, 5 upgrade paths, journal-hardening spec 11 extracted
  - `006-pii-graph-engine.md` — the graph-based automation engine: architecture, node kinds, route/if-then-else, loop_budget, capabilities/effects, event-sourced journal + resume, git-log-graph research & `pii graph show` implementation
  - `007-pii-phase4-plan.md` — opus MVP-complete review + Phase 4 "daily driver" plan: turn-loop hardening, session durability, memory v2, TLS, service packaging, web viewer, self-improvement
  - `012-p47-remaining-opus-plan.md` — opus P4.7-remaining plan: timer-resume blocker, heal-fix is 70% built, specs 30-36
  - `013-dsh-context-slice-c-reference.md` — dsh-context plugin as the reference design for Slice C (context management): context insight = projection over the existing journal, read the harness's own token accounting, delta-mode diagnostic for the compaction no-op
  - `014-local-model-spec-implementation.md` — granite/laguna as the pidag SDD workhorse: feasibility from real spec-36 attempts (full one-shot fails, micro-split succeeds), adoption gate, recommendation (granite-4.2-3b, spec sized to ~32K context)
  - `015-llm-in-process-llama-cpp.md` — in-process LLM via llama-cpp-2: embed Granite 4.2 inside Pii (feasibility, sync-trait bridge, tradeoffs)
  - `016-ml-applications.md` — classical ML on Pii (regression/classification/clustering): top-3 apps (run-cost prediction, failure-mode classification, run clustering) with data sources + crates
  - `018-manuals-self-contained.md` — plan to make the two PDF manuals self-contained (no _docs refs), SSOT between the 2 files, document design patterns
  - `019-generic-provider-endpoints.md` — generic OpenAI + Anthropic endpoint support for pii-model (base_url override, route through pii-net, steal from pi_agent_rust)
  - `020-git-graph-web-ui.md` — port vscode-git-graph to Pii's web viewer: journal-as-git-graph, configurable columns + detail level, WASM vs server-render
  - `021-mqtt-broker.md` — MQTT for Pii: broker vs bridge, topic mapping, security (pii-net chokepoint), Android coordination
  - `022-android-events-app.md` — Android background app receiving/emitting Pii events: transport (MQTT/REST), background-service pattern, notification mapping, auth
  - `02-guides/023-llamacpp-granite-on-pi-process.md` — the full process of making llama.cpp + granite work on pi: router mode, the built-in `llamacpp` provider, flag order, ds4-bench results, and the in-process provider (spec 35)
  - `024-pii-memory-mcp-status-and-next.md` — verified status of Pii's memory + MCP: both work end-to-end (3 real memory RPCs, 15 tests; Hermes↔pii-mcp connected + tools responding), 8 honest capability caveats (memory is not a first-class MCP tool, not in the agent loop, vector lane is trigram-lexical, no e2e coverage), and the ordered next steps N1-N6
  - `025-model-routing-and-failover-plan.md` — PLAN: port Hermes's typed-failure failover (FailoverReason taxonomy, per-reason recovery policy, ordered provider chain) into Pii, plus cost routing (cascading + safety rules + peak-aware provider preference). Gap analysis vs `PiiError`'s 2-way split, 7-spec breakdown for the planner, risks
  - `026-codebase-analysis-and-phase5-plan.md` — codebase/package map, debt + risk findings (S1-S3), and the Phase 5 plan (5.0 re-baseline, 5.1 Track A failover, 5.2 structural ratchet)
  - `027-orchestration-resumption-spec38-verification.md` — verification of spec-38 + the gate state at that checkpoint
  - `028-sdd-orchestration-review-checkpoint.md` — consolidated checkpoint: map, debt, next-phase
  - `029-coding-harness-design-questions-for-opus.md` — coding-harness graph: the design questions put to opus
  - `030-OPUS-START-HERE.md` — compact entry point for an opus review pass: the open architecture decisions, linked to their reasoning
  - `031-fable-review-ground-truth-2026-09-18.md` — ground truth supplied to the fable review
  - `032-fable-pii-review-2026-09-18.md` — review findings and candidate improvements
  - `05-operations/033-spec65-guardrail-violation-doc-format.md` — spec-65 guardrail violation: the worker changed the doc storage format
  - `034-cactus-needle3-automation-research.md` — Cactus Needle 3 research and relevance to pii (grammar-constrained decode, confidence-band routing, tool retrieval; measured caveats incl. classification 1/5 and confidence routing failing on vague input)
  - `037-pii-native-graph-service-design.md` — a native "pii graph" service: the design answer
  - `038-pii-chromecast-device-manager-feasibility.md` — pii as a Chromecast/Google Home device manager: feasibility (control nearly free; discovery hits the SSRF boundary)
  - `039-jev-system-one-decision-layer-analysis.md` — ANALYSIS: System-1 decision layer for pii — Jev (TypeSafe System One): what it is + the internet's common usages; **Laya (Apache-2.0, local, $0, zero egress) as the open-source Jev counterpart**; where it fits (a decision feeding the existing `route` node; the graph stays the sole edge authority); Jev vs Needle 3 (judge vs dispatcher, not substitutes); decision forks with discarded options recorded. No implementation
  - `040-laya-on-llamacpp-and-graph-driving.md` — RESEARCH+MEASURED: Laya runs from llama.cpp as a **hybrid** (ModernBERT backbone in llama.cpp `modern-bert` Q8_0 + decision head in PyTorch; **parity Δp=0.0006**, and on CPU the hybrid is *slower*, so the win is format/memory/packaging not speed). Host llama.cpp setup **renamed generic** (image `localhost/llamacpp:local`, container/unit `llamacpp`; port 8082 + default model unchanged so pi is unaffected) with a new `LLAMA_EXTRA_ARGS` hook, and Laya added as a **second instance on 8083 in embeddings mode** sharing the read-only model mount. Plus the internet research on how a System-1 model improves our projects: deterministic-shell/probabilistic-core, rules-before-model, state-guardrails as a separate layer — **the graph drives and Laya decides** (answer lands in `$.payload`, Laya may only select among existing ports), **Laya is a transition guard and never owns the transition table**, ranked applications L1-L5, and 5 caveats
- [99-archive/](99-archive/) — deprecated / historical

## Status

- [x] Repos cloned (5/5)
- [x] Claude Code (opus) architecture research complete
- [x] SSOT docs written
- [x] Implementation started (`crates/` — 18 crates, 35k+ src lines, 40+ specs landed as of 2026-09-16)
- [`06-research/042-PII-REVIEW-STAGED-STATE-2026-09-28.md`](06-research/042-PII-REVIEW-STAGED-STATE-2026-09-28.md) — the full review's staged state: never dispatched, why, and the exact resume command (read before the pii intervention).
- `041-BRIEF-full-review.md` — the brief for the full opus review of this repo (derives from `031`/`032`)
- **Consolidated (2026-09-28):** `002` now carries passes 1+3 · `012` carries `008`+`009` · `026` carries `010`+`011` · `037` carries `035`+`036` · `014` carries `004`+`017` (absorbed verbatim as `## [merged]` sections; originals in `99-archive/06-research-merged-2026-09-28/`).

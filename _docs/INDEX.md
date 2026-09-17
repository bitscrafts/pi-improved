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
- [03-reference/](03-reference/) — technical reference
- [04-development/](04-development/) — developer docs / analysis
- [05-operations/](05-operations/) — operational / environment docs
- [06-research/](06-research/) — research write-ups (numbered)
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
  - `014-local-model-spec-implementation.md` — granite/laguna as the pidag SDD workhorse: feasibility from real spec-36 attempts (full one-shot fails, micro-split succeeds), adoption gate, recommendation (granite-4.2-3b, spec sized to ~32K context)
  - `015-llm-in-process-llama-cpp.md` — in-process LLM via llama-cpp-2: embed Granite 4.2 inside Pii (feasibility, sync-trait bridge, tradeoffs)
  - `016-ml-applications.md` — classical ML on Pii (regression/classification/clustering): top-3 apps (run-cost prediction, failure-mode classification, run clustering) with data sources + crates
  - `017-tiny-models-real-usage.md` — tiny chat models (Needle/SmolLM2/Qwen-0.5-1.5B/Granite-3b): where they beat the B1 embedder (routing, tool-arg repair, summarization, extraction)
  - `018-manuals-self-contained.md` — plan to make the two PDF manuals self-contained (no _docs refs), SSOT between the 2 files, document design patterns
  - `019-generic-provider-endpoints.md` — generic OpenAI + Anthropic endpoint support for pii-model (base_url override, route through pii-net, steal from pi_agent_rust)
  - `020-git-graph-web-ui.md` — port vscode-git-graph to Pii's web viewer: journal-as-git-graph, configurable columns + detail level, WASM vs server-render
  - `021-mqtt-broker.md` — MQTT for Pii: broker vs bridge, topic mapping, security (pii-net chokepoint), Android coordination
  - `022-android-events-app.md` — Android background app receiving/emitting Pii events: transport (MQTT/REST), background-service pattern, notification mapping, auth
  - `023-llamacpp-granite-on-pi-process.md` — the full process of making llama.cpp + granite work on pi: router mode, the built-in `llamacpp` provider, flag order, ds4-bench results, and the in-process provider (spec 35)
  - `024-pii-memory-mcp-status-and-next.md` — verified status of Pii's memory + MCP: both work end-to-end (3 real memory RPCs, 15 tests; Hermes↔pii-mcp connected + tools responding), 8 honest capability caveats (memory is not a first-class MCP tool, not in the agent loop, vector lane is trigram-lexical, no e2e coverage), and the ordered next steps N1-N6
  - `025-model-routing-and-failover-plan.md` — PLAN: port Hermes's typed-failure failover (FailoverReason taxonomy, per-reason recovery policy, ordered provider chain) into Pii, plus cost routing (cascading + safety rules + peak-aware provider preference). Gap analysis vs `PiiError`'s 2-way split, 7-spec breakdown for the planner, risks
- [99-archive/](99-archive/) — deprecated / historical

## Status

- [x] Repos cloned (5/5)
- [x] Claude Code (opus) architecture research complete
- [x] SSOT docs written
- [x] Implementation started (`crates/` — 18 crates, 35k+ src lines, 40+ specs landed as of 2026-09-16)

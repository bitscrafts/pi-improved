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
├── crates/     # the Pii Rust workspace (19 crates; see Cargo.toml)
└── xtask/      # workspace tooling member (gate lints, promote-build)
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
  - `pii-mcp-bridge.md` — the Hermes↔pii integration: pii-mcp crate (MCP server + client, separate binary, rmcp quarantine); MCP = integration surface, UDS JSON-RPC = control plane
  - [`023-llamacpp-granite-on-pi-process.md`](02-guides/023-llamacpp-granite-on-pi-process.md) — the full process of making llama.cpp + granite work on pi: router mode, the built-in `llamacpp` provider, flag order, ds4-bench results, and the in-process provider (spec 35)
  - `pi-local-llamacpp.md` — how to call pi with local llama.cpp models (granite/laguna): the built-in `llamacpp` provider, router port 8082, and the critical `--provider`/`--model`-before-`-p` flag order
- [`observability-and-deploy-conventions.md`](02-guides/observability-and-deploy-conventions.md) — **the observability + deploy conventions**: what the gate enforces (contract crate, bounded exemption registry, per-file print budget, name-registry completeness), health identity, the operator-surface rule, trajectories, and the deploy/runbook commands
- [`writing-a-channel.md`](02-guides/writing-a-channel.md) — **writing a channel**: the `Channel` trait contract (cursor, `Unavailable`, `health_issue`), the `[[channel]]` keys, scope rule, the events a channel emits, and the telegram adapter as a copyable skeleton
- [`writing-an-sop.md`](02-guides/writing-an-sop.md) — **writing an SOP**: the `[[step]]` format (`act`/`check`/`approve`/`notify`), how it compiles into a graph, `requires`/`check` failure semantics, the approval flow, and the cut-a-release example
- [`learning-loop.md`](02-guides/learning-loop.md) — **Learning loop, stage 1 (extraction)**: `pii learn scan|list|show`, thresholds, refusals, the injection stance, bounds; proposes only, never acts
- [`wasm-plugins.md`](02-guides/wasm-plugins.md) — **WASM plugin host**: the ABI, epoch/fuel/memory limits, no ambient authority, `[wasm]` keys, the `wasm` node, building a plugin, and what the sandbox is not
- [`hardware-hal.md`](02-guides/hardware-hal.md) — **Hardware HAL**: the claim + scope contract, `[hal]` keys, the simulated backend, the `hal` node, and the real `hal-gpiod` backend (compiled, never run on hardware) with how to bring up a board
- [`mqtt.md`](02-guides/mqtt.md) — **MQTT**: the `[[mqtt]]` keys, JSON-else-text payload mapping, the `mqtt` publish node (QoS/retain explicit), bounded reconnect backoff, and the plaintext-credentials refusal (no TLS in this build)
- [`model-tiers.md`](02-guides/model-tiers.md) — **model tiers**: the `[[model_tier]]` keys, the node-model precedence, the capability-layer tool gate, and the no-silent-downgrade rule
- [`quality-gate-and-fresh-tree.md`](02-guides/quality-gate-and-fresh-tree.md) — **the gate manual**: all eight steps and why each exists, the two-tree model (`quality-gate.sh` vs `fresh-gate`, and why to believe the export), the `tmp_scratch` rule every test must follow, and the acknowledgement limits
- `03-reference/` — technical reference  *(section not created in this project)*
- `04-development/` — developer docs / analysis  *(section not created in this project)*
- [05-operations/](05-operations/) — operational docs, including incident records
  - [`033-spec65-guardrail-violation-doc-format.md`](05-operations/033-spec65-guardrail-violation-doc-format.md) — spec-65 guardrail violation: the worker changed the doc storage format (resolved)
  - [`105-runbook.md`](05-operations/105-runbook.md)
  - [`dormant-capabilities.md`](05-operations/dormant-capabilities.md) — capabilities that exist but are deliberately not wired, each with its reason and the CHECK that keeps it dormant (`pii-http-fetch` first, per the owner decision of 2026-10-10) — operator runbook: start/stop, health, metrics, logs, backup/restore, upgrade, wedged daemon
- [06-research/](06-research/) — research write-ups (numbered)
  - **Start with [`06-research/000-TOPIC-MAP.md`](06-research/000-TOPIC-MAP.md)** — the SSOT topic map for the research tree (canonical / evidence / superseded, per subject). It replaces the per-file list formerly kept here.
  - [`06-research/000-CANDIDATES-INDEX.md`](06-research/000-CANDIDATES-INDEX.md) — the ten "researched, not scheduled" candidates
  - [`06-research/030-OPUS-START-HERE.md`](06-research/030-OPUS-START-HERE.md) — compact entry point for an opus review pass
  - [`06-research/043-opus-full-review-2026-10-06.md`](06-research/043-opus-full-review-2026-10-06.md) — the full claude+opus review of this codebase (2026-10-06): gate results run for real, the eight decisions, the defects
  - [`06-research/045-opus-rereview-2026-10-09.md`](06-research/045-opus-rereview-2026-10-09.md) — the RE-REVIEW (2026-10-09): batches A-D judged by behaviour, its ranked findings, and the two the orchestrator reproduced independently
  - **The 045 work-through (specs 83-96), evidence in `06-research/`:**
    [`046`](06-research/046-mutation-verification-batch-cd-2026-10-09.md) mutation-verified the batch C/D guards and filed five green mutants ·
    [`047`](06-research/047-close-green-mutants-log-2026-10-09.md) closed four of them ·
    [`048`](06-research/048-replay-err-divergence-measured-2026-10-09.md) measured a REAL defect (replay vs live divergence) ·
    [`049`](06-research/049-replay-err-fix-remeasure-2026-10-09.md) fixed it and re-measured ·
    [`050`](06-research/050-inline-methods-measured-2026-10-09.md) measured all 11 inline RPC handlers ·
    [`051`](06-research/051-scratch-helper-and-sweep-2026-10-09.md) killed the non-hermetic-test class at the root
  - [`06-research/053-observability-contract-and-enforcement-2026-10-09.md`](06-research/053-observability-contract-and-enforcement-2026-10-09.md) — **observability: the contract, the enforcement, and what landed** (2026-10-09): the measured adoption picture (13 crates with zero, 139 raw prints, health with no identity), the `pii-observe` decision and its rationale, the rules the gate now enforces, and the outcome table for specs 97-106 with its honest limits
  - [`06-research/057-vision-implementation-plan-2026-10-10.md`](06-research/057-vision-implementation-plan-2026-10-10.md) — **the direction is the vision**: the measured state of every pillar, and the ORDER that implements all of them as specs 111-122 (gateway completion, then IoT/reactive, then self-improvement, then pillar 3), with the rules each stage inherits
  - [`06-research/055-exemption-audit-2026-10-10.md`](06-research/055-exemption-audit-2026-10-10.md) — **the exemption audit** (2026-10-10): measured crate by crate; of the four sanctioned exemptions, one is permanent (`pii-observe`) and one honest (`pii-abi`, no I/O), while `pii-proto` (a real async transport, and the module that can hang) and `pii-uds` (the one `UnixStream::connect` site outside the daemon) were exempted at the wrong granularity — with the mechanical rule that makes "nothing to observe" decidable
  - [`06-research/052-production-readiness-2026-10-09.md`](06-research/052-production-readiness-2026-10-09.md) — **honest production-readiness assessment + the road to production** (2026-10-09): what works (proven end-to-end), quality measured, the specification-vs-reality table per pillar and phase, every stale doc claim, the readiness gaps by axis, and a phased P0-P5 plan with measurable exit criteria
- [99-archive/](99-archive/) — deprecated / historical

## Status

- [x] Repos cloned (5/5)
- [x] Claude Code (opus) architecture research complete
- [x] SSOT docs written
- [x] Implementation started (`crates/` — 19 crates plus the xtask member; **96 specs authored, 83-96 landed 2026-10-09** — the 045 re-review work-through)
- [`06-research/042-PII-REVIEW-STAGED-STATE-2026-09-28.md`](06-research/042-PII-REVIEW-STAGED-STATE-2026-09-28.md) — the full review's staged state: never dispatched, why, and the exact resume command (read before the pii intervention).
- `041-BRIEF-full-review.md` — the brief for the full opus review of this repo (derives from `031`/`032`)
- **Consolidated (2026-09-28):** `002` now carries passes 1+3 · `012` carries `008`+`009` · `026` carries `010`+`011` · `037` carries `035`+`036` · `014` carries `004`+`017` (absorbed verbatim as `## [merged]` sections; originals in `99-archive/06-research-merged-2026-09-28/`).
- [`_docs/06-research/045-opus-rereview-2026-10-09.md`](06-research/045-opus-rereview-2026-10-09.md) — the claude+opus RE-REVIEW (2026-10-09): batches A-D judged by behaviour,
  with the two findings the orchestrator reproduced independently.

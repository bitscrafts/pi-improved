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
- [03-reference/](03-reference/) — technical reference
- [04-development/](04-development/) — developer docs / analysis
- [05-operations/](05-operations/) — operational / environment docs
- [06-research/](06-research/) — research write-ups (numbered)
  - `001-pii-architecture-plan.md` — Claude Code (opus) analysis of the 5 reference repos + full Pii architecture plan (pass 1)
  - `002-pii-architecture-plan-pass2.md` — pass 2: 12 new directions (observability, client-server, networking, layered memory, plugins, sync, callbacks, MCU, WASM first-class, critical review)
  - `003-pii-architecture-plan-pass3.md` — pass 3: nano-model Tier-0 as CPU assistant, harness/DAG/pipelines (orki diamond fix, fanout/convergence), reframes pass 2 §9
- [99-archive/](99-archive/) — deprecated / historical

## Status

- [x] Repos cloned (5/5)
- [x] Claude Code (opus) architecture research complete
- [x] SSOT docs written
- [ ] Implementation started

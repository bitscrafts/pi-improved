# Pii — Vision & Architecture Overview

**Pii** ("Pi Improved") is a single static Rust binary that unifies a terminal-native
AI coding agent, a personal assistant daemon, a reactive IoT/automation engine, and a
self-improving cognitive layer. It aims to replace Claude Code, Home Assistant, and a
messaging gateway with one performant, local-first, modular system.

## The Four Pillars

1. **Coding agent core** (from `pi_agent_rust`) — sub-100ms cold start, structured
   concurrency, capability-gated `Ctx`, JSONL sessions, bash mediation.
2. **Assistant/gateway** (from `zeroclaw`) — 30+ channels, ~20 LLM providers, tools,
   SOP engine, WASM plugin host, hardware HAL.
3. **Cognitive memory + model tiering** (from `yantrikclaw`) — entities/relationships,
   tier-aware tool selection for model sizes 0.5B→100B+.
4. **Self-improvement loop** (from `hermes-agent`) — skill extraction, procedural
   templates, trajectory logging for future fine-tuning of a local ~9B model.

## Key Architectural Decisions (from opus research)

- **Rust workspace of ~14 crates**, with `pii-abi` as the kernel ABI (traits + types
  only, no transport deps). Enforced by `xtask deps-check` + a hard 2,000-line
  file-size gate.
- **Graph-based orchestration**: dynamic DAG-with-bounded-cycles, routed by data,
  checkpointed by an event-sourced journal. `Suspend` on the node return type makes
  suspension compositional. Load-time validation rejects unbounded cycles.
- **`Ctx` capability context that actually enforces** (pi documented it but didn't
  build it): accessors return `Result`, `child()` is monotonically attenuating,
  scope re-checked after canonicalisation, three layers of defence (Ctx / nono
  Landlock / WASI).
- **redb persistence** (single-writer MVCC ACID, pure Rust, no C dep for static
  musl). Consequence: the daemon owns the DB; UDS control channel needed in Phase 2.
- **WASM plugins** via `wasmtime 47.x` + WASI 0.2 component model. Epoch interruption
  primary, fuel opt-in, pooling allocation. Core daemons (MQTT, MCP) stay native.
- **Worker IPC designed from scratch** (no reference does this): SOCK_STREAM UDS +
  newline-delimited JSON-RPC 2.0, optional SCM_RIGHTS fd handoff, hot reload via
  atomic rename. Worker isolation is opt-in per subgraph, not the default.
- **Self-improvement**: skills keep hermes' shape (progressive disclosure,
  read-before-write as tool precondition, never-delete lifecycle) + `Skill.graph`
  for zero-token graph promotion after shadow-run agreement.
- **Trajectory logging** with `outcome.verifier` (programmatic pass/fail),
  `user_signal`, `escalation`, and mandatory `redaction` — for future 9B fine-tuning.

## The Three Open Decisions

1. **redb vs SQLite** — recommended: redb (accepting hand-written BM25 and
   daemon-owns-the-DB).
2. **First milestone** — recommended: Phases 1–2 (~7 wk) to a coding agent that
   replaces the current one. Alternative: MVP cut (~5 wk) to prove the substrate.
3. **`pii worker --graph <id>`** — recommended: drop as default execution mode, keep
   worker isolation as an opt-in `SubgraphPolicy` field.

## Implementation Phases

| Phase | Scope | Est. |
|-------|-------|------|
| 0 | Toolchain + workspace skeleton + CI gates | ½ wk |
| 1 | Core engine (Ctx, graph, journal, resume, cold start) | 3–4 wk |
| 2 | Coding harness (tools, sandbox, verify-fix) | 3–4 wk |
| 3 | Assistant/gateway (daemon, UDS, channels) | 3–4 wk |
| 4 | IoT/reactive (MQTT, hardware, WASM plugins) | 2–3 wk |
| 5 | Self-improvement (skills, distillation, trajectories) | 3–4 wk |

See [06-research/001-pii-architecture-plan.md](../06-research/001-pii-architecture-plan.md)
for the full plan.

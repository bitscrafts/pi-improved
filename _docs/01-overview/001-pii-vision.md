# Pii — Vision & Architecture Overview

**Pii** ("Pi Improved") is a single Rust binary (`pii`; statically linked when built for
`x86_64-unknown-linux-musl`, see below) that unifies a terminal-native
AI coding agent, a personal assistant daemon, a reactive IoT/automation engine, and a
self-improving cognitive layer. It aims to replace Claude Code, Home Assistant, and a
messaging gateway with one performant, local-first, modular system.

> **Static-linking status (spec 104, measured 2026-10-09).** The default
> `cargo build` produces a *dynamically* linked glibc binary (libgcc_s, libm,
> libc). The "static" claim holds for the musl release build:
> `cargo build --release --target x86_64-unknown-linux-musl -p pii-cli` gives
> `ELF 64-bit LSB pie executable, x86-64 ... static-pie linked, stripped`
> (4.4 MB) and `ldd` answers `statically linked`. Only `pii` (the `pii-cli`
> crate) was built and checked that way; `pii-web`, `pii-mcp` and `pii-evolve`
> are separate binaries that were not part of that measurement. Deploy
> artefacts: `deploy/Dockerfile`, `deploy/pii.service`.

## The Four Pillars

1. **Coding agent core** (from `pi_agent_rust`) — sub-100ms cold start, structured
   concurrency, capability-gated `Ctx`, JSONL sessions, bash mediation.
2. **Assistant/gateway** (from `zeroclaw`) — channels (measured: 5 kinds — `cli`, `discord`, `telegram`, `matrix` (unencrypted rooms), and `webhook`, the vendor-free inbound/outbound HTTP channel; "30+" was the zeroclaw figure, not ours — the rest are written against the documented trait, see `_docs/02-guides/writing-a-channel.md`), LLM providers (measured: 5 kinds — `ollama`, `openai` (OpenAI-compatible, base URL configurable, so any vendor of that shape), `anthropic`, `gemini`, and in-process `llamacpp` behind a feature; `scripted` is a test fixture, not a provider; the "~20" was the zeroclaw figure, not ours), tools,
   SOP engine (built, spec 113: a declarative `[[step]]` file that compiles into a graph and runs through the existing journal/resume/approval machinery — `pii sop {compile|lint|show|run|resume}`, see `_docs/02-guides/writing-an-sop.md`; `act`/`check`/`approve`/`notify` steps only, no branching), WASM plugin host, hardware HAL.
3. **Cognitive memory + model tiering** (from `yantrikclaw`) — entities/relationships,
   tier-aware tool selection for model sizes 0.5B→100B+ — **built (spec 114)**: `[[model_tier]]` names a
   provider + tool allowlist, enforced by the capability layer; see `02-guides/model-tiers.md`.
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
| 4 | IoT/reactive (MQTT **done, no TLS**; hardware and WASM plugins absent) | 2–3 wk |
| 5 | Self-improvement (skills, distillation, trajectories) | 3–4 wk |

**Phase 4 status (spec 117, 2026-10-10).** MQTT exists: `[[mqtt]]` subscribes a broker to a
graph and the `mqtt` node publishes ([guide](../02-guides/mqtt.md)) — plain TCP only, **no
TLS**, MQTT 3.1.1, clean sessions. **Still absent:** the hardware HAL and WASM plugins.

See [06-research/001-pii-architecture-plan.md](../06-research/002-pii-architecture-plan-pass2.md)
for the full plan.

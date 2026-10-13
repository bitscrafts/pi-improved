# Pii — Vision & Architecture Overview

**Pii** ("Pi Improved") is one main Rust binary (`pii`, plus the separate `pii-web`, `pii-mcp` and
`pii-evolve` binaries; `pii` is statically linked when built for
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
> artefacts: `deploy/pii.service` (the deployment path: **systemd on the host**) and
> `deploy/Dockerfile` (a spare container recipe, untested here; systemd is the deployment path).

> **Deployment reality (measured 2026-10-11, `_docs/06-research/059-opus-vision-judgement-2026-10-11.md`).**
> Production is **systemd on the host**, not a container. The local model runs under **llama.cpp**
> (`:8082` for the pi-local models, `:8083` for laya); **a local ollama is not part of the final version**
> (user directive 2026-10-10). **The tree is ahead of the machine:** the deployed `pii` binary predates
> this tree and no `pii.service` unit is installed on the host, so every "built" below means built and
> gated in the tree, not running in production.

## The Four Pillars

1. **Coding agent core** (from `pi_agent_rust`) — sub-100ms cold start, structured
   concurrency, capability-gated `Ctx`, JSONL sessions, bash mediation.
2. **Assistant/gateway** (from `zeroclaw`) — channels (measured: 5 kinds — `cli`, `discord`, `telegram`, `matrix` (unencrypted rooms), and `webhook`, the vendor-free inbound/outbound HTTP channel; **only `cli` has a user: `discord`, `telegram`, `matrix` and `webhook` have no deployment on any machine** (059 Q8; Discord is not configured yet); "30+" was the zeroclaw figure, not ours — the rest are written against the documented trait, see `_docs/02-guides/writing-a-channel.md`; **egress from a graph exists (spec 128):** the `post` node writes into a configured channel through `Channel::send`, capability `post`, idempotence key, bounded — see `_docs/02-guides/discord-graphs.md`), LLM providers (measured: 5 kinds, but **two wire protocols reach every vendor**: OpenAI-compatible (`openai:`, base URL configurable, and `llamacpp-server:` aimed at the local llama.cpp) and Anthropic; `gemini` is a third protocol **pending the keep-or-retire decision**, and the in-process `llamacpp:` loader is feature-gated. The kinds: `llamacpp-server` (an HTTP llama.cpp server, host from `[llama] host`, default loopback; spec 129 — the `ollama:` kind is **retired and refused**, and the host's `ollama.service` (`127.0.0.1:11434`, unreachable from the container, measured `000`) must not be depended on: disabling it is a user-side deployment step), `openai` (OpenAI-compatible, base URL configurable, so any vendor of that shape), `anthropic`, `gemini` (pending decision), and in-process `llamacpp` behind a feature; `scripted` is a test fixture, not a provider; the "~20" was the zeroclaw figure, not ours), tools,
   SOP engine (built, spec 113: a declarative `[[step]]` file that compiles into a graph and runs through the existing journal/resume/approval machinery — `pii sop {compile|lint|show|run|resume}`, see `_docs/02-guides/writing-an-sop.md`; `act`/`check`/`approve`/`notify` steps only, no branching), WASM plugin host (built, spec 122: `pii-wasm` — `wasm` graph nodes running untrusted core-wasm modules under epoch, fuel and memory limits with no WASI and no imports; **not fuzzed**; see `_docs/02-guides/wasm-plugins.md`), hardware HAL (built, spec 121: `pii-hal` — GPIO/I2C/SPI behind a claim + pin/bus capability scope; **verified only on the simulated backend**, the real `hal-gpiod` GPIO backend is compiled by the gate and never run on hardware; see `_docs/02-guides/hardware-hal.md`).
3. **Cognitive memory + model tiering** (from `yantrikclaw`) — entities/relationships,
   tier-aware tool selection for model sizes 0.5B→100B+ — **model tiering built (spec 114)**: `[[model_tier]]` names a
   provider + tool allowlist, enforced by the capability layer; see `02-guides/model-tiers.md`.
   **Memory status (spec 126):** entities and relationships are **built** — typed records with content-derived ids, mandatory provenance, idempotent upsert, a refuse-on-dangling integrity rule, capped and sorted queries, and `remember`/`recall` graph nodes gated by `memory_write`/`memory_read`; see `02-guides/memory-entities.md`. **Memory tiering is built (spec 127), which completes the pillar's memory half:** a record is `hot`, `warm` or `cold` as a pure function of when it was last read and how often (stated thresholds, configurable), queries can rank by tier, a cold record is *demoted* and never deleted, a byte budget refuses writes it cannot hold, and the only removal path is an explicit, audited `pii memory archive` that writes and verifies a JSONL dump before removing anything (`pii memory restore` reads it back); see `02-guides/memory-entities.md`. **Pillar 3 status: complete for entities/relationships + tiers. Still absent:** model-driven inference or summarisation over memory (tiering is arithmetic, no model in the loop), cross-instance sync or replication of the store, and any automatic deletion (by design).
4. **Self-improvement loop** (from `hermes-agent`) — skill extraction, procedural
   templates, trajectory logging for future fine-tuning of a local ~9B model. **Status (spec 125):** trajectories are logged (spec 103); **extraction is built** (spec 123) — `pii-learn`, `pii learn {scan|list|show}`: deterministic, offline candidate lessons, each citing its evidence, which only *propose*; **verification is built** (spec 124) — `pii learn verify`: a candidate must survive an isolated shadow run of the real engine and ends `confirmed`, `refuted` or `inconclusive` ([guide](../02-guides/learning-loop.md)). **promotion is built** (spec 125) — the loop is closed as *propose → verify → promote → measure → revert*: a `confirmed` verdict can be promoted to exactly one action, `quarantine_graph` (the engine stops starting a graph that fails every run), with a ledger record, stored rollback, a TTL, a measured outcome window and an automatic reversal when the metric did not improve. **Still absent:** any other action — the loop edits no code, config or threshold — and there is no fine-tuning.

## Key Architectural Decisions (from opus research)

- **Rust workspace of 24 crates**, with `pii-abi` as the kernel ABI (traits + types
  only, no transport deps). Enforced by `xtask deps-check` + a hard file-size gate
  (see `quality-gate.sh` for the current limit). Measured 2026-10-13 in `/projects/pi-improved`
  (re-measure with the commands, do not trust the figures): 24 workspace members
  (`cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(len(json.load(sys.stdin)["workspace_members"]))'`),
  400 `.rs` files (`git ls-files '*.rs' | wc -l`), ~90,500 lines of Rust
  (`git ls-files '*.rs' | xargs cat | wc -l`), ~1,126 test attributes
  (`git ls-files '*.rs' | xargs grep -E '#\[(tokio::)?test' | wc -l`), 158 specs
  (`ls specs/*.md | wc -l`).
- **Graph-based orchestration**: dynamic DAG-with-bounded-cycles, routed by data,
  checkpointed by an event-sourced journal. `Suspend` on the node return type makes
  suspension compositional. Load-time validation rejects unbounded cycles.
- **`Ctx` capability context that actually enforces** (pi documented it but didn't
  build it): accessors return `Result`, `child()` is monotonically attenuating,
  scope re-checked after canonicalisation, three layers of defence (Ctx / nono
  Landlock / WASI).
- **redb persistence** (single-writer MVCC ACID, pure Rust, no C dep for static
  musl). Consequence: the daemon owns the DB; UDS control channel needed in Phase 2.
- **WASM plugins** via `wasmtime 48.0.5`, **core modules with no WASI and no imports** (built, spec 122;
  the WASI 0.2 component model was the plan and is not what shipped). Epoch interruption
  primary, fuel opt-in, per-instance memory cap; the pooling allocator is not used. Core daemons (MQTT, MCP) stay native.
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
| 4 | IoT/reactive (MQTT **done, no TLS**; hardware HAL **done on the simulated backend, real GPIO backend compiled but never run on hardware**; WASM plugin host **done, offline-tested, not fuzzed**) | 2–3 wk |
| 5 | Self-improvement (skills, distillation, trajectories) | 3–4 wk |

**Phase 4 status (spec 117, 2026-10-10).** MQTT exists: `[[mqtt]]` subscribes a broker to a
graph and the `mqtt` node publishes ([guide](../02-guides/mqtt.md)) — plain TCP only, **no
TLS**, MQTT 3.1.1, clean sessions. **Hardware HAL (spec 121, 2026-10-10):** `pii-hal` gives GPIO/I2C/SPI behind a claim and a pin/bus scope ([guide](../02-guides/hardware-hal.md)); it is *exercised only on the simulated backend* — the real GPIO backend (`hal-gpiod`) compiles in the gate but has never run on hardware. **WASM plugin host (spec 122, 2026-10-10):** `pii-wasm` runs third-party `wasm` nodes without rebuilding `pii` ([guide](../02-guides/wasm-plugins.md)) — core modules only, no WASI, limits enforced by the runtime; tested offline against hand-written fixtures and **never fuzzed**. **Phase 4 has nothing left marked absent.**

**Dormant capabilities** (code present, deliberately not wired) are listed in
`_docs/05-operations/dormant-capabilities.md`; a feature above marked "built" but absent from any
deployment (SOP, WASM, HAL, MQTT, the non-`cli` channels) is exercised by tests only.

See [06-research/002-pii-architecture-plan-pass2.md](../06-research/002-pii-architecture-plan-pass2.md)
for the full plan.

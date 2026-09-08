# Pii — pii-mcp: MCP server + client (feature-gated crate)

**Date**: 2026-09-08 · **Author**: orchestrator · **Status**: DESIGN NOTE — the mechanism for Hermes↔pii integration

## Why

The user uses Hermes daily and wants to connect it to pii to **exercise pii's
engine** as an always-on test harness. The cleanest seam is MCP (Model Context
Protocol): Hermes already speaks it natively (`hermes mcp add`), and pii's
roadmap already planned MCP on both flanks.

## The two roles (both in one crate)

- **Server** (`pii mcp` subcommand): pii exposes its native tools (fs, edit,
  bash, glob, grep, http, memory) as an MCP server over stdio. Hermes registers
  it → `mcp_pii_*` tools appear → Hermes calls pii's tools, exercising the
  engine daily.
- **Client**: pii the agent reaches OUT and consumes external MCP tool servers,
  exposing their tools to pii graphs/agent.

## Why MCP, not WASM (for this seam)

- MCP is the standard inter-process RPC for tool exposure; Hermes auto-discovers
  it. WASM is for running portable/isolated guest code inside a host — it adds
  `wasmtime` + a capability story and solves a problem we don't have (single
  static binary; adding a node kind is a `reg.register()` line).
- The opus review put WASM on the do-NOT-build list for P4. WASM could later
  host evolve/user node-kinds as sandboxed plugins — a separate, deferred
  feature, not the Hermes↔pii bridge.

## The honest nuance (from roadmap pass2 §563)

**MCP is the integration surface, NOT the control plane.** MCP models
tools/resources/prompts — "an agent reaching outward." It has no first-class
session, no assistant-delta stream, no approval round-trip, no graph run, no
listener table. So:
- MCP **server** = Hermes calls pii's *tools* — great for dogfooding the engine.
- Full **control** (start a run, watch trace, resolve approval) = pii's own UDS
  JSON-RPC (`session.*`/`run.*`/`approve.*`), which MCP should NOT tunnel
  (pass2 explicitly rejects "tunnelling Pii's real API through tools/call and
  losing every type").

## Crate shape

- New `crates/pii-mcp`, **feature-gated** (pluggable, default off) — the
  convention from `_docs/02-guides/feature-gating.md`.
- Exists specifically to **quarantine `rmcp`** (its `reqwest 0.13` requirement)
  so the rest of pii doesn't drag in a conflicting HTTP stack (roadmap pass1
  §1.4, pass2 §1869/§1955).
- Deps: `rmcp`, `pii-tools` (the toolset to expose), `pii-proto`/`pii-server`
  (client RPC to the daemon).

## Wiring

```bash
# Hermes side (after the crate is built + daemon running)
printf 'Y\n' | hermes mcp add pii --command /root/.local/bin/pii-mcp
# → mcp_pii_* tools appear in new Hermes sessions
```

## Build order

1. `pii-mcp` crate scaffold (feature-gated, server first — the daily-exercise
   win), then client.
2. Register in Hermes, verify tools appear.
3. Later: `pii-evolve` crate on top (the boss loop), which can also use the MCP
   client to reach external tools.

## Guardrails

- Feature-gated, default off (pluggable).
- Server exposes pii's TOOLS, not the full control API (that stays UDS JSON-RPC).
- `rmcp` quarantined in `pii-mcp`; no new HTTP stack in core crates.
- The daemon must be running for the server to proxy to it.

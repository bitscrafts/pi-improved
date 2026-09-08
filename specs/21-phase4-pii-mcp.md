# pi-improved — spec-21: phase4-pii-mcp

- **Project**: `/projects/pi-improved`
- **Priority**: P4.7 — the Hermes↔pii integration seam. Hermes is used daily; connecting it to pii lets Hermes exercise pii's engine as an always-on test harness. MCP is the standard, Hermes-native mechanism.
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
  on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
  commits and updates HANDOFF.md)*
- **Source**: Phase 4.7 - pii-mcp crate (2026-09-08), design note `_docs/02-guides/pii-mcp-bridge.md`
- **Depends-On**: none (the daemon must be running for the server to proxy to it, but that's runtime, not a build dep).

---

## Verified Premises

- `crates/pii-tools/src/lib.rs:55-78` — `ToolSet::builtin` exposes 7 tools: `fs_read`, `fs_write`, `edit`, `multi_edit`, `grep`, `glob`, `bash`. These are the tools the MCP server exposes.
- `crates/pii-tools/src/lib.rs:23-34` — the `Tool` trait is `{ name, spec, call }`; `spec()` returns a `ToolSpec { name, description, schema }`. This maps directly onto MCP's tool schema.
- `crates/pii-proto/src/client.rs` — `Client` provides UDS JSON-RPC request/response demux (used by pii-cli/pii-tui). The MCP client's RPC to the daemon reuses this.
- `crates/pii-server/src/uds.rs` — `client_connect` + `runtime_dir` resolve the daemon socket.
- `Cargo.toml` — workspace has NO `rmcp` dep yet. The roadmap (pass1 §1.4, pass2 §1869/§1955) says `pii-mcp` exists specifically to quarantine `rmcp` (its `reqwest 0.13` requirement) so core crates don't drag in a conflicting HTTP stack.
- `_docs/02-guides/feature-gating.md` — the convention: new subsystems are feature-gated crates, default off. `pii-mcp` follows this.

---

## Overview

The user uses Hermes daily and wants to connect it to pii to **exercise pii's
engine**. The cleanest seam is MCP: Hermes already speaks it natively
(`hermes mcp add`), and pii's roadmap planned MCP on both flanks.

This spec builds the **`pii-mcp` crate** — a feature-gated crate with two roles:
1. **Server** (`pii mcp` subcommand): exposes pii's native tools (fs, edit,
   bash, glob, grep, http, memory) as an MCP server over stdio. Hermes registers
   it → `mcp_pii_*` tools appear → Hermes calls pii's tools, exercising the
   engine daily.
2. **Client**: pii the agent reaches OUT and consumes external MCP tool servers,
   exposing their tools to pii graphs/agent.

**MCP is the integration surface, NOT the control plane** (roadmap pass2 §563):
MCP models tools/resources/prompts. It has no first-class session, no
assistant-delta stream, no approval round-trip, no graph run. So the server
exposes pii's TOOLS; full control (start a run, watch trace, resolve approval)
stays on pii's own UDS JSON-RPC.

---

## Requirements

### Functional

- **R1 (pii-mcp crate)**: a new `crates/pii-mcp` crate, **feature-gated** (default off, per the pluggable convention). It quarantines `rmcp` so core crates don't drag in its `reqwest 0.13` requirement.
- **R2 (MCP server)**: `pii mcp` subcommand (wired into `pii-cli` behind the feature) starts an MCP server over stdio exposing pii's native tools (`fs_read`, `fs_write`, `edit`, `multi_edit`, `grep`, `glob`, `bash`) as MCP tools. Each MCP tool call invokes the corresponding `pii_tools::Tool::call` with the parsed args. The server proxies to the daemon (via UDS) for tools that need it, or calls the toolset directly.
- **R3 (MCP client)**: pii can consume an external MCP tool server — a graph/agent node can call tools from a remote MCP server, exposing them as pii tools. The client connects to an MCP server (stdio or HTTP) and lists/calls its tools.
- **R4 (feature gate)**: the `pii-mcp` crate is behind a `mcp` feature (default off). `cargo build --features mcp` includes it; the default build has none of it. `pii-cli`'s `pii mcp` subcommand is `#[cfg(feature = "mcp")]`-gated.

### Non-Functional

- **N1**: `rmcp` confined to `pii-mcp` (deps-check invariant, mirroring rustls→pii-net and axum→pii-web).
- **N2**: the quality gate stays green; the test count may only go up.
- **N3**: `xtask` gates stay green (deps-check must allow rmcp under pii-mcp only).
- **N4**: the server exposes pii's TOOLS, not the full control API (that stays UDS JSON-RPC).

---

## Architecture

```
Hermes (MCP client)  --stdio-->  pii-mcp server  --UDS-->  pii daemon
   |  mcp_pii_fs_read, mcp_pii_bash, ...              (session.*/run.*/approve.*)
   v
pii-mcp crate (feature-gated, quarantines rmcp)
   |  server: ToolSet::builtin -> MCP tools
   |  client: external MCP server -> pii tools
```

**Key decision — server exposes pii's TOOLS, not the control API.** MCP models
tools/resources/prompts; it has no session/delta/approval/graph-run. Tunnelling
pii's real API through `tools/call` would lose every type (roadmap pass2 §563
explicitly rejects it). So the server maps `ToolSet::builtin` → MCP tools; the
control plane stays on UDS JSON-RPC.

**Key decision — `rmcp` quarantined in `pii-mcp`.** The roadmap's reason: `rmcp`
requires `reqwest >= 0.13` while the ecosystem is on 0.12. Keeping it in its own
crate prevents a conflicting HTTP stack from leaking into core crates.

**Key decision — feature-gated, default off.** Per the pluggable convention
(`feature-gating.md`): an installer that doesn't want the Hermes integration
builds none of it.

**What this spec is not**: the `pii-evolve` boss crate (separate), the control
API over MCP (rejected), WASM plugins (deferred).

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R1 | `mcp_crate_builds_with_feature` (build check) | `cargo build -p pii-mcp --features mcp` | compiles; default build has no pii-mcp |
| R2 | `server_lists_builtin_tools` (pii-mcp unit) | start the MCP server, call `tools/list` | returns the 7 builtin tool names (fs_read, fs_write, edit, multi_edit, grep, glob, bash) |
| R2 | `server_calls_fs_read_tool` (pii-mcp unit) | `tools/call` with `fs_read` on a temp file | returns the file content |
| R2 | `server_calls_bash_tool` (pii-mcp unit) | `tools/call` with `bash` `echo hi` | returns `hi` |
| R3 | `client_lists_remote_tools` (pii-mcp unit) | point the client at a fake MCP server | lists its tools |
| R3 | `client_calls_remote_tool` (pii-mcp unit) | call a tool on the fake server | returns the result |
| R4 | `mcp_feature_off_excludes_crate` (build check) | default `cargo build` | `pii-mcp` not compiled; `pii mcp` subcommand absent |

**R2 (`server_lists_builtin_tools`) is the acceptance test.** The requirement a
plausible implementation gets wrong: it exposes the control API (session/run)
instead of the tools, or it doesn't map `ToolSet::builtin` correctly. The test
asserts the 7 builtin tools appear.

---

## Exit Criteria

- [ ] `cargo build -p pii-mcp --features mcp` — compiles (R1)
- [ ] `cargo test -p pii-mcp` — all pass (R2, R3)
- [ ] `cargo test --workspace` — 0 failures (N2)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N2)
- [ ] `cargo fmt --all --check` — clean (N2)
- [ ] `cargo xtask lint-files` — 0 hard errors (N2)
- [ ] `cargo xtask deps-check` — pass (N1, N3)
- [ ] `cargo xtask net-check` — pass (N3)
- [ ] Manual: `hermes mcp add pii --command <pii-mcp>` → `mcp_pii_*` tools appear in a new Hermes session

**Prose criteria**:

1. Test counts pasted raw, one line per binary, **unsummed**.
2. The server must expose pii's TOOLS (fs/edit/bash/grep/glob), not the control API (session/run/approve) — a control-API-over-MCP implementation is a FAIL.

---

## Guardrails

- **G1 — do NOT edit this spec.** If it is wrong, STOP and report it.
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT regenerate a pinned fixture.**
- **G5 — no hardcoded absolute paths.** Test artefacts under `_tmp/`.
- **G6 — report raw output, never summed totals.**
- **G7 — feature-gated, default off.** The default build must have none of `pii-mcp` (pluggable convention).
- **G8 — `rmcp` confined to `pii-mcp`.** No new HTTP stack in core crates (deps-check).
- **G9 — server exposes TOOLS, not the control API.** Full control stays on UDS JSON-RPC.

### Error handling expectations

- A tool call that fails (e.g. `bash` returns non-zero) must surface as an MCP tool error, not a crash.
- The server must handle a missing daemon gracefully (report it, not hang).
- The client must handle a remote server that disconnects (return an error, not panic).

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/pii-mcp/Cargo.toml` | new crate: `rmcp`, `pii-tools`, `pii-proto` (client), `pii-server` (uds); feature `mcp` |
| `crates/pii-mcp/src/main.rs` | `pii mcp` server + client entry |
| `crates/pii-mcp/src/server.rs` | `ToolSet::builtin` → MCP tools (R2) |
| `crates/pii-mcp/src/client.rs` | external MCP server → pii tools (R3) |
| `crates/pii-cli/Cargo.toml` | optional `pii-mcp` dep behind `mcp` feature (R4) |
| `crates/pii-cli/src/main.rs` | `#[cfg(feature = "mcp")]` `pii mcp` subcommand (R4) |
| `Cargo.toml` | add `crates/pii-mcp` to workspace members |
| `xtask/src/deps.rs` | rmcp-confined-to-pii-mcp invariant (N1) |

**Not modified**: `crates/pii-tools/` (the toolset is already there), `crates/pii-server/` (no daemon change), `crates/pii-proto/` (no wire change).

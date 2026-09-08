# Pii — Feature-Gating Subsystems (how to make the monolith compilable-down)

**Date**: 2026-09-08 · **Author**: orchestrator · **Status**: DESIGN NOTE — pattern for optional subsystems

## Why this exists

Pii is intentionally a single-process, event-sourced monolith (one redb store,
single-writer; journal + bus + approval gate + budget are coherent in-process).
That monolith is its strength. But the installer may want to build **a smaller
or more hardened binary** without shipping every subsystem. Cargo **features**
are the right lever for *"what's in the build"* — NOT for *"what's a
microservice."* A feature trims the compile; it does not create a process/IPC
boundary. (Service boundaries are a separate decision, deferred.)

## The core convention

Gate **surfaces with risk or optional deps**; never gate the **engine substrate**.

- **Gate**: render surfaces (tui, web), token/network subsystems (channels,
  github), the optimizer (evolve), memory if store-lean matters, provider
  backends, the sandbox (landlock), the TLS variant.
- **Never gate**: the substrate the engine needs to run — `pii-abi`, `pii-core`,
  `pii-graph`, `pii-proto`, `pii-tools`, `pii-net` (the socket chokepoint is the
  product), `pii-server`, `pii-store`.

## Every subsystem worth a feature

| Feature | What it gates | Why gate it | Status |
|---|---|---|---|
| `evolve` | the self-improving boss (writes code, spends tokens, posts GitHub) | **MUST be opt-in** — the risk surface is deliberate | user-decided, upcoming `pii-evolve` crate |
| `web` | `pii-web` viewer | already a detached crate; non-default | near-free |
| `tui` | interactive TUI | default build can be headless (`pii -p`/`pii daemon`) | needs optional-dep + cfg-gating |
| `channels` | Discord/outbound messaging (token + egress) | **security** win — don't ship code that dials out | candidate |
| `tls` | rustls/HTTPS outbound in `pii-net` (already confined to pii-net) | smaller non-TLS builds; only needed by https providers/evolve-PR | candidate |
| `memory` | pii-memory (global store + embedder) | store-lean deployments | candidate |
| `sandbox` | Landlock OS confinement | skip where Landlock absent (some containers) | candidate |
| `github` | future `pr`/issue capability for evolve/heal | opt-in network+token surface | future |
| `provider-*` | per-model-backend (gemini/anthropic/ollama/…) | carry only the providers you need | candidate |

## The strongest argument

Beyond size, **two of these are a security win**: `channels`, `tls`, and
`github` all do network egress and may carry a token. A feature that *excludes
the code* is stronger than a runtime config that *disables* it — the dialing
code isn't compiled in, so it cannot be reached. This is the case for treating
network/token subsystems as opt-in features specifically.

## Mechanical pattern (tui as the worked example)

```toml
# pii-cli/Cargo.toml
[dependencies]
pii-tui = { path = "../pii-tui", optional = true }   # was unconditional

[features]
default = []
tui = ["dep:pii-tui"]
```

```rust
// pii-cli/src/main.rs
#[cfg(feature = "tui")] mod cmd_tui;
#[cfg(feature = "tui")] if cmd == "tui" { return cmd_tui::cmd_tui(&mut args); }
```

`web` is even easier — it is already a detached crate (`pii-web`) with no
reference in `pii-cli/src/main.rs`; it just needs a `web` feature that pulls it
in.

## Build

- Default: `cargo build` — monolith minus gated subsystems.
- With a subsystem: `cargo build --features tui,evolve` (or the installer's
  chosen set).
- Each feature is independent, so the install can build *any* combination.

## Honest caveat

A feature **trims**; it does not **service-ize**. Gating `memory` as a feature
means "this build has no memory code" — it does NOT mean "memory runs in its
own process." If a subsystem later needs its own lifecycle (cross-process lease,
independent scale), that's a separate service/IPC design, deferred by the opus
plan. The feature gate is complementary and makes a future detach cleaner.

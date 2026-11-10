# pi-improved — spec-47: reorganize `pii-server` into `engine/`, `node_kinds/`, `daemon/` module boundaries

- **Project**: `<repo>`
- **Priority**: HIGH — Phase 5.2, F-13 from the 026 report, OPTION 1 of 3
  reviewed options (see the workspace memory `pii/phase5.2/f13-scope-options-for-opus` for
  the other two, deferred to a future opus review, not executed here).

## Overview

`pii-server` fuses three concerns in one 7,632-line crate: an **engine
bridge** (journal→bus, run spawning, JSON-RPC dispatch, session
durability), **server node kinds** (`agent`, `gate`, `memory`, `heal`,
`worker`, `spawn`, `crash`), and a **daemon app** (lifecycle, timer wheel,
cron, approvals, channel router — already partly separated as `daemon/`).

**This spec does NOT split `pii-server` into separate crates.** A real
coupling problem rules that out for now: `dispatch.rs` (engine-bridge)
calls `daemon::methods` (daemon-app) for ~10 JSON-RPC methods
(`WorkerList/Reload`, `ApproveList/Resolve`, `HealList/Get`, `CronList`,
`ChannelList`, `MemorySearch/Put/Consolidate`, `SysReload`, `RunResume`) —
untangling that requires either moving routing logic or accepting a real
crate dependency edge that would partly defeat the point of splitting.
That decision is EXPLICITLY DEFERRED to a future opus-reviewed spec (see
the workspace memory key above) — this spec is the safer, lower-risk half: **reorganize
the concerns into clear module directories inside the SAME crate**, with
a documented boundary, so the eventual crate split (if/when opus greenlights
it) has much less work left to do.

## Requirements

### Functional

- R1. Create `crates/pii-server/src/engine/` and move the engine-bridge
  files into it: `bus.rs`, `runs.rs`, `dispatch.rs`, `subscribe.rs`,
  `registry.rs`, `session_durability.rs`, `conn.rs`, `uds.rs`,
  `loopback.rs`, `tool.rs`. Each keeps its exact current module name
  (`bus`, `runs`, `dispatch`, etc.) but now lives at `engine::bus`,
  `engine::runs`, etc. — i.e. this is `mv crates/pii-server/src/bus.rs
  crates/pii-server/src/engine/bus.rs` for each file, plus a new
  `engine/mod.rs` that does `pub mod bus; pub mod runs; ...` and
  re-exports whatever `lib.rs` needs.
- R2. Create `crates/pii-server/src/node_kinds/` and move the
  already-directory-shaped node-kind modules into it: `agent/` (the
  whole directory, from spec-43's split) and `nodes/` (the whole
  directory, containing `crash.rs`/`gate.rs`/`heal.rs`/`memory.rs`) plus
  `compaction.rs` (agent-support, used only by `agent/turn.rs` — verify
  this via grep before moving, don't assume). Result:
  `node_kinds::agent`, `node_kinds::nodes`, `node_kinds::compaction`.
- R3. `daemon/` stays exactly where it is (`crates/pii-server/src/daemon/`)
  — it is already the one concern with a clean existing boundary; do not
  move or rename anything inside it.
- R4. `lib.rs` updates its `pub mod` declarations to `pub mod engine; pub
  mod node_kinds; pub mod daemon;` (daemon unchanged) and any `pub use`
  re-exports adjust their paths accordingly (e.g. `pub use
  engine::registry::{lint_registry, registry};` if that re-export is kept
  — verify against R6, don't break any existing public re-export
  silently).
- R5. Update EVERY `use crate::X` / `crate::X::Y` reference across the
  whole crate (`src/` AND `tests/`) to the new paths
  (`crate::engine::X`, `crate::node_kinds::X`). This is a mechanical
  rename — do NOT change what anything does, only where it lives. Grep
  `crate::(bus|runs|dispatch|subscribe|registry|session_durability|conn|
  uds|loopback|tool|agent|nodes|compaction)::` across the whole
  workspace (not just `pii-server`) first, since other crates (`pii-cli`,
  `pii-mcp`) may reference `pii_server::` paths that also need updating —
  confirmed by the orchestrator: `pii_server::(dispatch|daemon|nodes|
  runs|bus|registry)` has real hits in `pii-server/tests/*.rs` AND
  `pii-cli/src/{cmd_tui,cmd_oneshot,cmd_daemon}.rs`. Update all of them.
- R6. Add a short module-boundary doc comment to `lib.rs` (or a new
  `engine/mod.rs`/`node_kinds/mod.rs` doc comment) explaining the three
  concerns and noting the known `dispatch.rs`→`daemon::methods` coupling
  as a DOCUMENTED, not-yet-resolved constraint — so a future reader
  (including the opus review that will decide on the full crate split)
  starts from an accurate map instead of re-discovering the coupling.

### Non-Functional

- N1. Every moved file's byte content is UNCHANGED except `use`/path
  updates required by the move — this is a pure reorganization, not a
  rewrite. No logic changes, no test changes beyond import-path fixes.
- N2. Each file individually still satisfies `xtask lint-files` (≤450
  effective lines) — none of the moved files should need splitting
  further as part of this spec (if one does, STOP and report, don't
  silently also split it — that's separate scope).
- N3. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()`
  anywhere. No `unsafe` code.

## Architecture

```
crates/pii-server/src/
├── lib.rs                  — Engine type, EngineConfig, top-level pub mod
├── engine/                 — NEW: the engine-bridge concern
│   ├── mod.rs
│   ├── bus.rs
│   ├── runs.rs
│   ├── dispatch.rs
│   ├── subscribe.rs
│   ├── registry.rs
│   ├── session_durability.rs
│   ├── conn.rs
│   ├── uds.rs
│   ├── loopback.rs
│   └── tool.rs
├── node_kinds/              — NEW: the server node-kinds concern
│   ├── mod.rs
│   ├── agent/               — (moved as-is from src/agent/, spec-43's split)
│   ├── nodes/                — (moved as-is from src/nodes/)
│   └── compaction.rs
└── daemon/                  — UNCHANGED: the daemon-app concern
    ├── mod.rs
    ├── approvals.rs
    ├── config.rs
    ├── methods.rs
    ├── router.rs
    ├── scheduler.rs
    └── status.rs
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| (all existing tests across `pii-server/src/**` and `pii-server/tests/*.rs`, unchanged assertions) | — | every test that passed before the reorg still passes after — this is the primary proof of a pure move |
| `pii_server_module_boundaries_documented` (new, wherever the doc comment from R6 lives) | grep for the documented `dispatch.rs`→`daemon::methods` coupling note | present, non-empty |

## Exit Criteria

- [ ] `[ -d crates/pii-server/src/engine ]`
- [ ] `[ -d crates/pii-server/src/node_kinds ]`
- [ ] `[ ! -f crates/pii-server/src/bus.rs ]`
- [ ] `[ ! -f crates/pii-server/src/runs.rs ]`
- [ ] `[ ! -f crates/pii-server/src/dispatch.rs ]`
- [ ] `[ ! -d crates/pii-server/src/agent ]`
- [ ] `[ ! -d crates/pii-server/src/nodes ]`
- [ ] `[ -d crates/pii-server/src/daemon ]`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`

## Guardrails

- G1. Do NOT change any test's assertions, names, or add/remove any test
  beyond import-path fixes forced by the move.
- G2. Do NOT touch `crates/pii-server/src/daemon/` — it already has the
  right boundary; moving or renaming anything inside it is out of scope.
- G3. Do NOT attempt the full crate split (separate `pii-engine`/
  `pii-nodes`/`pii-daemon` crates) — that is a DIFFERENT, deferred option
  requiring an opus-reviewed decision on the `dispatch.rs`→
  `daemon::methods` coupling. This spec is module reorganization ONLY,
  inside the single `pii-server` crate.
- G4. Do NOT touch `specs/` (read-only for the workhorse).
- G5. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G6. If moving `compaction.rs` (R2) reveals it is used by something
  other than `agent/turn.rs`, STOP and report rather than guessing where
  it belongs — re-verify with a grep before finalizing its new location.
- G7. If any external crate (`pii-cli`, `pii-mcp`, or any other workspace
  member) references a `pii_server::` path this spec moves, that
  reference MUST be updated in the same change — a reorg that breaks
  another crate's build is not "done," it is half-done.

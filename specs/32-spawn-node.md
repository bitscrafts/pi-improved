# pi-improved — spec-32: spawn-node-and-worktree-provisioning

- **Project**: `/projects/pi-improved`
- **Priority**: HIGH — closes the gap between `boss-watch` (detect + gate) and
  `heal-fix` (localize→patch→validate). Today `boss-watch`'s `escalate` gate is
  terminal in both directions; nothing turns an approved detection into a
  scoped `heal-fix` run. The boss cannot heal — it files tickets.

## Verified Premises

- There is **no `subgraph`/`spawn`/`map` node kind** — searched
  `crates/pii-graph/src/node.rs:43-55` plus `crates/pii-server/src/registry.rs:23-55`
  (12 kinds total: agent, tool, gate, heal_detect, memory, debug_crash, worker,
  timer, transform, join, ... none chain runs).
- `Engine::start_run` (`crates/pii-server/src/lib.rs:355`) is the same path
  `run.start` uses (`crates/pii-server/src/dispatch.rs:153-172`).
- The registry is built INSIDE `Engine::new` (`lib.rs:192-201`) — a spawn node
  factory cannot hold the `Arc<Engine>` directly (chicken-and-egg). The
  `worker` node solves this by carrying a shared pool Arc the daemon fills
  after construction (`crates/pii-server/src/nodes/worker.rs:50-85`). The spawn
  node uses the same pattern: a shared `Arc<Mutex<Option<Arc<Engine>>>>` handle.
- `heal_seed`'s git worktree logic (`xtask/src/heal_seed.rs:81-101`) creates
  `git worktree add -b heal/<sig> <path> HEAD` under `_tmp/heal-worktrees`.
- `HealAttempt` already carries `worktree`, `branch`, `base_commit`
  (`crates/pii-store/src/heal.rs:86-88`).
- `validate_scope_root` (`crates/pii-server/src/runs.rs:261-283`) requires
  `scope_root` to be an absolute path inside the workspace — so worktrees must
  stay under `_tmp/heal-worktrees` (do NOT relocate to `/root/.pii-dev/`).
- All 4 budget dims now thread through `Budget::new` (`runs.rs:56-61, 169`).

## Overview

Add a `spawn` node kind (registered in `pii-server`, not `pii-graph` — it
needs the `Engine`) that calls the same path as `run.start` to launch a child
run, plus a `worktree` provisioning step reachable from a graph. Wire
`boss-watch.toml` so an approved detection provisions a worktree and spawns a
scoped `heal-fix` run.

## Requirements

### Functional

- **R1 (spawn node)**: a `spawn` node kind: given `graph` + an input
  expression, it calls the same path as `run.start`
  (`crates/pii-server/src/dispatch.rs:153-172`), returns the child `run_id` on
  `ok`, and journals a `RunSpawned`-style link so `pii graph trace` shows the
  parent→child edge.
- **R2 (worktree provisioning)**: a `worktree` provisioning step reachable
  from a graph: promote `heal_seed`'s git logic (`xtask/src/heal_seed.rs:81-101`)
  into a callable form and expose it as a `tool` (not an agent capability),
  returning `{worktree, branch, base_commit}` — the exact fields `HealAttempt`
  already carries.
- **R3 (child scope_root)**: the child run's `scope_root` is the worktree;
  `validate_scope_root` (`runs.rs:261-283`) must accept it, which requires
  worktrees to stay under the workspace (`_tmp/heal-worktrees`).
- **R4 (boss-watch wiring)**: `boss-watch.toml` gains
  `escalate --allow→ provision → spawn(heal-fix) → done`; `deny` still
  terminates. Budget for the child run is bounded by `[limits]`.
- **R5 (rate limit)**: refuse to spawn a heal for a signature whose
  `attempt_count` (`heal.rs`) is ≥ 2 — route to a second escalation gate
  instead (008 §6).

### Non-Functional

- **N1**: the quality gate stays green; the test count only goes up.
- **N2**: no new third-party dependencies.
- **N3**: `silent-noop-check` (spec 24) stays green; new fields/types must have
  a producer.
- **N4**: the spawn node is a capability — it only spawns runs the graph
  explicitly names, never arbitrary code.

## Architecture

The `spawn` node factory carries a shared `Arc<Mutex<Option<Arc<Engine>>>>`
handle (the `worker` node pattern). The daemon fills it after `Engine::new`.
At dispatch, the node locks the handle, calls `engine.start_run(graph, input)`,
and returns the child `run_id` on `ok`. A `RunSpawned` event (new variant in
`pii-proto`/`pii-abi`) journals the parent→child link.

The `worktree` provisioning is a `tool` (like `bash`/`grep`) that shells
`git worktree add -b heal/<sig> <path> HEAD` under `_tmp/heal-worktrees`,
returning `{worktree, branch, base_commit}`.

## TDD Contract

| Test | Input | Expected Output |
|---|---|---|
| `spawn_node_starts_child_run` (T1) | graph with `spawn(graph="starter/verify-fix")` | a second run header exists in the store, `status = Running`, and its `input` carries the parent's payload |
| `spawn_child_scope_root_is_worktree` (T2) | spawn with `scope_root = <wt>` | child ctx denies `fs_write` to `<workspace>/crates/...` and allows `<wt>/crates/...` |
| `provision_returns_branch_and_base_commit` (T3) | provision against a temp clone | `{worktree, branch, base_commit}` all non-empty; `git worktree list` shows the new entry |
| `spawn_refused_after_two_attempts` (T4, acceptance) | `HealAttempt{attempt_count: 2}` for the signature | no child run is created; routes to the escalation port |

T4 is the one a plausible implementation gets wrong — it will spawn and THEN
check.

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-server spawn_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — spawn node tests (R1, R3, R5)
- [ ] `cargo test -p pii-server provision_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — worktree provisioning (R2)
- [ ] `cargo test -p pii-server boss_watch_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — boss-watch wiring (R4)
- [ ] `cargo test --workspace` — 0 failures (N1)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N1)
- [ ] `cargo fmt --all --check` — clean (N1)
- [ ] `cargo xtask silent-noop-check` — green (N3)

## Guardrails

- **G1 — do NOT edit this spec.**
- **G2 — do NOT commit.** Leave work in the working tree for orchestrator review.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — the spawn node only spawns runs the graph explicitly names.**
- **G5 — worktrees stay under `_tmp/heal-worktrees`** (do NOT relocate to
  `/root/.pii-dev/` — `validate_scope_root` requires workspace-relative).
- **G6 — do NOT touch `specs/`** (read-only for implementers).

## Files to Modify

- `crates/pii-server/src/nodes/spawn.rs` — the `spawn` node + factory (new).
- `crates/pii-server/src/registry.rs` — register the spawn factory with the
  shared engine handle.
- `crates/pii-server/src/lib.rs` — add the shared engine handle to the Engine
  (or a setter the daemon calls after `Engine::new`).
- `crates/pii-server/src/daemon/mod.rs` — fill the engine handle after
  construction.
- `crates/pii-tools/src/` — the `worktree` provisioning tool (R2).
- `crates/pii-proto/src/event.rs` / `crates/pii-abi/src/journal.rs` — the
  `RunSpawned` event variant (R1).
- `graphs/starter/boss-watch.toml` — the `provision → spawn(heal-fix)` wiring
  (R4).
- Tests: `crates/pii-server/tests/` for the TDD contract.

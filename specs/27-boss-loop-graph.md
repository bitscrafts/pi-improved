# pi-improved — spec-27: boss-loop-graph

- **Project**: `/projects/pi-improved`
- **Priority**: Make 008 §3/§4's central claim — "the boss runs on the substrate
  it heals" — true. Spec 23 shipped a crate whose `driver::run_cycle` is a
  hand-rolled `for` loop shelling out to `cargo xtask`. This spec replaces it
  with a real graph run on the daemon (dogfooding).
- **Status**: IN PROGRESS · *(lifecycle: SPECIFIED → IN PROGRESS on dispatch →
  IMPLEMENTED — awaiting review → DONE after the orchestrator commits and updates
  HANDOFF.md)*
- **Source**: `_docs/06-research/011-pii-architecture-conformance-review-pass1.md`
  Part B spec 27 (fixes A15) + `_docs/06-research/008-pii-evolve.md` §3/§5 (Slice B).
- **Depends-On**: spec 24 (silent-noop-check = the gate the graph runs), spec 25
  (pii-evolve honesty / standalone opt-in crate), spec 26 (runtime approval
  cycle — promotions must suspend fresh, never auto-allow). Green at HEAD.

---

## Verified Premises

Checked before writing/amending (2026-09-09):

- `graphs/starter/heal-watch.toml` — the existing S0.3 graph: `timer` (cron
  `*/30`) → `heal_detect` (`gate = "reachability-check"`) → `escalate` (`gate` node,
  policy "heal-escalation") → `done`. This is the exact shape to model the boss watch
  on (`caps = ["proc"]`, no `fs_write`, read-only detection).
- `crates/pii-server/src/nodes/gate.rs:80-84` reads `input.payload.cycle` at
  runtime (spec 26 R1). If `payload` lacks `cycle`, it falls back to `cfg.cycle = 0`,
  finds its cached decision, and auto-allows: **A17a, resurrected inside the boss.**
  Because `heal_detect`'s `detected` value is `{"signature_hash", "detail"}` without
  `cycle`, the graph MUST route through a `cyc` transform node (`expr = "$.input"`)
  before `gate` to forward the run's root input (carrying `cycle`).
- `crates/pii-server/src/registry.rs:30-50` — `heal_detect` and `gate` are owned
  by `pii-server`'s registry, not `pii-graph`'s builtin registry. A suspending run
  runs on the daemon control plane (`daemon/mod.rs:272` creates the approval row,
  `pii approve` resumes it).
- `crates/pii-evolve/src/driver.rs` — hand-rolled `GateResult` loop to replace.
- Spec 25 delivered `pii-evolve` as a **standalone opt-in crate** depending on
  `pii-core` ONLY (no `evolve` Cargo feature). It is a client that connects over UDS.
- `state_write` is DEFERRED to Slice C (008 §5:164) where context compaction and
  sessions land; `pii-server`'s source is NOT modified, preserving `pii-evolve`'s
  isolation and pii-server's clean boundary.

---

## Overview

008 §3/§4 and 009 make the boss's core identity: *"The boss runs on the substrate
it heals — a pii graph, using pii's own journal, gates, approvals, budget, memory,
and model hierarchy. Not a bolt-on service."*

Today that claim is false. `pii-evolve`'s `driver::run_cycle` is a for loop that
shells out to `cargo xtask`. There is no journal event, no `gate` node, no
`ApprovalStore` row.

This spec converts the boss into a real graph, modeled on the existing
`heal-watch.toml` (whose shape is right). `pii-evolve`'s driver becomes a thin
UDS launcher that sends `run.start` for `starter/boss-watch` with
`input: { state_dir, cycle }` to the daemon, records `last_run_id` + `cycle_count` in
`state.md`, and returns.

The dogfooding claim becomes true: the boss WATCH is a pii graph run on the daemon
with journaled nodes, a `gate` node that (after spec 26) suspends fresh on every
attempt, and the `pii approve` path actually resumes it.

**No model calls anywhere in this spec** — detection, gating, and the ledger are
all deterministic nodes. The boss's "decide what to do next with a model" (spec 28)
comes later.

---

## Requirements

### Functional

- **R1 (`boss-watch` graph)**: a new `graphs/starter/boss-watch.toml` modeled on
  `heal-watch.toml`: `timer` (cron `*/30 * * * *`) → `check` (`heal_detect`,
  `gate = "silent-noop-check"`, `caps = ["proc"]`).
  - `check.ok`/`check.flake` → `done` (`transform`, `join = "any"`, `value = "no-silent-noop"`).
  - `check.detected` → **`cyc` (`transform`, `expr = "$.input"`)** → `escalate`
    (`gate`, `policy = "boss-escalation"`, `prompt = "Boss detected silent-noop-check failure. Approve promotion/heal attempt?"`).
  - `escalate.allow`/`escalate.deny` → `done`.
  - Header comment states: no `FS_WRITE`, no `agent` node, periodicity via the daemon scheduler.
- **R2′ (`driver::run_cycle` becomes thin launcher)**: `pii-evolve` gains
  `pii-proto = { path = "../pii-proto", features = ["client"] }` + workspace `tokio` +
  `serde_json` (no `pii-graph`, `pii-store`, or `pii-server` deps). `run_cycle(cfg: &BossLaunch)`
  loads `state.md`, increments `cycle_count`, connects to UDS socket
  (`<runtime_dir>/control.sock`), sends `run.start` for `starter/boss-watch` with
  `input: { "state_dir": ..., "cycle": cycle_count }`, records `last_run_id` +
  `cycle_count` + `phase = "watching"` in `state.md`, and returns `LaunchedRun { run_id, cycle }`.
  All manual `cargo xtask` shelling, `GatePolicy`, and `CycleOutcome::PendingHuman` are
  deleted from `pii-evolve`.
- **R3 (`PendingHuman` becomes a real daemon suspend)**: In the graph, "pending human" =
  the `gate` node suspending with a fresh `approval_id` (spec 26 R1/R2) and a
  `HealAttempt` ledger row carrying `decision: None`. `pii approve <id> --allow`
  resumes the run on the `allow` port; `--deny` terminates on `deny`. Re-running with
  a fresh cycle in input mints a fresh approval id and suspends again.
- **R4 (CLI client)**: `pii-evolve` CLI accepts `--socket PATH` (default
  `$PII_RUNTIME_DIR/control.sock` or `~/.pii/runtime/control.sock`). It prints the
  launched `run_id` and exits 0 on successful dispatch.
- **R5 (no model, bounded)**: the boss-watch graph contains ONLY
  `{timer, heal_detect, transform, gate}` nodes — NO `agent` node (R5). Every node has
  `caps = ["proc"]` or empty — NO `fs_write`.

### Non-Functional

- **N1**: `pii-server` source (`crates/pii-server/src/`) is UNTOUCHED — only a new
  test file `crates/pii-server/tests/boss_watch.rs` is added to test the graph on
  the daemon registry.
- **N2**: no new third-party dependencies.
- **N3**: the quality gate stays green; workspace tests only increase.
- **N4**: `silent-noop-check` (spec 24) stays green — zero unwired surfaces added.
- **N5**: never operate on prod or the live tree; `boss-watch.toml` has no `fs_write`
  capabilities.

---

## Architecture

```
[cron */30] timer -> check: heal_detect(gate=silent-noop-check, caps=[proc])
    -> ok / flake -> done(transform value="no-silent-noop")
    -> detected   -> cyc: transform(expr="$.input")  <- FORWARDS ROOT INPUT WITH CYCLE
                       -> escalate: gate(policy=boss-escalation)
                            -> allow / deny -> done

pii-evolve (standalone client crate)
    driver::run_cycle -> StateDoc::load -> cycle++ -> UDS client -> run.start("starter/boss-watch", {state_dir, cycle})
                      -> record last_run_id + cycle_count in state.md -> return LaunchedRun
    deps: pii-proto[client] + pii-core + tokio + serde_json ONLY
```

---

## TDD Contract

| id | test | file | given | expects |
|----|------|------|-------|---------|
| R1 | `boss_watch_validates_on_daemon_registry` | `pii-server/tests/boss_watch.rs` | load real `graphs/starter/boss-watch.toml` via `Engine::load_spec` | validates OK; no `agent` node; no `fs_write` cap anywhere |
| R1 | `boss_watch_green_completes` | `pii-server/tests/boss_watch.rs` | `HealFactory` with stub runner returning pass | drive run past timer; journal has `Suspended{Timer}` then `NodeFinished` for `check`; terminates on `done` |
| R3 | `gate_suspends_and_approve_resumes` | `pii-server/tests/boss_watch.rs` | stub runner fails twice | journal shows `Suspended{Approval}`; `engine.approvals.pending_approvals()` has row; `approvals::resolve(id, true)` resumes to `allow -> done`; `--deny` terminates on `deny` |
| R3b | `second_cycle_suspends_with_fresh_id` | `pii-server/tests/boss_watch.rs` | run twice with `input.cycle = 1` then `2` | two different approval_ids; run 2 suspends fresh (does NOT reuse run 1 allow); **MUTATION CHECK: deleting `cyc` node makes this test FAIL** |
| R2′ | `launch_records_run_id_in_state_md` | `pii-evolve/tests/launcher.rs` | call `run_cycle` against stub UDS server | `state.md` carries `last_run_id` equal to stub's run id, `cycle_count` incremented, `phase = "watching"` |
| R4 | `run_cycle_sends_run_start` | `pii-evolve/tests/launcher.rs` | call `run_cycle` against stub UDS server | stub receives `run.start` with `graph == "starter/boss-watch"` and `input.cycle == cycle_count + 1`; driver spawns NO subprocess |
| R5 | `no_model_node_in_boss_watch` | `pii-server/tests/boss_watch.rs` | parse `boss-watch.toml` | kind set ⊆ `{timer, heal_detect, transform, gate}` |

---

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-server boss_watch_` — graph validate + run + gate-approve + fresh-cycle + no-model tests (R1, R3, R3b, R5)
- [ ] `cargo test -p pii-evolve launcher_` — launcher state.md + UDS `run.start` tests (R2′, R4)
- [ ] `git diff --quiet -- crates/pii-server/src` — pii-server source UNTOUCHED (N1)
- [ ] `cargo test --workspace` — 0 failures (N3)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N3)
- [ ] `cargo fmt --all --check` — clean (N3)
- [ ] `cargo xtask silent-noop-check` — green (N4)
- [ ] `git diff --quiet -- specs xtask quality-gate.sh .github` — those paths untouched (G5)

---

## Guardrails

- **G1 — do NOT edit this spec.**
- **G2 — do NOT commit.** Leave work in the working tree for orchestrator review.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — NO model node in the boss-watch graph.**
- **G5 — do NOT touch `crates/pii-server/src/`, `specs/`, `xtask/`, `quality-gate.sh`, `graphs/starter/*` (except ADDING `boss-watch.toml`), `.github/`, or any `#[test]` assertion.**
- **G6 — do NOT operate on prod or live working tree.**
- **G7 — no hardcoded absolute paths.** Test artefacts under `_tmp/`.
- **G8 — report raw output, never summed totals.**

---

## Files to Modify

| File | Change |
|------|--------|
| `graphs/starter/boss-watch.toml` | NEW: boss loop graph (R1) |
| `crates/pii-server/tests/boss_watch.rs` | NEW: graph execution + suspend/resume + fresh cycle tests (R1, R3, R3b, R5) |
| `crates/pii-evolve/Cargo.toml` | add `pii-proto = { path = "../pii-proto", features = ["client"] }`, `tokio`, `serde_json` (R2′) |
| `crates/pii-evolve/src/driver.rs` | convert `run_cycle` into UDS launcher; delete `GatePolicy`, manual `cargo xtask` loop, `CycleOutcome::PendingHuman` (R2′) |
| `crates/pii-evolve/src/main.rs` | CLI connects via `--socket` path (R4) |
| `crates/pii-evolve/src/lib.rs` | update doc comments to state client launcher identity |
| `crates/pii-evolve/tests/launcher.rs` | NEW: launcher unit tests with stub UDS server (R2′, R4) |

**Not modified**: `crates/pii-server/src/` (N1), `pii-cli`, `pii-mcp`, `pii-web`, `specs/`.

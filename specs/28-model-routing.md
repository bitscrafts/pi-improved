# pi-improved — spec-28: model-routing-wired-into-agent-layer

- **Project**: `/projects/pi-improved`
- **Priority**: Fix A16 (dead `routing.rs`/`context.rs` from spec 23) properly —
  wire model routing into the agent node's config, not as a new node kind.
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
  on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
  commits and updates HANDOFF.md)*
- **Source**: `_docs/06-research/011-pii-architecture-conformance-review-pass1.md`
  Part B spec 28 (fixes A16) + `_docs/06-research/008-pii-evolve.md` §3.2.
- **Depends-On**: spec 25 (pii-evolve honesty — deleted the dead `routing.rs`),
  spec 26 (heal-ledger `attempt_count` for the escalation rule). Green at HEAD.

---

## Verified Premises

Checked before writing (2026-09-10):

- `crates/pii-server/src/agent.rs:109-112` — `AgentNode` already reads an optional
  `model` from the node config: `cfg.get("model").and_then(as_str)`. The wiring
  point is this config key.
- `crates/pii-server/src/agent.rs:19-34` — `AgentNode` struct fields; `model:
  Option<String>`.
- `crates/pii-server/src/daemon/config.rs:148-162` — `Config` has `[core]`,
  `[net]`, `[[worker]]`, `[[channel]]`, `[memory]`, `[[cron]]` sections, all
  `deny_unknown_fields`. There is NO `[evolve]` section yet — spec 28 adds one
  for the routing policy.
- `crates/pii-server/src/agent.rs:155` — the turn loop head; `:355` — the tool
  dispatch site (spec 29's `ctx.check()` points).
- `crates/pii-server/src/agent.rs:214-228` — the token-budget charge already
  routes to the `budget` port on exhaustion.
- `crates/pii-graph/src/run.rs:257-272` — the run loop checks `Tokens` and
  `MicroUsd` remaining; `MicroUsd` is never charged (A7) — spec 29 handles that.
- `crates/pii-evolve` — `routing.rs`/`context.rs` were DELETED in spec 25 (dead
  code). Spec 28 re-adds routing as a config-driven policy, NOT a crate module.

---

## Overview

008 §3.2's model routing ("per-request decision, not one-time setting; explicit
and auditable") was shipped in spec 23 as `routing.rs` — a module with no caller
but its own tests (A16). Spec 25 deleted it. This spec re-adds routing the way
009 item 6 said it should be done: **the wiring point is the agent node's config,
not a new node kind.**

`AgentNode` already reads an optional `model` from its config. Spec 28 adds a
`route_step` key: when present, the node resolves the model through a
`RoutePolicy` loaded from a new `[evolve.routing]` config section, and records
each `RouteDecision` into the journal so `pii graph trace` shows it. The 008 §3.2
escalation rule ("escalate after 2 attempts") reads the `attempt_count` spec 26
adds to the heal ledger.

**No new node kind.** The `route` node already exists and routes control flow,
not models. Model routing belongs in the agent/config layer.

---

## Requirements

### Functional

- **R1 (`[evolve.routing]` config)**: a new `EvolveSection` in
  `crates/pii-server/src/daemon/config.rs` with a `routing` sub-table mapping a
  step name (`localize`, `patch`, `review`, `research`, `orchestrator`) to a model
  spec string. `deny_unknown_fields` applies. Default: empty (no routing — agent
  nodes behave exactly as today).
- **R2 (`route_step` node key)**: `AgentNode`'s config gains a `route_step` key.
  When present, the node resolves its model via the `RoutePolicy` from
  `[evolve.routing]`; when absent, it uses the existing `model` key (or the
  provider default) — zero behavior change for non-routed graphs.
- **R3 (broadcast the decision)**: each `RouteDecision` (step → resolved model) is
  broadcast on the live bus (F12 — the agent node's live events are bus-only,
  never journalled, exactly like `ToolCall`/`ToolResult`/`AssistantDelta`), so
  subscribers and `pii graph trace`-style live views see it.
- **R4 (loud on unassigned step)**: a `route_step` that has no entry in
  `[evolve.routing]` fails the node with `NotFound` — it must NOT silently
  default. 008 §1 is explicit that a single misroute cascades, so it must be loud.

### Non-Functional

- **N1**: no new third-party dependencies.
- **N2**: default build unchanged — no `[evolve.routing]` config means no routing
  behavior.
- **N3**: the quality gate stays green; test count only goes up.
- **N4**: `silent-noop-check` (spec 24) stays green — the new `route_step` key and
  `[evolve.routing]` section must not add an unconsumed surface.

---

## Architecture

```
[evolve.routing]  (new config section, deny_unknown_fields)
  localize = "google:gemini-3.6-flash"
  patch    = "nvidia:deepseek-ai/deepseek-v3.2"
  review   = "claude-sonnet-5"
  research = "glm-5.2"

agent node config:
  { "kind": "agent", "route_step": "patch", ... }
  -> AgentNode::build resolves model via RoutePolicy[step]
  -> if step missing from policy: Err(NotFound)  (R4, loud)
  -> journal Event::RouteDecision { step, model }  (R3)

AgentNode::run uses self.model (resolved) exactly as today.
```

**Key decision — config-driven, not a new node kind.** 009 item 6 was right: the
`route` node routes control flow, not models. The agent node already reads an
optional model; `route_step` is just a smarter way to fill that field. The
`RoutePolicy` is data (config), not code.

**Key decision — the bus is the audit trail.** Each `RouteDecision` is broadcast
on the live bus (F12), so subscribers see which model each step used. This is the
"explicit and auditable" requirement from 008 §3.2, delivered the same way the
agent node's other live events (`ToolCall`/`ToolResult`/`AssistantDelta`) are.

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R1 | `routing_config_parses` | TOML with `[evolve.routing] patch = "..."` | `Config::parse` accepts it; `RoutePolicy` has the entry |
| R1 | `routing_config_denies_unknown` | TOML with `[evolve.routing] bogus = "..."` | `Config::parse` errors (deny_unknown_fields) |
| R2 | `route_step_resolves_model` | agent node with `route_step = "patch"` + policy mapping patch→modelX | node's resolved model == modelX |
| R2 | `no_route_step_uses_explicit_model` | agent node with `model = "m"` and no `route_step` | node's model == "m" (unchanged behavior) |
| R3 | `route_decision_is_journaled` | run a routed agent node with a bus subscriber | bus carries `RouteDecision { step, model }` (assert the event, not just "journal non-empty") |
| R4 | `unassigned_step_fails_loud` | agent node with `route_step = "patch"` but policy has no `patch` | node fails with `NotFound` (NOT a silent default) |

**R4 is the acceptance test.** A plausible implementation silently defaults an
unassigned step to the provider default — but 008 §1 says a single misroute
cascades, so it must be loud. The test MUST assert the node fails with `NotFound`.

---

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-server routing_config_ 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -p pii-server route_step_ 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -p pii-server route_decision_ 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -p pii-server unassigned_step_ 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test --workspace` — 0 failures (N3)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N3)
- [ ] `cargo fmt --all --check` — clean (N3)
- [ ] `cargo xtask silent-noop-check` — green (N4)

---

## Guardrails

- **G1 — do NOT edit this spec.**
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — NO new node kind.** Model routing lives in the agent/config layer, not a
  new `route`-like node.
- **G5 — do NOT touch `specs/`, `xtask/`, `quality-gate.sh`, `.github/`, or a
  `#[test]` assertion.**
- **G6 — no hardcoded absolute paths.** Test artefacts under `_tmp/`.
- **G7 — report raw output, never summed totals.**

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/pii-server/src/daemon/config.rs` | R1: add `EvolveSection` with `routing` sub-table |
| `crates/pii-server/src/agent.rs` | R2/R3/R4: `route_step` key, resolve via policy, journal decision, loud NotFound |
| `crates/pii-proto/src/event.rs` | R3: add `Event::RouteDecision` variant (if not present) |
| `crates/pii-server/src/registry.rs` | thread the `RoutePolicy` into `AgentFactory` |
| `crates/pii-server/tests/` or `src/agent.rs` tests | R1-R4 tests |

**Not modified**: `pii-graph` (no new node kind), `pii-evolve` (routing is
config-driven, not a crate module), `pii-cli`, `pii-mcp`, `pii-web`.

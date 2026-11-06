# Spec: the seven single-use kinds are derived from the ones that remain

## Overview
Spec 383 step 3, the derivation Opus 046 proved on paper. Seven kinds are each used by one graph:
`worker` (2 graphs), `classify` (1), `plan` (1), `memory` (3), `select`/`record`/`enqueue` (the ledger,
step 2), `heal_detect`/`heal_record` (2). A kind that exists for one graph is a graph written in Rust.

**Project**: /projects/pi-improved
**Phase**: 383 step 3 of 4
**Depends on**: 391 (the ledger) — the store kinds move there first

## Requirements
### Functional
- R1 `worker` → `coder`: a worker node is a coder with a different brief. The graphs that use it
  (`code-index`, `intervention-report`) call `coder` with the same fields.
- R2 `classify` → `route` widened: a classifier is a route over a computed predicate. 383 step 1 gave
  `transform` comparisons, so the predicate is expressible in the graph.
- R3 `plan` → `agent`: a planner is a model call. It is an `agent` node with a prompt template (386).
- R4 `memory` → `remember`/`recall`: the two directions already exist; `memory` is the write, so it
  becomes `remember`.
- R5 `debug_crash` → a test fixture: the only graph using it exists to panic a node. It moves out of
  `graphs/` into `crates/pii-graph/tests/fixtures/`.
- R6 `heal_detect`/`heal_record` → `ledger` with `op = "heal.record_run"` plus a `transform` over the
  observation; detection is a comparison, not a kind.

### Non-Functional
- R7 after this row, `graphs/*.toml` uses no kind that is used by exactly one graph. The count is
  measured by parsing `[[node]]` tables, never by grep.

## TDD Contract
| Test name | Input | Expected |
|---|---|---|
| `every_kind_is_used_by_more_than_one_graph` | the 32 graphs | no kind with a single user |
| `the_two_worker_graphs_load_as_coder` | `code-index`, `intervention-report` | lint ok, coder kinds |
| `CONTROL_the_detector_is_a_transform` | heal graph | a transform computes the predicate |

## Exit Criteria
- [ ] `test "$(grep -rl 'kind = "classify"' graphs/ | wc -l)" = 0`
- [ ] `test "$(grep -rl 'kind = "plan"' graphs/ | wc -l)" = 0`
- [ ] `test "$(grep -rl 'kind = "memory"' graphs/ | wc -l)" = 0`

Why this section has fewer criteria than the requirements imply: `sdd`'s contract step runs every
criterion BEFORE any code exists and refuses a row if one already passes (measured 2026-11-05: eight
dispatches refused, none spent). A criterion that passes today is a regression guard, not a test of the
new behaviour, so the guards are stated under Guardrails and enforced by the gate.

## Guardrails
- Must stay green, and are enforced by the gate rather than by a criterion (they pass today):
  no graph uses `kind = "worker"` or `kind = "debug_crash"` (R1 and R5 landed in the working tree; the same
  `grep -rl` test, expecting 0),
  `cargo test -p pii-cli --test every_graph_loads`, `pii graph lint graphs/self-improve.toml`,
  `pii graph lint graphs/intervention-report.toml`.
- Do NOT delete a graph to make its kind disappear: the flow is what the operator runs.
- Do NOT move a flow into Rust to avoid touching the graph; 383's premise is that everything is a graph.
- Do NOT let `remember` become unguarded: it already requires `MEMORY_WRITE`, and that must hold.

## Files
- `crates/pii-server/src/node_kinds/nodes/mod.rs`
- `crates/pii-server/src/node_kinds/nodes/classify.rs`
- `crates/pii-server/src/node_kinds/nodes/plan.rs`
- `crates/pii-server/src/node_kinds/nodes/memory.rs`
- `crates/pii-server/src/node_kinds/nodes/worker.rs`
- `crates/pii-server/src/lib.rs`
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-server/tests/live_registry_refuses_a_drifted_skill.rs`
- `crates/pii-server/tests/live_registry_supplies_the_skill_source.rs`
- `crates/pii-server/tests/model_routing.rs`
- `crates/pii-server/tests/skill_edit_through_the_daemons_store.rs`
- `crates/pii-server/tests/skill_put_through_the_daemon.rs`
- `crates/pii-cli/tests/every_graph_loads.rs`
- `crates/pii-graph/tests/fixtures/crash.toml`
- `graphs/code-index.toml`
- `graphs/intervention-report.toml`
- `graphs/starter/crash.toml`
- `graphs/self-improve.toml`
- `graphs/digest-now.toml`
- `graphs/discord-triage.toml`

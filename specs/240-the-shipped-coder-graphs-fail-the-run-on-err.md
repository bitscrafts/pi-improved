# Spec: 240 — the shipped coder graphs end a failed agent in a node that fails the run

**Project**: <repo>
**Phase**: filed 2026-10-23T10:30Z by the orchestrator (defect 22 in `_docs/05-operations/agent-management-findings-2026-10-23.md`).
Premises read, not assumed:

- Four shipped graphs route the coder's `err` port to a `transform` named `refused` with no outgoing edge:
  `graphs/claude-probe.toml`, `graphs/coder-selffix.toml`, `graphs/coder-demo.toml`, `graphs/coder-podman.toml`
  (each has `[[node]] id = "refused" kind = "transform" value = "refused"` and an edge `from = "code" port = "err" to = "refused"`).
- A `refuse` node kind exists (`crates/pii-graph/src/nodes.rs` `RefuseNode`, registered in `crates/pii-graph/src/node.rs`
  as `"refuse"`): it declares NO ports and returns `Outcome::Fail { error: "run.refused{hook=<hook>, reason=<reason>}" }`,
  reading cfg keys `hook` and `reason` (both default to placeholders). `graphs/fragments/podman-guard.toml` already uses it.
- Lint V11 (spec 238, `crates/pii-graph/src/validate.rs`) warns on exactly this shape, so the four graphs each carry one
  V11 warning today.
- Measured: `r000000000641` and `r000000000706` exited rc 1 and finished `succeeded` because of this wiring.

## Overview

The coder is correct: a non-zero exit goes to `err`. The graphs then end that path in a node that completes normally, so a
failed agent finishes the run green. The fix is graph data, not engine behaviour: end `err` in a `refuse` node.

## Requirements

- **R1** In each of the four graphs, the node `refused` changes from `kind = "transform"` to `kind = "refuse"` with cfg
  `hook = "coder"` and `reason = "the agent failed"` (replacing `value = "refused"`). The node id, the edge and every other
  node stay as they are.
- **R2** After the change `pii graph lint` on each of the four graphs prints NO V11 warning.
- **R3** The `ok` path is unchanged: `done` stays a `transform`, and a run whose agent succeeds still finishes `succeeded`.
- **R4** No other graph in `graphs/` is edited. `graphs/queue.toml`, the digest graphs and `graphs/fragments/` are out of scope.

## Architecture

Four small TOML edits plus one test that reads the shipped files. No Rust change outside the test. Rejected: making the
engine fail a run that reached a terminal through an `err` port (it would change every graph, including ones where `err`
legitimately ends a run in success, such as a digest that records a failure and carries on).

## TDD Contract

New test file `crates/pii-graph/tests/shipped_coder_graphs_fail_on_err.rs`, reading the four files from
`CARGO_MANIFEST_DIR/../../graphs/` (follow how `coder_err_edge_lint.rs::the_shipped_probe_graph_warns` locates and parses
`graphs/claude-probe.toml`; if that test now asserts the probe graph WARNS, update its assertion to the new truth — see
Guardrails).

| test | given | expects |
|---|---|---|
| `each_shipped_coder_graph_ends_err_in_a_refuse_node` | the four graphs | the node the `code` node's `err` edge reaches has kind `refuse` |
| `no_shipped_coder_graph_carries_the_v11_warning` | lint each of the four | no warning contains `V11` |
| `the_ok_path_still_ends_in_a_transform` | the four graphs | the node the `ok` edge reaches has kind `transform` (CONTROL) |
| `a_graph_that_still_ends_err_in_a_transform_still_warns` | an inline graph built the old way | exactly one `V11` warning (CONTROL: the lint still fires) |
| `a_failed_agent_fails_the_run` | run `graphs/claude-probe.toml`-shaped graph in-process with a coder that returns the `err` port (use a stub node of kind that emits `err`, as neighbouring graph tests do) | the run's final status is failed, error contains `run.refused` |

## Exit Criteria

- [ ] `cargo test -q -p pii-graph --test shipped_coder_graphs_fail_on_err 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-graph --test coder_err_edge_lint 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `! printf '%s\n' claude-probe coder-selffix coder-demo coder-podman | xargs -I{} cargo run -q -p pii-cli --bin pii -- graph lint graphs/{}.toml 2>&1 | grep -q V11`
- [ ] `! grep -c 'kind = "refuse"' graphs/claude-probe.toml graphs/coder-selffix.toml graphs/coder-demo.toml graphs/coder-podman.toml | grep -q ':0$'`
- [ ] `cargo clippy -q -p pii-graph --all-targets -- -D warnings`

## Guardrails

- `specs/` are READ-ONLY for the implementer. If a graph is not shaped as described (no `refused` node, a different edge
  layout), STOP and report which graph and what it contains instead of editing around it.
- `coder_err_edge_lint::the_shipped_probe_graph_warns` asserts the probe graph WARNS. After R1 that assertion is false by
  design. Change ONLY that one test, to read an inline graph built the old way (the shape 238 lints), and say so in the
  Closure. Do not weaken any other assertion and do not touch the lint itself.
- Do not edit `graphs/queue.toml`, `graphs/fragments/*`, the digest graphs, or any engine file.
- Do not add a node kind. `refuse` exists; use it.
- Tests write under `_tmp/`. Do NOT commit; the orchestrator commits.

## Closure

_(empty — the implementer's readings go here)_

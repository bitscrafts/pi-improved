# Spec: the vocabulary has a fence, and tool's surface is closed

## Overview
Spec 383 step 4, deliberately last. A lint written before the library is adapted refuses its own
repository, an expected refusal gets silenced, and a silenced check is how the next defect ships. Steps
2 and 3 shrink the vocabulary; this row keeps it small.

**Project**: /projects/pi-improved
**Phase**: 383 step 4 of 4
**Depends on**: 391 (the ledger), 392 (the derivations)

## Requirements
### Functional
- R1 `xtask lint-graph-kinds` parses `[[node]]` tables with a TOML parser — never a grep — and refuses
  three things: a kind outside the declared vocabulary; a kind used by exactly one graph
  (`specialised`); a graph that re-implements a flow another shipped graph provides (measured by call
  targets and node kinds, reported as `duplicate-of <graph>`).
- R2 the declared vocabulary lives in one file (`docs/03-reference/graph-vocabulary.md` is prose; the
  list itself is `graphs/VOCABULARY` — one name per line, with its capability and effect class).
- R3 `tool`'s surface is CLOSED: `fs_read`, `glob`, `grep`, `fs_write`, `run` with a literal argv0 and
  no shell. A `tool` node whose `run` is a shell string is refused at load, naming the node.
- R4 the lint runs in the gate (`xtask fresh-gate`), so the vocabulary cannot grow silently.

### Non-Functional
- R5 the lint's own refusals are tested by three controls: a legal graph passes; a single-user kind is
  reported; a duplicate flow is reported.
- R6 the lint reports the count it measured, so a passing run is evidence (`23 kinds over 32 graphs,
  0 specialised`).

## TDD Contract
| Test name | Input | Expected |
|---|---|---|
| `a_kind_outside_the_vocabulary_is_refused` | a graph with `kind = "sel ect"` | refuse, names the kind |
| `a_single_user_kind_is_specialised` | a fixture graph with a unique kind | report `specialised` |
| `a_duplicate_flow_is_named` | a fixture copying `queue`'s shape | report `duplicate-of queue` |
| `CONTROL_every_shipped_graph_passes` | `graphs/*.toml` | pass, with the measured count |
| `a_shell_string_in_tool_is_refused` | `tool` with `run = "bash -c ..."` | refuse at load, names the node |

## Exit Criteria
- [ ] `cargo run -q -p xtask -- lint-graph-kinds 2>&1 | grep -q '0 specialised'`
- [ ] `cargo test -q -p xtask --test lint_graph_kinds 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `test -f graphs/VOCABULARY`
- [ ] `grep -q 'lint-graph-kinds' xtask/src/fresh_gate.rs`
- [ ] `cargo test -q -p pii-graph --test tool_surface_is_closed 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`

## Guardrails
- Do NOT implement the lint by grepping the files: a `kind =` grep counts EDGE kinds too, and that
  mistake already produced a wrong analysis that misdirected a whole session.
- Do NOT add an exception list to pass this row. If a graph fails, the graph moves (392) — the fence
  is not negotiated with.
- Do NOT run a shell string from `tool`: authority comes from the committed spec, not from the graph.

## Files
- `xtask/src/lint_graph_kinds.rs` (new), `xtask/src/main.rs`, `xtask/src/fresh_gate.rs`
- `xtask/tests/lint_graph_kinds.rs` (new)
- `graphs/VOCABULARY` (new)
- `crates/pii-graph/src/nodes.rs` (tool surface), `crates/pii-graph/tests/tool_surface_is_closed.rs` (new)
- `docs/03-reference/graph-vocabulary.md`

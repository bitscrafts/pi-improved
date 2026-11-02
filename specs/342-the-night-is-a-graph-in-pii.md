# Spec: The night is a graph composed from the graphs pii already has

## Overview

Composability is pii's headline feature: a graph is a puzzle of pieces that already exist, and a graph includes
another graph with `kind = "call"` (spec 255) — pinned by sha256, caps checked at load, cycles refused with their
chain. This spec is the night built that way: a short host that calls `queue` by pin, routes its exhaustion to a
review, and posts the result.

It is deliberately **not** a new mechanism, and it is not the `night_pick`/`night_land` sketch an abandoned attempt
left in the tree: those duplicated `select` and `record`, one ran `bash -c` from a payload (criteria are data,
spec 215), and one **made git commits**, which breaks the rule that only the orchestrator commits.

**Project**: /projects/pi-improved
**Phase**: composability — depends on 343 for the four gaps that make `call` usable from a graph.

## Verified premises (read-only review of HEAD `79e4262`, 2026-11-02)

- `call` exists and is used in production (`graphs/spec.toml:44-52`), 14 files call a graph by pin.
- `graphs/queue.toml` **cannot be called today**: it declares no `exit` (`graph.call.exit`, `call.rs:221-229`), it
  carries absolute paths (`:24,33`), and its kinds read the blackboard by fixed ids (`queue.rs:26,237`,
  `coder/helpers.rs:127`) which namespacing turns into `None`.
- `graphs/spec.toml` has no `exit` either; and its `refuse` nodes end the **whole** run as `Refused` rather than
  routing to an exit port — a called graph that refuses would end the night.
- The expanded spec is stored content-addressed at run start and resumed from the store, so a night suspended
  across an edit of `queue.toml` keeps running the pinned copy.
- `max_entries` defaults to 8 (`call.rs:36`) and survives a resume (`state.rs:237`): a night that re-enters the
  call on every tick dies on the 9th row unless it is set to at least the loop budget.

## Requirements

### Functional

1. `graphs/night.toml` is a **short host**: a `timer` entry, a `call` to `queue` by pin with `max_entries` at least
   the loop budget, a branch for an exhausted queue into a review, and `post` for the report. No new node kinds.
2. **Every kind in the night exists in a graph it composes** — the guardrail is a check, not a sentence:
   the night's kind set is a subset of the union of the kinds used by `queue`, `spec` and the digest.
3. **No absolute project paths** in the graph: paths arrive through `input` (`workdir = "$.input.workdir"`).
4. `queue.toml` becomes callable: `exit = "record"`, a `needs` list, paths from input, and its peer lookups as
   expressions (spec 343 R5). `spec.toml` gains `exit` and `needs`, and its refusals leave on an exit port rather
   than ending the host run.
5. The night stops on exhaustion: when the queue has no open row, the run reaches `review` and posts, rather than
   failing inside `record`.

### Non-Functional

- The graph carries no duplicate of an existing kind; a reviewer must be able to see the composition in the file.
- ≤450 effective lines per file; the graph is data, so this binds the checks, not the TOML.

## Architecture

```
night.toml
  clock (timer, cron */5)
        │
        ▼
  sdd:  kind = "call"  graph = "queue"  sha256 = <pin>  max_entries = <loop budget>
        in:  row = "$.input.row"   workdir = "$.input.workdir"
        exit: record
        │ ok                      │ none (queue exhausted)
        ▼                         ▼
      loop                      review ──► post (channel, tag)
```

## TDD Contract

| Test (`night_graph_composes.rs`) | Assertion |
|---|---|
| `night_calls_queue_at_its_current_pin` | the `CallRecord` for `sdd` has `graph == "queue"` and `sha256 == sha256(graphs/queue.toml)` |
| `night_graph_uses_only_existing_kinds` | every `kind` in the night appears in a composed graph |
| `night_graph_kinds_are_a_subset_of_the_graphs_it_composes` | the subset relation holds, printed on failure |
| `night_graph_carries_no_absolute_project_paths` | no `/projects` or `/root` outside a comment |
| `night_stops_on_exhaustion` | an exhausted queue reaches `review`, not `sdd__dispatch` |
| `night_call_sets_max_entries_at_least_its_loop_budget` | the call's `max_entries` ≥ the loop count it declares |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test night_graph_composes`
- [ ] `pii graph lint graphs/night.toml`
- [ ] `pii graph lint graphs/queue.toml`
- [ ] `cargo run -q -p xtask -- fresh-gate`

## Guardrails

- Do NOT keep `night_pick` or `night_land` in any branch: they duplicate `select` and `record`.
- Do NOT run a shell from a node payload (`bash -c`): criteria are data (spec 215).
- Do NOT commit from a node: only the orchestrator commits.
- Do NOT copy `queue.toml`'s node set into the night — call it.
- Do NOT let one bad row end the night: `spec.toml`'s refusals must leave on an exit port.

## Files

- `graphs/night.toml`
- `graphs/queue.toml`
- `graphs/spec.toml`
- `crates/pii-server/tests/night_graph_composes.rs`
- `specs/checks/night-graph.sh`

# Spec: A graph includes a graph — by extending `call`, not by adding a kind

## Overview

Composition already ships. **Spec 255's `kind = "call"`** inlines another graph at load time: it carries a sha256
pin, three capability layers, a cycle chain and a depth cap, and `graphs/spec.toml:44-52` uses it in production.
Fourteen files in this tree already call a graph by pin.

This spec is therefore **not** "build inclusion". It closes the four gaps that make `call` hard to use from
another graph — output naming, failure attribution, a suspension test that has never existed, and the fixed-id
reads that break under namespacing — and it records the decision so nobody builds a second include mechanism.

**Project**: /projects/pi-improved
**Phase**: composability — the night (342) depends on this.

## Verified premises (from a read-only review of HEAD `79e4262`, 2026-11-02)

- `crates/pii-graph/src/fragment/call.rs:1-17` — "a `call` node inlines another graph at load time"; pin at
  `:201-206`, caps at `:150-156` and `:242-256`, cycle chain at `:157-162`, depth cap 4 at `:33`.
- `graphs/spec.toml:44-52` calls `docs` by pin; `grep -rln 'kind *= *"call"' graphs crates` → 14 files.
- The expanded spec is stored content-addressed at run start (`run/mod.rs:111-112`) and resume deserialises that
  blob (`engine/runs.rs:419-424`), so editing a called graph cannot change a suspended run.
- **Nothing tests a suspension inside a call** — the only call tests mentioning gates use a counting gate.
- `queue` kinds read the blackboard by fixed ids (`queue.rs:26`, `queue.rs:237`, `coder/helpers.rs:127`), so a
  namespaced `sdd__select` resolves to `None` once `queue` is called.

## Requirements

### Functional

1. **No new node kind.** Inclusion stays `kind = "call"`: load-time inline, one run, one journal, namespaced ids
   (`<call>__<node>`). An alias or a `graph` kind is refused by review, not by code.
2. **Output naming.** A host may read the called graph's terminal values by the call's own id:
   `$.nodes.<call>.<key>` resolves to `$.nodes.<call>__<exit>.<key>`. Rewrite happens at load, in a new
   `crates/pii-graph/src/fragment/call_output.rs` (`alias_outputs(doc, call, exit)`, ~60 lines to keep `call.rs`
   under the 450-line gate), called from `splice` after the host edge rewrite (`call.rs:346-358`). A host that
   never reads `$.nodes.<call>` must load byte-identically.
3. **Failure attribution.** A run that fails inside a called graph names the namespaced node (`sdd__judge`) **and**
   the call: the cycle refusal gains the call id — `graph.call.cycle: a -> b -> a (call 'back' in graph 'b')` —
   and the `graph.call` journal event carries graph, sha256 and caps (`fragment/call_record.rs:12-29`).
4. **Suspension inside a call is the run's own suspension**, keyed to the namespaced node (`sub__tick`). A finished
   call is not re-entered on resume; a suspended one resumes at the suspended node. `max_entries` survives a
   resume (`sched/state.rs:237`) and must be set to at least the loop budget.
5. **Called graphs read their peers by expression, not by fixed id.** `queue`'s `select`/`judge`/`dispatch` lookups
   become config expressions defaulting to today's fixed ids when absent, so a standalone `queue.toml` behaves
   exactly as it does now (the control) while a namespaced call works.
6. **A called graph must be callable at all**: `exit` declared, `needs` declared, paths taken from `input` rather
   than absolute. `queue.toml` and `spec.toml` gain these, which changes their pins.

### Non-Functional

- Refusals stay refusals: caps at load (`graph.call.widen`, `graph.grant.ceiling`), cycles at load. No silent
  intersection, no run-time escalation.
- No line numbers in load errors (the loader works on `toml::Table`, which carries no spans); the call id is
  unique per file and is enough to find the line.
- ≤450 effective lines per file.

## Architecture

```
night.toml ──► load_host_with_graphs(src, fragments, graph_dirs, reg)          [fragment/weave.rs]
                 │  expand_host: find graph file → sha256 == pin? → cycle? → depth ≤ 4
                 │               caps ⊆ host, node caps ⊆ call → splice namespaced nodes
                 │               NEW: alias_outputs  $.nodes.sdd.x → $.nodes.sdd__<exit>.x
                 ▼
              ONE GraphSpec → stored at run start (content-addressed)
              run: one journal · graph.call event on sdd__in · suspend inside the call suspends the RUN
              resume: GraphSpec::from_json(store) — never re-reads graphs/*.toml
```

## TDD Contract

| Test (file) | Assertion |
|---|---|
| `child_outputs_appear_under_the_call_id` (`call_outputs_and_cycles.rs`) | running the host, `$.nodes.sdd.verdict` is `"approved"`; fails if `call_output.rs` is reverted |
| `CONTROL_the_exit_id_still_resolves` (same) | `$.nodes.sdd__judge.verdict` still resolves |
| `a_host_without_the_alias_is_byte_identical` (same) | a host that never reads `$.nodes.<call>` loads to an equal spec |
| `a_cycle_names_the_call_that_closes_it` (same) | error contains `a -> b -> a` and `call 'back'` |
| `a_timer_inside_a_call_suspends_the_host_run` (`call_suspends_and_resumes.rs`) | status Suspended, key's node is `sub__tick` |
| `CONTROL_the_subgraph_alone_suspends_at_tick` (same) | the child alone suspends at `tick` |
| `resuming_the_host_continues_inside_the_call_at_the_suspended_node` (same) | `sub__after` then `done`; a counting node before the call ran exactly once |
| `a_call_that_finished_before_the_suspend_is_not_rerun` (same) | counting node inside the call has count 1 after resume |
| `a_failure_inside_a_call_names_the_call_and_the_node` (same) | run fails, terminal names `sub__boom`; journal `graph.call` names graph + sha |
| `CONTROL_a_host_failure_names_the_host_node` (same) | a host-side failure names the host node |
| `the_entry_count_survives_a_resume` (same) | 8th entry passes, 9th fails, across a resume |
| `queue_kinds_read_their_peers_inside_a_call` (`queue_in_a_call.rs`) | item ends Done/Failed with a verdict, never "no item was selected" |
| `CONTROL_queue_toml_alone_records_the_same_verdict` (same) | standalone `queue.toml` unchanged |
| `a_host_select_node_is_not_read_by_the_called_record` (same) | the called `record` writes the called item |

## Exit Criteria

- [ ] `cargo test -q -p pii-graph --test call_outputs_and_cycles`
- [ ] `cargo test -q -p pii-graph --test call_suspends_and_resumes`
- [ ] `cargo test -q -p pii-server --test queue_in_a_call`
- [ ] `cargo test -q -p pii-graph --test call_inlines_a_graph`
- [ ] `cargo run -q -p xtask -- fresh-gate`

## Guardrails

- Do NOT add a `graph` kind, or make `graph` an alias of `call` — two spellings of one meaning.
- Do NOT introduce a child run per include: it buys only a run-time graph name nobody needs, and costs a resume,
  cancel, budget and journal subsystem.
- Do NOT filter outputs (`output = [...]`): the host can still read `$.payload`, so a filter promises isolation it
  cannot enforce. The alias gives the ergonomics without the false promise.
- Do NOT re-read `graphs/*.toml` on resume "for safety": the pinned spec is already in the store, and re-reading
  makes a resume non-deterministic.
- Do NOT intersect caps silently.
- Do NOT test through a bare helper: a checker nothing calls can be green while nothing composes (measured — an
  abandoned attempt had 7 passing tests for a module the loader never invoked).

## Files

- `crates/pii-graph/src/fragment/call_output.rs` (new)
- `crates/pii-graph/src/fragment/call.rs`
- `crates/pii-graph/tests/call_outputs_and_cycles.rs` (new)
- `crates/pii-graph/tests/call_suspends_and_resumes.rs` (new)
- `crates/pii-server/src/node_kinds/nodes/queue.rs`
- `crates/pii-server/src/node_kinds/nodes/coder/helpers.rs`
- `crates/pii-server/tests/queue_in_a_call.rs` (new)
- `graphs/queue.toml`
- `graphs/spec.toml`

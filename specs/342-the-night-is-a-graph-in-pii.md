# Spec: The night is a graph composed from the graphs pii already has

## Overview

pii's composability is one of its most important features: a graph is a puzzle of pieces that already exist, and a
capability that is missing is expressed as another graph, not as another node. The operator's picture:

> *"one graph that does tdd, one graph that does sdd, and other graphs composing them!"*

The night of 2026-11-02 was nine shell scripts in a Hermes skill. The first attempt at moving it into pii proposed two
new node kinds (`night_pick`, `night_land`). Both duplicated what exists:

```
graphs/queue.toml   select → coder → judge → record     the SDD loop already runs a row end to end
graphs/spec.toml    coder → judge → call → judge → refuse   authoring a spec, with its own judge
graphs/discord-digest.toml   recall → post              the report egress that already works
node kinds          timer · select · coder · judge · record · gate · worker · agent_route · post · sop
```

So this spec adds **no node kind**. The night is those pieces, in that order, with pii's `[[cron]]` for the tick.

**Project**: /projects/pi-improved
**Phase**: after the 2026-11-02 night (rows 335–340); supersedes the withdrawn "port the shell into the binary" idea

## Requirements

### Functional

- `graphs/night.toml`, `entry = "clock"`, one node per existing piece and **no new kind**:
  - `clock` — `timer`, fired by a `[[cron]]` entry; a night is many firings, not one long process.
  - `pick` — `select` (from `graphs/queue.toml`): the next open row, or `none`.
  - `work` — `coder` (spec 207): dispatches the row's agent; the worker's handle is recorded so a resume has one.
  - `judge` — `judge`: runs exactly the row's criteria argv, never a shell string.
  - `record` — `record`: `done` with the commit, or the failure with its evidence.
  - `spec` — when a row needs a spec that does not exist, the `spec.toml` sequence runs as a sub-sequence of nodes
    (author → judge → refuse), not as an inline prompt.
  - `review` — only when `pick` yields `none`: `agent_route` picks the frontier model and the existing `worker` runs the
    review; the report is posted by the same `post` node `graphs/discord-digest.toml` uses.
- Stop conditions stay configuration, the three the operator named (*"a time constraint … or a loop count like
  (opus -> drain, opus -> drain) 2x … or finish when the intervention ends"*): `[night] until`, `[night] rounds`,
  exhaustion.
- A row needing a human decision uses the existing `gate` node (store-backed suspension), so a restart resumes it.
- Reuse is by **name and pin**, not by copy: any fragment the composed graphs rely on (e.g. `graphs/fragments/…`) is
  attached the way fragments already are — by name with a sha256.

### Non-Functional

- **The graph's kind set must be a subset of the kinds used by the graphs it composes.** If the night needs a piece
  that does not exist, that piece is its own spec — never a `night_*` node.
- The graph carries no absolute project paths in its body: paths come from the run's input, so the same graph serves a
  second project.
- It lints (`pii graph lint`), declares `[caps]`, and resumes after a daemon restart from its store.

## What does NOT exist yet, and is not invented here

Graph-includes-graph — one graph naming another by name + pin, the way fragments are attached — is **not** supported
today. So "the night graph composes `queue.toml`" is true at the level of **kinds, fragments and sequences**, not by
referencing the file. Making a graph reference another graph is composability's own next step; it is a spec of its own,
and this night graph is the first customer that would justify it.

## Architecture

```
[[cron]] every 5 min ──> clock(timer)
   clock ─> pick(select) ─┬─ row ─> work(coder) ─> judge ─> record ─┐
                          │                                        │
                          └─ none ─> review(agent_route + worker) ─> post ─> suspended on the timer
                                                                     (next firing resumes at pick)
```

## Files

- `specs/342-the-night-is-a-graph-in-pii.md`
- `graphs/night.toml`
- `crates/pii-server/tests/night_graph_composes.rs`

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| night_graph_uses_only_existing_kinds | `graphs/night.toml` built through the real registry | every kind resolves, and none of them is named `night_*` |
| night_graph_kinds_are_a_subset_of_the_graphs_it_composes | the kinds in night vs union(queue, spec, discord-digest) | subset, listed by name |
| night_graph_carries_no_absolute_project_paths | the graph's text | no `/projects/` and no `/home/` outside comments |
| night_stops_on_exhaustion | a state file with every row done | `pick` yields `none`, the run reaches `review`, not `work` |

## Exit Criteria

- [ ] `pii graph lint graphs/night.toml`
- [ ] `cargo test -q -p pii-server --test night_graph_composes`
- [ ] `pii run start night --input '{"dry_run":true}'`

## Guardrails

- Do NOT add a node kind for the night, and do NOT keep `night_pick`/`night_land` in any branch: they are the exact
  duplication this spec exists to prevent.
- Do NOT copy node blocks out of `queue.toml` and then change them: reuse the kinds, keep one meaning.
- Do NOT put `/projects/...` paths in the graph body.
- Do NOT delete the shell reference until the composed graph passes the criteria those rows already satisfy.

# Spec: The night is a graph in pii, not a cron of shell scripts

## Overview

pii has a graph engine: nodes, `[[cron]]` entries, runs on disk, suspension, resume, capability scopes. The night of
2026-11-02 was driven by nine shell scripts and four Python modules in a Hermes skill instead — because that is how a
driver gets built quickly, not how a product keeps one. This spec puts the night where pii's own machinery already
lives: a graph with a timer, a pick node, a worker node, a gate node, a landing node, a review node and a post node,
each of them a thing the daemon knows how to run and resume.

The Hermes skill (`night-loop`) keeps the *method* — which primitives carry a night, and the traps that cost money.
It stops carrying pii's queue.

**Project**: /projects/pi-improved
**Phase**: after the 2026-11-02 night (rows 335–340) and the withdrawal of the port-to-binary idea

## Requirements

### Functional

- `graphs/night.toml` — one graph, `entry = "clock"`:
  - `clock`: a `timer` node, `cron = "*/5 * * * *"`, so a firing is a tick and a night is many ticks, not one long
    process.
  - `pick`: reads the queue (`scripts/autonomy/plan.tsv` + `_tmp/autonomy/state.json`) and emits the next row, or
    `none`.
  - `work`: a `worker` node that dispatches the row's coding agent, with the row's spec and criteria as its brief.
  - `verify`: runs the row's criteria (a `bash` node), failing the run with the criterion's own output as evidence.
  - `land`: commits the row's declared paths with a neutral identity and records the commit in the queue state.
  - `review`: when the queue has no open row, a `worker` node runs the frontier review and files it under
    `_docs/06-research/`.
  - `post`: a `channel` node posts the night's report to the Discord thread, tagged `status`.
- Every node's failure leaves evidence a person can read: the node's output, its input, and the run id.
- A night stops on the clock (`[night] until`), a count (`[night] rounds`), or exhaustion — the same three conditions
  the operator asked for, now as graph configuration.
- The queue's meaning stays pii's: rows, criteria, paths and commits are read and written by pii code, not by shell.

### Non-Functional

- Nothing in the graph reaches into `~/.hermes`; the graph is data pii's daemon loads and runs.
- A night must survive a daemon restart: the run resumes from its store, not from a shell variable.
- Capability scopes: the graph declares `[caps]` for what it may do (fs write under the repo, git, channel post).
- No behaviour may depend on a model being reachable except `work` and `review`.

## Architecture

```
graphs/night.toml
  clock(timer) -> pick(fs+state) -> work(worker) -> verify(bash) -> land(bash+state)
                                                                   |
                                     queue still open? ------------+-- no --> review(worker) -> post(channel)
                                     yes: next tick resumes at pick, run stays "suspended" on the timer
```

## Files

- `specs/342-the-night-is-a-graph-in-pii.md`
- `graphs/night.toml`
- `crates/pii-graph/src/nodes/night_pick.rs`
- `crates/pii-graph/src/nodes/night_land.rs`
- `crates/pii-cli/src/cmd_night.rs`

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| night_pick_returns_none_when_queue_is_empty | plan with every row done | `pick` emits `none`, no error |
| night_pick_returns_the_first_open_row | one pending, one done | the pending row's id and spec |
| night_land_refuses_a_dirty_path_it_does_not_declare | an untracked file outside the row | failure naming the path, nothing committed |
| night_land_records_the_commit | a row whose criteria pass | state row `done` with that commit |
| night_timer_resumes_the_run | a run suspended on the clock | the next firing continues at `pick`, same run id |

## Exit Criteria

- [ ] `cargo test -q -p pii-graph --test night_nodes`
- [ ] `pii graph lint graphs/night.toml`
- [ ] `pii run start night --input '{"dry_run":true}'`

## Guardrails

- Do NOT put shell logic in the graph's `bash` nodes beyond the gate command: the queue's meaning belongs in pii code.
- Do NOT delete the Hermes skill: it stays the generic method for other projects, and its traps are the documentation
  for why this graph looks the way it does.
- Do NOT let a node hold the run open across a paid call without recording the worker's handle.

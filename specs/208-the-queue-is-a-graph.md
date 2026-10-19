# Spec: the queue is a graph

## Overview
User directive 2026-10-19: "in the future you will need to implement the queue management as a graph!" Today the development queue is `scripts/autonomy/plan.tsv` + `_tmp/autonomy/state.json` + `cycle.sh` + a manual `control` file, driven by a Hermes cron. Every one of this week's queue defects came from that shape being a set of files a shell re-reads and re-interprets: three dispatchers colliding on a lock, `OUTCOME=locked` indistinguishable from work, a pause nobody resumes, duplicate spec numbers, parked rows skipped silently, and "queue empty" printed while three rows waited. pii already has the substrate that removes the whole class: a store, runs with statuses, a journal, gates, budgets, memory, and a scheduler. The queue must become one of its graphs.

**Project**: /projects/pi-improved

## Verified premises
- `pick` is a Python function over a TSV (`scripts/autonomy/state.py`); row identity is the row id, status lives in a second file, and the lock is a flock on a third.
- Cycle outcomes already map onto pii run statuses: done → succeeded · needs-review/parked → failed with a reason · requeued-quota → suspended.
- 074/207 give the dispatch half (a `coder` node that spawns an agent, criteria deciding the verdict, refusal as a port).

## Requirements
### Functional
1. A **work item** is a first-class stored object: id, spec path, title, criteria (verbatim), touched paths, `needs` (list of ids), status, attempts, worker session, last reason. No TSV, no second state file.
2. Queue management is a GRAPH — `graphs/queue.toml` — whose nodes are: `select` (pick the first item whose `needs` are done) → `dispatch` (a `coder` node) → `judge` (run the item's criteria; non-zero fails the node) → `record` (memory put, and the item's status update) → `heal` (a port on failure, never a silent retry) → `commit` is an operator act, not a node.
3. `select` is a GATE, not a hope: an item whose `needs` are unmet is not dispatched, and the run's output names what it is waiting for. Blocked ≠ empty, and the two must print differently.
4. The schedule is pii's own `[[cron]]` + a `timer` node — not a Hermes cron polling a shell script.
5. A run of an item is a pii RUN: it is visible in `pii run ls`, resumable, cancellable, and its journal carries every node's start/finish. Cancelling the run cancels the dispatch (196's rule applied to the queue).
6. Learning is automatic: on close, the outcome document is written through `pii memory learn` (206) with the item id, verdict, and the failing criterion when there is one.

### Non-Functional
- Idempotent selection: two concurrent ticks must never dispatch the same item (the store's single writer replaces the flock).
- Every status change is an event in the journal, so "why is this row parked" is a query, not an archaeology of logs.
- The transition is reversible for one release: the graph reads and writes the same store while `plan.tsv` is still importable.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| select_skips_unmet_needs | A1 needs A2(pending) | run output names A2; nothing dispatched |
| select_is_idempotent | two ticks at once | one dispatch, one event, the second tick reports `already-running` |
| blocked_is_not_empty | all pending rows blocked | OUTCOME names the blocking edge, never `queue-empty` |
| cancel_stops_the_dispatch | cancel the run mid-dispatch | the coder child is stopped, `run.cancelled` recorded, no re-dispatch |
| learn_on_close | an item that fails its criteria | one memory document naming item + criterion |

## Exit Criteria
- [ ] `cargo test -p pii-server --test queue_graph_selects`
- [ ] `cargo test -p pii-server --test queue_blocked_is_not_empty`
- [ ] `cargo run -q -p pii-cli -- graph check graphs/queue.toml`
- [ ] `pii run start queue` creates a run whose journal shows select → dispatch → judge → record
- [ ] `grep -c 'plan.tsv' scripts/autonomy/cycle.sh` is 0 (the shell loop is gone, or reads only for import)

## Guardrails
- Do NOT keep two sources of truth during the transition: the store wins as soon as the import runs.
- Do NOT let `select` fall back to "dispatch anyway" when needs are unmet.
- Do NOT put the commit inside the graph: a worker never commits.
- Do NOT schedule the graph and the shell cycle at the same time.

## Depends on
- 207 (the coder node spawns an agent) — without a dispatch node there is no queue node to compose.

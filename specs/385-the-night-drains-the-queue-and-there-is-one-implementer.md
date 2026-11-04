# Spec: the night drains the queue, and there is one implement-a-spec graph

## Overview

The operator asked: *"shouldnt the night have a queue on it?"* — and the measurement answers yes, for
two reasons at once.

**1. The shipped night does not work rows.** `graphs/night.toml` is `tick → call sdd → review →
report`. The queue is not on it. But `_docs/05-operations/the night loop` describes exactly the
opposite: *"a queue holds the gaps (one plan.tsv row per gap)"*. The document and the graph disagree,
and the graph is what ships.

**2. Two graphs implement a spec, and neither is the other's composition.**

```
spec   coder → judge → call docs → rejudge → done | unverified | refused     (queue calls this, row 364)
sdd    contract → call tdd → verify → call docs → reverify → ...            (night calls this, row 373)
```

`spec` proves a spec with a judge; `sdd` proves it with `tdd` (RED before GREEN). Both produce
`done | unverified | refused`. A host therefore has two ways to implement a spec, one of which
enforces the TDD contract and one of which does not — and which one runs depends on whether the work
arrived via the queue or via the clock.

That is precisely the "re-implements a flow another shipped graph already provides" case that spec 383
R6 makes the lint refuse. It cannot be fixed by an edge: one of the two graphs has to become the
single implementation.

**Project**: /projects/pi-improved
**Phase**: composability

## Decision (recorded 2026-11-04)

**`spec` is the one implement-a-spec graph, and it gains the TDD proof.** It is the one the queue
calls, the queue is how work arrives (a row carries the spec), and therefore it is the one that must
be strict. `sdd` is retired: its only unique content is the `tdd` call and the contract step, and both
belong inside `spec`. Retiring it removes a graph rather than adding one.

**`night` drains the queue.** `tick → call queue → review → report`. A night that does not drain the
queue is a clock that does nothing, whatever its report says.

## Requirements

- **R1** `graphs/night.toml` calls `queue` (pinned), not `sdd`. The night's job is to drain rows and
  report; the queue's job is to implement one row; `spec`'s job is to implement one spec.
- **R2** `graphs/spec.toml` proves a test was red before the code existed: it calls `tdd`, pinned, and
  a spec whose test passes without the change reaches a distinct port (`not-red` semantics preserved).
- **R3** `graphs/sdd.toml` and its documentation page are deleted, and no shipped graph calls it.
- **R4** The chain is closed and single: `night → queue → spec → tdd`, `spec → docs`. No graph
  implements a spec other than `spec`, and the lint in 383 R6 reports nothing for these files.
- **R5** A run of `night` on a queue with one row reaches that row's exit and reports it; a run on an
  empty queue reports `queue-empty` and does not fail.

## Exit Criteria

- [ ] `grep -q 'graph = "queue"' graphs/night.toml`
- [ ] `! grep -q 'graph = "sdd"' graphs/night.toml`
- [ ] `grep -q 'graph = "tdd"' graphs/spec.toml`
- [ ] `test ! -f graphs/sdd.toml`
- [ ] `cargo test -q -p pii-server --test night_calls_queue 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-server --test spec_is_one_graph 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`

## Guardrails

- Do NOT keep `sdd` "for compatibility": a second implementation of the same flow is the defect.
- Do NOT let `night` swallow a queue failure into `ok` — a night that could not drain must say so.
- Do NOT make `spec` call `sdd` to keep both alive. That is a rename, not a fix.
- Do NOT weaken the judge to keep `spec` passing: the TDD proof is added, not substituted.

## Files

- `graphs/night.toml`
- `graphs/spec.toml`
- `graphs/sdd.toml` (deleted)
- `graphs/DOCS/sdd.md` (deleted)
- `graphs/README.md`
- `crates/pii-server/tests/night_calls_queue.rs`
- `crates/pii-server/tests/spec_is_one_graph.rs`

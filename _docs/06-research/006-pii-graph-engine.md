# Pii Graph-Based Automation Engine — Architecture & Design

**Date**: 2026-09-06 · **Author**: orchestrator (via `pii-graph-engine` skill) · **Project**: /projects/pi-improved

## 1. Overview

Pii ships a deterministic, statically-validated **graph automation engine**
(`crates/pii-graph`) that is the control plane under every agent behavior:
a graph is a dataflow/control-flow program expressed in TOML (or the
built-in `default` graph) whose execution is **event-sourced** — every
step appends to an append-only run journal, and a run can be resumed by
replaying that journal. The daemon (`pii-server`) spawns graph runs over
JSON-RPC; the TUI streams their events live; the whole spec-driven
development pipeline (spec -> DAG -> implement -> gate) runs on pidag,
which uses pi/pii as its worker.

A graph is NOT a script. It is a typed assembly of **nodes** (pure
functions, LLM agents, tools, approval gates, timers), connected by
**edges** that carry data or ordering, with explicit **join policies**
for fan-in, **route** nodes for if/then/else, **loop budgets** for
bounded cycles, **capability declarations** at three layers, and
**effects declarations** that decide parallel vs serial execution.

## 2. The Specification Model (`crates/pii-graph/src/spec.rs`)

```
GraphSpec { name, entry: NodeId, caps, limits, nodes: [NodeSpec], edges: [Edge] }
NodeSpec  { id, kind, join: JoinPolicy, loop_budget: Option<u32>,
            retry: RetryPolicy, caps, cfg: serde_json::Value }
Edge      { from, port: PortName, to, kind: EdgeKind }
```

- **Edges**: `EdgeKind::Data` (value-carrying dependency: readiness + payload)
  vs `EdgeKind::Order` (ordering-only: readiness, no payload).
- **JoinPolicy** (how inbound edges gate a node's readiness):
  - `All` (default) — every inbound edge must fire (serial fan-in join).
  - `Any { cancel_losers: bool }` — any one inbound edge fires the node;
    optionally cancels sibling branches once one wins (a race).
- **Limits**: `max_steps`, `max_wall_ms`, `max_tokens`, `max_parallel`
  (0 = auto = available parallelism), per-run budgets enforced in the
  tick loop (step count, wall clock, token budget, micro-dollar budget —
  exhaustion is terminal and non-retryable).
- **RetryPolicy**: per-node `max` retries on retryable failures.
- **Expression language** (`crates/pii-graph/src/expr.rs`): deliberately
  tiny — `$.path.into.context` object-member access only; no arrays, no
  functions. Used by `transform.expr`, `route.on`, and tool args
  (`args = { command = "$.input.test_command" }`). Unresolvable paths
  yield `null`; malformed (non-`$.`) expressions are `Invalid`.

## 3. Node Kinds (composability primitives)

| Kind | Layer | Role | Output ports |
|---|---|---|---|
| `transform` | pure built-in | JSON reshape via `expr` | ok / err |
| `route` | pure built-in | branch on `on`-expr via `rules[]` | one per rule |
| `emit` | built-in | write a value to a file (fs_write cap) | ok / err |
| `wait` | pure built-in | sleep/fire-or-timeout (`fired`/`timeout`) | fired / timeout |
| `timer` | pure built-in | delay `in_ms` or 5-field `cron` | ok |
| `agent` | server | LLM agent: system prompt, tools allowlist, `max_turns`, `loop_budget`, compaction | ok / err / budget |
| `tool` | server | human-authored command (e.g. `bash`) | ok / err |
| `gate` | server | suspend the run for human approval | ok |
| `heal_detect` | server | self-healing ledger trigger (cargo xtask runner) | ok |
| `memory` | server | standalone `consolidate` maintenance op | ok |
| `worker` | server | supervised sub-process pool | ok |
| `crash` | debug | deliberate failure for tests | — |

Static lint surface (`pii graph lint`, V0+ rules in `validate.rs`)
rejects unknown kinds, dangling edges, unsatisfiable joins, and cyclic
specs without a `loop_budget`; every error message carries its rule code.

## 4. If/Then/Else — the `route` node

Branching is not a separate "cond" node: `route` maps a `$.` expression
to output ports via an ordered rule list, and edges wire each port to a
different downstream node. The last rule without an `eq` is the else.

```toml
[[node]]
id = "gate"
kind = "route"
on = "$.payload.exit_code"
rules = [
  { eq = 0, port = "pass" },
  { port = "fail" },          # fallthrough / else
]
```

Real examples in `graphs/starter/`:
- `verify-fix.toml`: `gate` routes the test exit code to `done` (pass)
  or `retry` (fail).
- `heal-fix.toml`: `route_probe` routes `ok` (exit 0) or `broken`;
  `route_retest` similar after the fix agent runs.

## 5. Loops — bounded cycles via `loop_budget`

Cycles are legal when a member node declares `loop_budget = N`.
`crates/pii-graph/src/sched/loop_helpers.rs`:
- `is_in_cycle` — tarjan SCC over the graph (a cycle = SCC >1 or a
  self-loop) — used by validation and rendering.
- `reset_loop` — when a cycle re-enters, BFS from the re-entry node and
  reset every node/edge in the reachable set to `Pending`, so the loop
  body genuinely re-runs instead of reading stale state.

Canonical example — `graphs/starter/verify-fix.toml`:

```
fix [agent] (entry) (loop_budget=3)
 |-- ok --> test
test [tool: bash]
 |-- ok --> gate
gate [route: $.payload.exit_code]
 |-- pass --> done
 |-- fail --> retry
retry [transform: $.payload]
 ^-- loops back to "fix" (budget=3)
done [transform: $.payload]
```

`fix` re-enters up to `loop_budget` times; exceeding the budget is a
terminal failure (`loop_budget exceeded for node 'fix'` — the
`loop_budget_exceeded_fails` test). Budget is checked before dispatch:
exactly `budget` executions, then terminal.

## 6. Capabilities and Effects (safety model)

**Capabilities** (`crates/pii-core/src/cap.rs`): bit flags — FS_READ,
FS_WRITE, PROC, NET, ... Effective node permissions are a **three-layer
intersection**: `run caps ∩ graph caps ∩ node caps`. A denial at any
layer fails the node with `capability denied`. This is a real bug class:
in 2026-09-05 the live `ollama:` model provider failed because NET was
missing at the run layer AND at the graph/node layers — all three had to
be fixed for a real LLM call to work.

**Effects** (`crates/pii-abi/src/effects.rs`): each node kind declares
static effect classes (PURE, READS_FS, WRITES_FS, NET, SPAWNS_PROC,
MUTATES_EXTERNAL, LONG_RUNNING). The scheduler partitions each batch:
`PURE | READS_FS` run in parallel (bounded by `max_parallel`, one OS
thread per chunk), `WRITES_FS | MUTATES_EXTERNAL` (the `SERIALISED`
default when undeclared) run sequentially. Effects fail closed — a node
that does not declare its effects is treated as fully serialising.

## 7. Execution: tick loop, batch drain, event-sourced journal

`crates/pii-graph/src/run.rs` + `sched/state.rs` + `sched/dispatch.rs`:

1. **Tick loop**: propagate dead branches to a fixpoint (journaling each
   new `NodeDead`); fail LOUDLY on an unsatisfiable join; drain ALL ready
   nodes into one batch (never `.pop()`); check loop budgets; dispatch.
2. **Partition**: parallel-safe nodes (effects) run in bounded thread
   chunks; serialising nodes run in order. Results apply as they arrive;
   `cancel_losers` runs after each result (race losers become
   `NodeCancelled`).
3. **Journal** (`journal.rs`, `pii-abi/src/journal.rs`): EVERY event
   appends to the run's append-only journal in redb — `Started`,
   `TickStarted { batch, width }`, `NodeStarted` (journaled BEFORE the
   node runs, so an interrupted node is visible as "may have executed"),
   `NodeFinished { port, value, value_hash, dur_ms }`, `NodeFailed`,
   `NodeCancelled`, `NodeDead`, `Suspended { key }`, `Resumed`,
   `Finished { terminal, output_hash }`. A header (status + next-seq)
   is persisted with each append; the blackboard snapshots every 32
   events and unconditionally on suspend.
4. **Terminal dispositions** (`RunStatus`): Running -> Succeeded | Failed
   | Suspended (human approval, e.g. heal-fix's `gate`) -> Resumed.
5. **Resume** (`run.rs::resume`): reads the journal, recovers the input
   from the content-addressed store, replays every event to rebuild
   state/blackboard/completion order, re-propagates dead, appends
   `Resumed`, and continues the tick loop. A node with a terminal event
   is never re-invoked; a `NodeStarted` with no terminal event re-runs
   (the documented Phase-1 boundary).

The **seq** counter is the single source of run ordering — it is the
same number the TUI shows as the live `[seq 0x…]` thinking indicator and
that `pii graph trace <run_id>` prints per event, so a live session can
be cross-referenced against a persisted trace by seq.

## 8. Research and implementation based on `git log --graph`

### 8.1 The lane-allocation algorithm (what git actually does)

`git log --graph` renders a DAG as terminal ASCII via a **lane
allocation** walk: keep an array of active slot/columns; walk items in
topological order; each item claims the slot that expects it (continuing
its parent's lane), the lowest free slot (a new branch), or opens extra
slots per additional parent (a merge/fan-in: `├─╯`) or per additional
child (a fork/fan-out: `├─┐`). Rows are commits; columns are edges into
past/future. It is tractable precisely because git renders **history**:
child-to-parent, strictly forward, no cycles.

### 8.2 Porting to pii graphs (specs/10-graph-visualize.md)

Vocabulary mapping: node = row (in topological order); Data/Order edge =
parent/child edge; `JoinPolicy::All`/`Any` fan-in = merge; `route`
fan-out = branch.

**The one thing that does not port: loops.** Git's DAG has no cycles;
the lane algorithm assumes forward flow. pii graphs genuinely cycle
(`loop_budget`). Forcing a back-edge through lane columns reproduces the
diagonal-crossing mess git itself shows on merge-heavy history. Design
decision: lay out nodes in topological order (back-edges excluded via
`is_in_cycle`) and render each loop closure as a separate annotation
line — `^-- loops back to "fix" (budget=3)` — never as a lane.

### 8.3 "You can repeat the day again" — why git never draws a cycle

The deeper reason (user's correction, 2026-09-06): git never renders
*structure*, it renders *history* — a commit that redoes yesterday's
work is a NEW commit further down the line, not a back-edge to
yesterday's. Time moves forward, so repeated work still has zero cycles
to draw. The same applies to pii's loops **once a run exists**: each
iteration is distinct executions at increasing seq
(`fix#1, test#1, gate#1, retry#1, fix#2, test#2, ...`). A trace-mode
renderer that lists EXECUTIONS (one row per seq event) instead of
DEFINITIONS (one row per node id) therefore unrolls every iteration like
`git log` unrolls repeated work — no cycle handling needed, because what
actually happened has no cycles by construction. The static
definition view (no run yet, `loop_budget=3` = "up to 3 may happen", not
"3 did happen") is where the `^-- loops back` annotation is unavoidable.

### 8.4 Implementation: `pii graph show <file>`

Committed `be32ca1` (spec 10, 7 unit tests, all starter graphs render):
deterministic, validate-first, **ASCII-only** (no `├─┐`/`●` box glyphs —
the standing no-emoji/plain-ASCII convention), rows of
`<id> [<kind-detail>]` plus `(entry)` / `(loop_budget=N)` /
non-default `(join: ...)` annotations, fan-out as
`|-- <port> --> <to>` lines, loops as the annotation line. Example
output (the real verify-fix.toml) is shown in section 5.

The trace-unrolling renderer (8.3) is recorded in the spec as the right
long-term answer for viewing loops post-run, but is a separate future
spec — v1 is static-structure-only, no daemon dependency.

## 9. The control plane around the engine

- `pii-server` (`Engine`, `crates/pii-server`) owns the registry
  (built-ins + agent/tool/gate/heal/memory/worker), spawns runs
  (`run.start`, `run.resume`, `run.cancel`), tracks sessions, and
  streams events (`event.subscribe` — replay-since + live, bounded
  queue, `Lagged` on overflow).
- `pii graph lint|run|resume|trace|show` and `pii tui` are the human
  surface; the daemon socket is local-only Unix (0600).
- Starter graphs in `graphs/starter/` are the living reference:
  `assistant` (agent -> transform), `react` (agent ok/err/budget
  routing), `verify-fix` (fix/test/gate/retry loop), `heal-fix` +
  `heal-watch` (self-healing: confined fix agent, approval gate,
  human-escalation), `slow`/`soak`/`soak_cron` (stress), `consolidate`
  (memory maintenance), `crash` (known-bad debug graph).

## 10. Status of adjacent work (as of 2026-09-06)

- **Journal hardening** (specs/11-journal-hardening.md, committed
  `b60a763`, IN FLIGHT via pidag): batch journal appends (one redb
  write transaction per N events + explicit flush at terminal/suspend/
  status transitions) and a synthetic `NodeInterrupted` marker on
  resume for nodes left mid-flight by a crash.
- **Global memory** (specs/09-global-memory.md): DRAFT, not implemented.
- **Web viewer** (specs/12-web-viewer.md): DRAFT, not implemented.
- Full dsh-vs-pii comparison incl. the graph layer: `_docs/06-research/
  005-dsh-vs-pii-harness-analysis.md`.

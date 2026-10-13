# Spec: a run survives the death of the process that was running it

> **STATUS: RESEARCH — NOT QUEUED FOR IMPLEMENTATION.** Held for the claude+opus review (user directive
> 2026-10-13: *"marque os specs como pesquisa e passaremos ao claude+opus antes de qualquer implementação"*).
> The review must judge them against the point of the source they came from: proactivity applied to pii's own
> self-improvement — the loop reads its logs by itself, resolves by itself, and asks permission only to put a
> change into production, with the operator as far from the loop as possible, deciding only the end.

## Overview

pii persists run *records*; it does not persist the *step*. A run whose process died mid-node stays `Running`
for ever, because nothing on start-up looks for it and a node cannot say whether re-running is safe. 135 fixed
the node that panics; this is the same class one level up — the process that dies.

The contract comes from Pi Durable (Earendil, 2026-10-12; `_docs/06-research/060-durable-agents-pi-durable-2026-10-13.md`):
**every step of a run is a task that stores a checkpoint before it moves on**; a new process opens the same
storage, finds the unfinished tasks and continues each from its last checkpoint; a model request cut off is
re-sent with the partial answer marked **aborted**; a tool call cut off **reruns only if it declares
`replay: "safe"`**, otherwise the model is told it was **interrupted**, with the output stored so far.

**Project**: /projects/pi-improved
**Phase**: post-vision hardening (durability; 060 §1)
**Source of the finding**: `_docs/06-research/060-durable-agents-pi-durable-2026-10-13.md`; DO-NOT-REPEAT 1.50
(a value asserted where nobody reads it) and 135 (a panicking node must produce a terminal)

## Requirements

### Functional
- **R1** On daemon start-up the store is swept for runs left unfinished by a previous process. Each one is
  either resumed or brought to a terminal state in that sweep — never left `Running` for a caller to poll
  for ever.
- **R2** A node declares whether re-running it is safe after a crash: a `replay_safe()` (or equivalent) on the
  node kind, **fail-closed** (the default is *not* safe). A node that does not declare it never reruns.
- **R3** A run cut off mid-node that is not replay-safe ends in a new terminal outcome **`Interrupted`**, with
  the output produced so far recorded in the run record and an event. A replay-safe node is resumed and the
  run finishes normally; if its resumption fails, it ends `Failed` as any node would.
- **R4** The sweep is **observable and idempotent**: one event per recovered run, a counter in `pii metrics`,
  and running the sweep twice changes nothing (the second pass finds nothing to do).
- **R5** The sweep respects the refusal path: a run whose graph is quarantined is refused (spec 125/131) and
  records a decision — it is never silently resumed.
- **R6** A run that is `Interrupted` must be indistinguishable, to a caller, from any other terminal run:
  `pii run get` answers with the outcome and the reason, and no caller polls for ever.

### Non-Functional
- No new dependency. Recovery runs before the daemon accepts work, so no caller can observe a half-recovered
  store.
- `#![forbid(unsafe_code)]` untouched; clippy `-D warnings` clean; the store stays the single writer's.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `a_run_left_running_by_a_dead_process_is_recovered` (new) | a store with a run in `Running` whose node is not replay-safe | after the sweep: `Interrupted`, the partial output recorded, an event emitted |
| `a_replay_safe_node_is_resumed_and_finishes` (new) | the same, but the node declares itself safe | the run completes normally |
| `the_sweep_is_idempotent` (new) | run the recovery twice | the second pass reports nothing to do |
| `a_quarantined_graphs_run_is_refused_not_resumed` (new) | the run's graph is quarantined | refused with the decision recorded (125/131 behaviour kept) |
| the pre-fix shape (scratch copy) | — | RED: the run stays `Running` — the control that proves the test sees the defect |

## Exit Criteria

- [ ] `cargo test -p pii-server --lib 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-store 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -1 | grep -qv 'error'`
- [ ] `grep -rn 'Interrupted' crates/pii-abi/src crates/pii-store/src | head -1` is non-empty (the outcome is a
      first-class value, not a string invented at the call site)

## Guardrails

- Do NOT sweep by deleting or rewriting history: an unfinished run keeps its record and gains an outcome.
- Do NOT resume anything by default. A node that does not say it is replay-safe does not rerun — a duplicate
  side effect is worse than an `Interrupted` run a person looks at (the same reasoning as the guardrail in
  spec 132: fail-closed beats convenient).
- Do NOT extend a timeout to hide a stuck run.
- If the store's run shape cannot express `Interrupted` without a migration, say so and STOP — a migration is
  its own item.
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit; scratch under `/projects/_tmp/`.

## Closure

(outcome, the recovery event and counter, the pre-fix RED, and the idempotence check — filled in when the item
is closed.)

# Spec: 131 — a refused run is a decision, and the ledger must show it

**Project**: /projects/pi-improved
**Phase**: after the `057` plan (111–130). Filed 2026-10-10 by the orchestrator while closing spec 125, which
recorded a deviation: the promotion's audit trail (an append-only `learn/ledger.jsonl`) is **not** the decision
ledger the spec asked for, because the CLI cannot write the redb the daemon holds open.

The deviation was accepted then. Digging in now sharpens it into a defect rather than a filing:

```rust
// crates/pii-server/src/engine/runs.rs:88-95 — the engine denies a quarantined graph
let started = quarantine_of(&engine, &spec.name).map_or_else(
    || pii_graph::run::run(&engine.store, &spec, &engine.reg, &ctx, &run_id, input),
    |reason| Err(PiiError::Denied(reason)),
);
```

Nothing is recorded. And the decision-ledger API's own documentation states the rule this breaks:

```rust
// crates/pii-store/src/heal.rs:321-323
/// Always sets `decision`, whether `allow` or
/// `deny` — the ledger must show a denied promotion just as durably as
/// an approved one.
```

So the product's self-improvement loop can **stop a graph and leave no decision behind**: the effect the loop
produced on the running system is not in the record the loop is audited by. The daemon owns the store and is the
only writer in this path, so the record belongs here — not in a CLI-side JSONL.

## Requirements

### Functional

- **R1 — the refusal is recorded.** When a run is refused because an in-effect promotion quarantines its graph,
  the engine records a **deny** decision through `pii_store`'s decision ledger, carrying at least: the graph, the
  named reason (the same string the error carries), the promotion id, and the run id.
- **R2 — it is durable and readable.** A *fresh process* opens the store and reads that decision back with those
  fields intact. Read-back is the criterion; an in-memory counter is not evidence.
- **R3 — the refusal is unchanged.** The caller still gets the same `PiiError::Denied(reason)` with the same
  named reason, and the existing spec-125 tests keep their claims.
- **R4 — a bookkeeping failure never hides the refusal.** If the ledger write itself fails, the refusal STILL
  stands (fail-closed), the failure is visible (a named event and/or a counter), and the two are counted
  separately — "refusals" and "refusals recorded" are two numbers, and a gap between them is a finding.
- **R5 — only denials.** An ordinary allowed run writes no decision: the ledger does not become a run log.
- **R6 — mutation control.** Deleting the record call reddens the read-back test. Paste the red output.

### Non-Functional

- 450-line file rule; no new dependency; `cargo fmt` + `clippy -D warnings` clean; tests create their scratch
  under `_tmp` with the sanctioned helper; the write is bounded (no unbounded retry).

## TDD Contract

| Test Name | Input | Expected Output |
| --- | --- | --- |
| `a_refused_run_is_recorded_as_a_deny_decision` | a graph quarantined by an in-effect promotion; one run start | the run is refused AND a deny decision exists with the graph, the reason, the promotion id and the run id |
| `the_recorded_decision_survives_a_restart` | the decision written; store closed and reopened by a new reader | the decision is still there with the same fields |
| `an_allowed_run_writes_no_decision` | a graph with no quarantine; one run start | no decision is written (the ledger stays empty) |
| `a_ledger_write_failure_does_not_swallow_the_refusal` | the ledger write made to fail (injected) | the caller still gets `Denied`; the failure is counted/emitted; the two counters disagree by exactly one |

## Exit Criteria

- [ ] `cargo test -p pii-server --lib` passes, with the four tests above present and passing
- [ ] `cargo test -p pii-store --lib` passes (the ledger read-back path is exercised through the store's own API)
- [ ] the mutation in R6 reddens `a_refused_run_is_recorded_as_a_deny_decision` (output pasted, then restored byte-identical)
- [ ] the two counters (refusals, refusals recorded) are rendered by `pii metrics` from a live daemon and differ by 0 after an ordinary refusal
- [ ] `bash quality-gate.sh` passes in the dev tree

## Guardrails

- Do NOT write the decision from the CLI, and do NOT add a second JSONL trail. The daemon holds the store; the
  record is written where the effect happens.
- Do NOT weaken the refusal to make recording possible, and do NOT make recording a precondition of the refusal.
- Do NOT record allowed runs.
- Do NOT touch `specs/`. If the store's decision API cannot carry a *system* decision (it was built for heal
  approvals, keyed by `signature_hash`) without a signature that misrepresents this event, STOP and report the
  shape you would need — do not overload `sig` with something that is not a signature.

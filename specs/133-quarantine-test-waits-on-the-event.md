# Spec: the quarantine test waits on the event it asserts

## Overview

`crates/pii-server/src/daemon/mod.rs::tests::a_quarantined_graph_is_refused_by_the_engine_and_runs_again_after_revert`
is the test that failed twice in `fresh-gate` on a loaded machine (spec 125's class, 2026-10-10/11) and could
not be reproduced in eight clean rounds. It was reproduced deterministically on 2026-10-11: see
`_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §2.2 (Q7) — inserting a **300 ms pause between the
commit and the event delivery** in a scratch copy turned it red **3 of 3** at `daemon/mod.rs:716:14`, with the
unpaused control green 3 of 3.

The mechanism is a test that waits for one fact and then asserts a different one:

```rust
assert_eq!(finished(&refused).status, RunStatus::Failed);   // waits for the STORE status
let ev = rec.events();                                      // then takes one SNAPSHOT of the events
let e = ev.iter().find(|e| e.kind == RUN_START_REFUSED)     // 716: asserts on the EVENT
    .expect("run.start.refused");
```

`finished()` returns as soon as the run's store status is terminal. Whether the `run.start.refused` event has
reached the observer by then is a separate fact, and under load the observer lags. The failure is therefore
**not a timeout** — raising the bound (already raised 2 s → 60 s by `5cb67a9`) cannot fix it, and a sleep only
moves the fuse.

**Project**: /projects/pi-improved
**Phase**: post-vision hardening (finding 059 F3 / Q7)
**Source of the finding**: `_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §0, §2.2, §2.3

## Requirements

### Functional
- **R1** The test must wait for the fact it asserts: after the run is terminal it must poll the observer's
  events (bounded, with the bound stated in the code as a constant with its reason) until
  `RUN_START_REFUSED` is present, or panic with the number of events actually seen. Waiting on the store and
  reading the events once is the defect; waiting on the event is the fix.
- **R2** The assertions must not be weakened. The refusal must still (a) be found, (b) carry an `error` field
  containing the exact expected text, and (c) be exactly **one** `RUN_START_REFUSED` at the end of the test.
  A test that passes by relaxing (a)/(b)/(c) is a worse outcome than the flake.
- **R3** The same wait-on-the-event helper is used for the final count assertion (line 736-741), so the count
  is read after the last event, not before it.
- **R4** No sleep is introduced as the mechanism and the 60 s bound is not raised as the fix.
- **R5** The run for an unrelated graph (`healthy`) and the post-revert run stay as they are: this spec changes
  how the test *waits*, not what it proves.

### Non-Functional
- The file keeps `#![forbid(unsafe_code)]` and the workspace keeps clippy `-D warnings` clean.
- No new dependency. No change outside `crates/pii-server/src/daemon/mod.rs` (and, if a helper must live
  elsewhere, say so in the report and keep it inside `pii-server`).
- Every other test in `pii-server` keeps passing, unchanged.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `a_quarantined_graph_is_refused_by_the_engine_and_runs_again_after_revert` | the existing fixture, unchanged | still red on a broken premise; waits on the event; asserts the error text and the count of 1 |
| (RED first) the same test with the event wait removed | a scratch copy with the pasted snapshot (the pre-fix shape) | red at the event expectation when delivery is delayed |
| (CONTROL, orchestrator) the fixed test with a 300 ms pause between the commit and the event delivery | scratch harness under `/projects/_tmp/` | **10 of 10 green** — this is the acceptance, and it is run outside the spec by the orchestrator |

## Exit Criteria

- [ ] `cargo test -p pii-server --lib 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-server --lib daemon::tests::a_quarantined 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo clippy -p pii-server --all-targets -- -D warnings 2>&1 | tail -1 | grep -qv 'error'`
- [ ] the new wait is a bounded poll with its bound as a named constant: `grep -n 'RUN_START_REFUSED' crates/pii-server/src/daemon/mod.rs` shows the wait helper used for both the find and the count

## Guardrails

- Do NOT weaken R2's assertions to make the test pass. If the fixed test cannot find the event within its
  bound, that is a finding about the engine, not a reason to relax the test — **STOP and report it**.
- Do NOT raise the `finished()` bound and do NOT add a sleep: neither is the mechanism (059 §2.2 measured it).
- Do NOT create, edit or rewrite anything under `specs/` (read-only for implementers).
- Do NOT commit. Leave the tree for the orchestrator.
- Destructive commands are banned; scratch under `/projects/_tmp/` only, and only scratch you create.
- If the shape differs from this spec's premise (the test was moved, the event kind renamed, the observer
  API changed), STOP and say so in one line rather than forcing the change.

## Closure

(outcome, evidence, the orchestrator's own mutation, and the 10-of-10 control — filled in when this item is
closed; the cycle commits the code, the closure section is written by the orchestrator.)

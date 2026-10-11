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

**Outcome: closed 2026-10-11 — the test waits on the event it asserts; the fix is real under the 059 probe.**

What landed: `crates/pii-server/src/daemon/mod.rs` — `refusals_seen(at_least)`, a bounded poll of
`rec.events()` (bound `EVENT_POLLS = 6000` × 10 ms = the same 60 s ceiling as `finished()`, stated as a
constant with its reason), used for **both** the `find` and the final count assertion (R1, R3). No assertion
was relaxed: the refusal is still found, its `error` field still carries the exact text, and the count is
still asserted to be exactly 1 (R2). No sleep was added as the mechanism and no bound was raised (R4).

Evidence, all run on the tree the worker left (commit `9fc7f9f`):

| check (the spec's own Exit Criteria) | result |
|---|---|
| `cargo test -p pii-server --lib` | rc=0 — `test result: ok. 117 passed; 0 failed; 1 ignored` |
| `cargo test -p pii-server --lib daemon::tests::a_quarantined` | rc=0 — `1 passed; 0 failed` |
| `cargo clippy -p pii-server --all-targets -- -D warnings` | rc=0 |
| the wait helper is used for both the find and the count | `mod.rs:734 refusals_seen(1)` (find) and `mod.rs:759 refusals_seen(1)` (count) |

**The orchestrator's own mutation (059 §2.2's probe), not the worker's word.** A 300 ms
`std::thread::sleep` inserted after the broadcast in `refuse_start`
(`crates/pii-server/src/engine/runs.rs:204`, the exact repro of 059 §2.2):

- **CONTROL, pre-registered: 10 of 10 green** with the fixed test and the probe in place (`green=10 red=0`).
- **REDDEN: 3 of 3 red** with the SAME probe and the test returned to its pre-fix shape (wait on the store
  status, read the events once): `panicked at crates/pii-server/src/daemon/mod.rs:738:14: run.start.refused`.
  The redden is also the control that the probe bites — a probe that changed nothing would have left the
  pre-fix shape green.

Both files were restored byte-identical afterwards (`sha256sum` before/after:
`mod.rs 43e22b8c…511c9`, `runs.rs f296af07…20a6`), and the tree is clean at `9fc7f9f`.

**The loop could not commit this item, and that was a defect in the loop, not in the work.** The cycle
reached `fresh-gate PASS` at 16:16:31Z and then died at `git add`: this plan row listed `pii.toml` among its
commit paths, a file that exists nowhere in the tree, so `git add` aborted with
`fatal: pathspec 'pii.toml' did not match any files` (`OUTCOME=add-failed`) and the item stayed
`needs-review` — a state in which the next cycle would have re-dispatched the same session, and billed it
again, forever. Fixed in `a86e85c` (the row now lists `crates;Cargo.lock;scripts/autonomy`; it was the only
row with the phantom path).


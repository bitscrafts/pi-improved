# Spec: no test opens a store at a path it does not own

## Overview

A redb store is a single-writer resource: the second process that opens the same file dies with
`Database already open. Cannot acquire lock`. Two runs of the same test binary are not exotic — a
developer's `cargo test`, a `fresh-gate` export, and a dispatched worker's run overlap routinely — so a test
that opens a **fixed path** is a test that fails depending on who else is running. It was paid for once
already: `crates/pii-memory/tests/persistence.rs` used `_tmp/phase3/persist-test.redb`, was fixed to a
per-invocation path (`eac80dc`, DO-NOT-REPEAT 1.48), and the same class was then found **still present** at
`crates/pii-memory/tests/lanes.rs:30` — `_tmp/phase3/lanes-test.redb`, used by three tests — where two
concurrent runs of the binary fail **2 of 6 rounds** (`_docs/06-research/059-opus-vision-judgement-2026-10-11.md`
§0, Q7-adjacent F4).

A survey of the tree (`grep -rn '_tmp/phase3' crates/*/tests/*.rs`) finds about a dozen sites that derive a
scratch directory from `CARGO_MANIFEST_DIR`. **Not all of them are this defect**: only a site that opens a
single-writer resource (a redb store, a lock file, a socket or a fixed port) at a name shared between runs
can collide. The job is to fix the ones that can, and to say with evidence why the others cannot.

**Project**: /projects/pi-improved
**Phase**: post-vision hardening (finding 059 F4; DO-NOT-REPEAT 1.48)
**Source of the finding**: `_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §2.2 (F4), §2.3

## Requirements

### Functional
- **R1** `crates/pii-memory/tests/lanes.rs` gives every test invocation its own store path, mirroring the
  pattern already proven in `crates/pii-memory/tests/persistence.rs` (`db()` = a per-invocation directory
  under the crate's `_tmp/phase3`). Keep a comment that states the mechanism and the measurement, as that
  file does — the comment is what stops the next person from "simplifying" it back.
- **R2** Sweep the class and **report it**: list every test site that opens a store (or another
  single-writer resource) at a path under `_tmp/phase3`, and for each either (a) make it own its path, or
  (b) state the evidence for why it cannot collide (it does not open a store; it already uses a tempdir; it
  is a single test in its binary and the path is unique to it). An unexamined site is the defect.
- **R3** Commit the class's **control**, so this is verifiable forever rather than once:
  `scripts/checks/concurrent-store.sh <package> <test-binary> [rounds]` runs the same test binary twice at
  the same moment, `rounds` times, and prints one line `ROUNDS <n> COLLISIONS <k>` (k = rounds in which any
  process reported a lock/open failure). Exit 0 only when k = 0. It must create its own scratch, clean only
  its own, and be safe to run while a gate exports the same tree.
- **R4** No test is weakened, ignored, `#[ignore]`d, or made to pass by retrying. The fix is the path.
- **R5** The RED is observed: run R3's harness before R1/R2 and record the collision count (the finding
  measured 2 of 6). After the fix it must be 0 of 10.

### Non-Functional
- No new dependency. No test loses coverage. Scratch stays under `_tmp/` (a test creates the scratch it uses).
- clippy `-D warnings` clean; the workspace `#![forbid(unsafe_code)]` untouched.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `lanes` (all three tests) | the per-invocation path | green, unchanged assertions |
| `scripts/checks/concurrent-store.sh pii-memory lanes 10` | two simultaneous runs, 10 rounds | `COLLISIONS 0`, rc 0 |
| the same harness on the pre-fix tree (scratch copy) | the fixed path | `COLLISIONS >= 1` — the control that proves the harness can see the class |

## Exit Criteria

- [ ] `cargo test -p pii-memory --test lanes 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `bash scripts/checks/concurrent-store.sh pii-memory lanes 10 2>&1 | tail -1 | grep -q 'COLLISIONS 0'`
- [ ] `cargo test -p pii-memory 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo clippy -p pii-memory --all-targets -- -D warnings 2>&1 | tail -1 | grep -qv 'error'`

## Guardrails

- Do NOT fix a collision with a retry, a sleep, a lock file you keep, or `#[ignore]`. A test that waits for
  the other run is a test that hides the defect (the class is a **path** class).
- Do NOT delete or rewrite other tests' scratch directories; leave anything you did not create alone.
- Do NOT add a dependency and do NOT invent a new test framework: the harness is a shell script.
- If a site's store API differs from the one in `persistence.rs`, fix only the sites whose mechanism you can
  demonstrate — and list the rest as unexamined in the report rather than claiming a sweep you did not do.
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit; scratch under `/projects/_tmp/`.

## Closure

**Outcome.** The class is fixed and now has a control that can see it.

- `crates/pii-memory/tests/lanes.rs` gives every test invocation its own store path (a per-pid directory under
  the crate's `_tmp/phase3`), with the mechanism and the measurement written above the function — the comment
  is what stops the next person from "simplifying" it back to a fixed name.
- The sweep found and fixed the same class in three more test files: `crates/pii-server/tests/fail_port.rs`,
  `failover_graph.rs`, `model_routing.rs`.
- The control is committed: `scripts/checks/concurrent-store.sh <package> <test-binary> [rounds]` runs the same
  test binary twice at the same moment and prints `ROUNDS <n> COLLISIONS <k>`, exiting 0 only when k = 0.

**The numbers.**

| run | result |
|---|---|
| the pre-fix shape, harness (the finding's measurement) | 2 of 6 rounds collide |
| the pre-fix shape, harness (orchestrator's own control, mutation restored byte-identical) | `ROUNDS 6 COLLISIONS 1` |
| the fixed tree, harness | `ROUNDS 10 COLLISIONS 0` |
| the item's own criteria on the tree the worker left | `cargo test -p pii-memory --test lanes` rc 0; harness rc 0 |
| `fresh-gate --worktree` on that tree | `quality-gate: ALL PASS` |

**Restored byte-identical.** The mutation that reddened the control was reverted and the file hashed back to
`b6616df02934da94` before this commit.

**Note on how this item was closed.** The cycle that produced it was killed *after* its gate passed and before
its commit, because the orchestrator rewrote `cycle.sh` while that cycle was still executing it
(DO-NOT-REPEAT 2.15 — a running script is not editable). The work was verified by hand against the same gate
output and committed by the orchestrator, which is the role the workflow assigns the commit to anyway.

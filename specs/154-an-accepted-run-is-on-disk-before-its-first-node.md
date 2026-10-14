# Spec: an accepted run is on disk before its first node runs

## Overview

The Opus 061 judgement measured the real defect behind the durability question (C-run, C-read): killing the daemon
1 s and 15 s into a 30 s node leaves **no record at all** — `pii run get` answers "unknown to this daemon".
The reason is that the run's journal buffers up to **16 events** before writing (`DEFAULT_BATCH`,
`crates/pii-graph/src/journal.rs:45`), so a crash in the first moments loses the whole run. What exists today
(`reconcile_boot`, `crates/pii-server/src/engine/session_durability.rs:300-324`, called at
`crates/pii-cli/src/cmd_daemon.rs:417`) can only reconcile runs that are **in the store**, and it discards its own
write result with `let _ =` (`:315`).

This is the only durability item the proactivity thesis needs: **a crash must leave evidence a log reader can see.**

**Project**: /projects/pi-improved
**Phase**: proactivity (the Opus 061 judgement; order 153 → 150 → 154 → 151 → 152)

## Requirements

### Functional

R1. At acceptance, the run header (`Running`) and its `Started` event are written **synchronously**, outside the
    16-event batch — so the run exists on disk before its first node executes.
R2. `reconcile_boot` handles its write error instead of `let _ =`, and emits **one event per reconciled run**
    (which run, why).
R3. A run whose process died mid-node is terminal after the next daemon start, with reason
    `interrupted by daemon restart`, and it is **not** auto-resumed (decision G8 stands).

### Non-Functional

R4. The extra synchronous write happens once per run, not per event, and does not change any existing event order.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| an_accepted_run_is_readable_before_its_first_node | accept a run on a slow node, kill the daemon at 2 s, restart | `pii run get <id>` answers terminal `failed`, reason `interrupted by daemon restart` |
| a_reconcile_emits_one_event | the same restart | exactly one reconcile event naming the run |
| a_write_error_is_not_swallowed | a store that refuses the header write | the error is surfaced, not `let _`-ed |
| nothing_is_auto_resumed | any reconciled run | its status is terminal, and no node ran after the restart |

## Exit Criteria

- [ ] `cargo test -p pii-server --lib 2>&1 | tail -2 | grep -q 'test result: ok'` and the four tests above are listed
- [ ] a recorded `kill -9` experiment in the closure: before the change the run answers "unknown", after it answers terminal
- [ ] `cargo xtask fresh-gate` on this spec's commit is `ALL PASS`

## Guardrails

- Do NOT add auto-resume (G8 decided against it) and do NOT invent a second interruption vocabulary: `NodeInterrupted`
  already exists (`crates/pii-abi/src/journal.rs:122-129`).
- Do NOT make every event synchronous: one extra write per run (R4).
- Do NOT swallow a reconcile error again.

## Closure (2026-10-14, orchestrator, independent of the worker's report)

Verification of the durability claim on the committed tree (`c509d1b`) — measured, not read from a report.

- **Criterion 1, re-run by me**: `cargo test -p pii-server --lib` → `test result: ok. 129 passed; 0 failed;
  1 ignored`, and the four TDD-contract tests are listed by name:
  `an_accepted_run_is_readable_before_its_first_node`, `a_reconcile_emits_one_event`,
  `a_write_error_is_not_swallowed`, `nothing_is_auto_resumed`.
- **Criterion 2, the `kill -9` experiment, run by me on the real wire** —
  `bash scripts/checks/daemon-crash-leaves-evidence.sh` (real `pii daemon`, real control socket, a 30 s
  `wait` node, `kill -9` 2 s in, daemon restarted, `pii run get` asked afterwards):
  - fixed tree → `{"dur_ms":0,"graph":"...","run_id":"r000000000001","seq":2,"status":"failed"}`;
    `pii graph trace r000000000001 --db <store>` shows `seq 0 Started`, `seq 1
    Finished{terminal:{Failure:{error:"interrupted by daemon restart"}}}`; the near-miss control (an id that
    never existed) still answers `run 'no-such-run-154' is unknown to this daemon (not in flight, not
    stored)`; verdict `CRASH-EVIDENCE OK`.
  - the *same* harness against the un-fixed tree (`EXPECT=pre`, see the mutation below) →
    `run 'r000000000001' is unknown to this daemon` — 061's recorded "before" half reproduced live, by the
    same harness, so neither direction is a hand-written input.
- **My own mutation, not the worker's**: `jr.flush()?;` at `crates/pii-graph/src/run/mod.rs:103` removed.
  It reddens both legs: `cargo test -p pii-server --lib
  an_accepted_run_is_readable_before_its_first_node` → `FAILED. 0 passed; 1 failed` (panicked at
  `reconcile_tests.rs:139`, "the probe node ran" — the header was not on disk when the node ran), and the
  live harness answered `unknown`. Restored byte-identical: `sha256sum` of the file
  `c7f7362ffc508b946f4cc1304fccf1b141b4d57686ef76785d062aa6d1be2442` before and after, `git diff --stat`
  empty.
- **Criterion 3, `fresh-gate`**: `cargo run -q -p xtask -- fresh-gate` on the working tree →
  `quality-gate: ALL PASS` / `fresh-gate: PASS` (recorded below by the run that produced it).

**The gap this closure fills.** The commit carried no `## Closure`, and criterion 2 — the recorded `kill -9`
experiment — existed nowhere; the A19 plan row had also dropped criteria 2 and 3, carrying criterion 1 alone.
Criterion 1 by itself is satisfiable in-process: `reconcile_tests.rs` reads the store from inside the same
process and calls `reconcile_boot()` directly, so the TDD contract's row 1 ("accept a run on a slow node,
kill the daemon at 2 s, restart") is implemented as an in-process probe, not as a crash. The live form now
exists as the harness above, and it is what the durability claim rests on.

**A pitfall re-earned while measuring.** The first version of the harness probed readiness with
`[ -S "$PII_RUNTIME_DIR/control.sock" ]`. A SIGKILLed daemon leaves that socket *file* behind, so the probe
answered "ready" while the next `pii run get` failed with ENOENT — a check that passed on a dead live path.
The probe is now a real round trip (`pii run ls`). Also: mutation backups must live OUTSIDE the scratch the
harness rotates (`mv`-ing the scratch aside carried the backup away; the restore was made from the moved
copy).

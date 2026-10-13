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

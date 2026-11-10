# Spec: a panicking node produces a terminal run

## Overview

When a node panics, the run stays `accepted` and `in flight; poll again` **forever**, and nothing is written
to the crash record. Measured 2026-10-11 (`_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §0, F2):
a node that panics leaves `pii run get` answering with the same in-flight state minutes later, so a caller
that polls — a channel poller, `pii-evolve`, a person — waits for a run that will never finish, and the
failure is invisible in the run record and in the events.

This is the worst shape of defect for an autonomous system: a run that cannot end and cannot be seen to have
failed. The fix is a terminal state plus a record, not a retry.

**Project**: <repo>
**Phase**: post-vision hardening (finding 059 F2)
**Source of the finding**: `_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §0, §2.2 (F2), §2.3

## Requirements

### Functional
- **R1** A node that panics must leave the run in a **terminal** state (`Failed`) with an error that names the
  node and the panic, in the run record. A caller polling `pii run get` must stop waiting.
- **R2** The panic must be **recorded as an event**, on the same observability seam as any other failure
  (`pii-observe` names; no raw print). A crash that only exists in stderr is not recorded.
- **R3** The daemon must survive the panic: the process keeps serving other runs (a panic in a node worker
  must not take the daemon down). If isolation already exists, say where in the report; if it does not, the
  smallest honest form is a catch at the node's boundary with the panic message captured.
- **R4** The run's failure must be visible in the existing counters (`pii metrics`) as a failed run, not as a
  silent in-flight one.
- **R5** A test drives the crash path: a graph/node that panics on purpose, then assert the run reaches
  `Failed` **within a bounded wait with the bound stated in the code** (the finding's gate: within 5 s) and
  that the crash event exists. The test must be RED before the change.

### Non-Functional
- No new dependency. `#![forbid(unsafe_code)]` untouched; clippy `-D warnings` clean.
- The change must not swallow panics that should abort a *worker* process (`pii-worker`): this is about the
  run's state in the daemon, and the report must state which boundary catches and which still aborts.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `a_panicking_node_leaves_a_terminal_run` (new) | a node whose handler panics | run status `Failed` within 5 s; the run's error names the node; the crash event present |
| the same test on the pre-fix tree (scratch copy) | — | RED: the run stays `Accepted`/`Running` past the bound — this is the control that proves the test sees the defect |

## Exit Criteria

- [ ] `cargo test -p pii-server --lib 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-server --lib crash 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo clippy -p pii-server --all-targets -- -D warnings 2>&1 | tail -1 | grep -qv 'error'`
- [ ] the new event name is registered: `grep -rn 'RUN_.*CRASH\|NODE_.*PANIC' crates/pii-observe/src/names.rs | head -1` is non-empty

## Guardrails

- Do NOT paper over the panic by marking the run failed **without** recording the panic's message and event:
  the record is the point (R2).
- Do NOT make the daemon's whole run loop exit on a panic, and do NOT catch panics around code where the
  panic means "this process must die" — if the boundary is ambiguous, **STOP and report** rather than
  choosing silently.
- Do NOT weaken existing tests, and do NOT raise a timeout to make the new test pass.
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit; scratch under `<scratch>/`.

## Closure

(outcome, the RED before, the live `pii run get` answer after, the event name and the counter — filled in when
the item is closed.)

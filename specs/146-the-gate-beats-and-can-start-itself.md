# Spec: the gate beats, and the agent can start the gate that gates it

## Overview

Two design gaps filed 2026-10-10 in `_docs/05-operations/open-items.md` §5, both about our own gate rather than
about pii's code: (a) the gate has **no heartbeat**, so a long step and a hung step look identical until the
timeout; (b) the agent **cannot bootstrap the gate that gates it** — the thing that authorises the work needs
the work to already exist.

**Project**: <repo>
**Phase**: the gate's own robustness (pre-video backlog, specs 145–148)

## Requirements

### Functional

R1. While any gate step runs, `quality-gate.sh` writes a heartbeat line to its own output at least every 30 s:
    the step name, elapsed seconds, and the step's own last line of output. A watcher must be able to tell
    "still working" from "hung" without reading the whole log.
R2. The step list has **one source of truth**. The local run and the `fresh-gate` export run the same list, read
    from the same place. Adding a step in that one place must change both paths.
R3. A step with no heartbeat for longer than the gate's stated limit is reported as such: the failure output names
    the step, the seconds since its last heartbeat, and its last output line. It must never be reported as a
    success or as a plain timeout.
R4. `cargo xtask fresh-gate` stays the documented way for an agent to start the gate that gates it, and it must
    not be possible to run the gate's steps in a way that skips any of them.

### Non-Functional

R5. No new dependency. The heartbeat must not change what the gate returns: a step's exit code remains the only
    verdict.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| a_long_step_emits_heartbeats | a step that sleeps without printing | at least one heartbeat line with the step name and elapsed < 30 s |
| the_step_list_has_one_source | a step added in the one place | both the local and the export path run it |
| a_hung_step_is_named | a step killed mid-run | failure output names the step, the heartbeat gap, and its last line |
| a_heartbeat_is_not_a_verdict | a step that heartbeats then exits non-zero | the gate fails, rc != 0 |

## Exit Criteria

- [ ] `cargo xtask fresh-gate` on this spec's commit is `ALL PASS`
- [ ] `bash scripts/checks/gate-heartbeat.sh` prints a heartbeat block and exits 0 (the harness this spec adds)
- [ ] `grep -c 'quality-gate.sh' xtask/src/*.rs | grep -v ':0' | wc -l` is at least 1 (the export really calls the gate, it is not re-implemented)
- [ ] a mutation of the step list in its one place is observed by both paths (the mutation is recorded in the closure)

## Guardrails

- Do NOT weaken, skip or reorder a step to make the heartbeat easier.
- Do NOT let a heartbeat be mistaken for a step's result (R5) — that is the false-green class this project has paid
  for twice.
- Do NOT invent a second gate implementation in the export path; call the one that exists.
- Do NOT touch `/tmp`, `/dev`, `/proc`, `/sys`, `/root`; scratch under `<scratch>/`.

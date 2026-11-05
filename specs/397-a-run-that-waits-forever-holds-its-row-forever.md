# Spec: a run that waits forever is a run that holds its row forever

## Overview
Every loop in pii is bounded. `max_steps` (1000), `max_wall_ms` (600 s) and the budget are checked between
steps, and a cycle inside one graph must declare `loop_budget` or the graph is refused (V3). A cycle
ACROSS graphs — A calls B calls A — is refused by the installer's closure check as of spec 397.

What none of that covers is the run that does not loop: the one **blocked inside a node**. A `gate`
awaiting an approval that nobody gives, an `agent` turn whose provider never answers, a `tool` running a
command that never exits. The limits are checked at the top of each tick, so a run sitting inside a node
passes no tick and touches no limit. It stays `suspended` — or `running` — forever, and it holds its
queue row: the row is claimed, so no other worker may take it, and no alarm fires.

Measured 2026-11-05: six `gate` nodes ship, none carries a deadline; `pii queue ls` has no column that
distinguishes "waiting for a human" from "stuck".

**Project**: /projects/pi-improved
**Phase**: operations (the queue must not be lost to a silence)

## Requirements
### Functional
- R1 a `gate` node declares how long it may wait (`deadline_ms`); a gate without one is refused at load,
  naming the node — a suspension with no end is a decision nobody made.
- R2 a run suspended past its declared deadline fails with `wait_deadline_exceeded`, naming the node and
  the deadline it passed; it does not stay suspended and it does not silently succeed.
- R3 failing so releases the queue row: the row returns to `pending` with its session and attempts
  intact, and the reason is recorded (`spec 323`).
- R4 `pii queue ls` marks a suspended run with how long it has been suspended and against which
  deadline, so an operator can see the difference between waiting and stuck.

### Non-Functional
- R5 the check runs on the daemon's own tick, so a killed client cannot leave a deadline unenforced.
- R6 no shipped graph gains a deadline larger than its own `max_wall_ms`.

## TDD Contract
| Test name | Input | Expected |
|---|---|---|
| `a_gate_without_a_deadline_is_refused` | graph with `kind = "gate"`, no `deadline_ms` | load refused, error names the node |
| `a_gate_with_a_deadline_loads` | same with `deadline_ms = 60000` | loads |
| `a_run_suspended_past_its_deadline_fails_and_names_it` | gate, deadline 50 ms, never approved | Failed(`wait_deadline_exceeded`), node named |
| `CONTROL_a_run_approved_inside_the_deadline_resumes` | approved at 10 ms | Success, no failure event |
| `the_row_returns_to_the_queue_with_its_session` | suspended row past deadline | status `pending`, `session` unchanged, `attempts` unchanged |

## Exit Criteria
- [ ] `grep -q 'deadline_ms' crates/pii-server/src/node_kinds/nodes/gate.rs`
- [ ] `cargo test -q -p pii-server --test gate_deadline 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `./target/debug/pii queue ls 2>&1 | grep -qi 'suspended'`

Why this section has fewer criteria than the requirements imply: `sdd`'s contract step runs every
criterion BEFORE any code exists and refuses a row if one already passes (measured 2026-11-05: eight
dispatches refused, none spent). A criterion that passes today is a regression guard, not a test of the
new behaviour, so the guards are stated under Guardrails and enforced by the gate.

## Guardrails
- The closure check `python3 deploy/graph-closure.py graphs $(sed -n '/^spec$/,$p' graphs/minimal.txt | tr '\n' ' ')`
  must stay green: it used to be a criterion and already passed.
- Do NOT fail a suspended run on `max_wall_ms` alone: a run may legitimately wait days for a human, and
  the graph author must choose that number (`deadline_ms`), not inherit one.
- Do NOT delete the suspension state on failure: the row keeps its session, and the operator can resume.
- Do NOT treat "no deadline" as "infinite by default": that is the state this spec exists to remove.

## Files
- `crates/pii-server/src/node_kinds/nodes/gate.rs`
- `crates/pii-server/src/engine/runs.rs`
- `crates/pii-cli/src/cmd_queue.rs`
- `graphs/starter/boss-watch.toml`
- `graphs/starter/heal-fix.toml`
- `graphs/starter/heal-watch.toml`
- `graphs/code-index.toml`

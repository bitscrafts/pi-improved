# Spec: every agent call is observable — identity, argv, pid, duration, verdict

## Overview
User directive (2026-10-19): "observability on the pii agent call is a must". A coder node spawns a process that costs
money and edits a tree; today a run that succeeds and a run that never started can both leave a green report, because the
call itself is not an event. The 0.1.1 bar depends on readings nobody can take without it.

**Project**: /projects/pi-improved

## Requirements
1. One event per dispatch, named constants in `pii-observe/src/names.rs`:
   `coder.spawned{agent, model, image_digest, argv0, argv_len, pid, session}` and
   `coder.exited{pid, rc, dur_ms, stop_reason, criteria_passed, criteria_failed}`.
2. `argv` is journalled as a DIGEST plus the argv0 — never the full argv when it can carry a secret; env is passed by
   NAME only and the journal records the names, never the values.
3. A refusal is an event before it is an error: `worker.start.refused{agent, model, reason}` with NO pid, routed to the heal port.
4. `pii run trace <run_id>` renders one line per node with: kind, start/end ms, pid if any, image digest if any, criteria
   counts, and the first failing criterion NAME. `pii run explain <run_id>` adds one paragraph built from those events only.
5. Health reports identity, not liveness: version, graph digest, store path, armed run count, and the number of agent calls
   in flight (`agents_in_flight`), so a daemon that lost its schedule cannot read as healthy.
6. Counters on `pii status`: dispatches, refusals, orphans (pid gone with no `coder.exited`), and criteria failures.

## Exit Criteria
- [ ] `cargo test -p pii-server --test coder_call_is_observable`
- [ ] `cargo test -p pii-server --test refusal_precedes_the_spawn`
- [ ] `cargo test -p pii-cli --test run_trace_names_the_call`
- [ ] `grep -c 'api_key\|API_KEY=' crates/pii-observe/src/names.rs` is 0 (no secret names leak into event fields)
- [ ] a live run on lnx shows `coder.spawned` then `coder.exited` with the SAME pid in both

## Guardrails
- Do NOT log argv values when an argv element came from an env file; digest + length only.
- Do NOT emit an exit event for a process that never started (that is what `worker.start.refused` is for).
- Do NOT let trace read the worker's own report: trace is built from the journal, so a lying worker cannot colour it.
- An orphan (spawned, no exit) must be countable, not invisible.

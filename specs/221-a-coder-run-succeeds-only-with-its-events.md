# Spec: a coder run succeeds only if its agent-call events exist

## Overview
Measured 2026-10-20: `pii run start coder-selffix` returned `{"run_id":"r000000004481"}` and the run reported
`{"dur_ms":115,"status":"succeeded"}`. A real agent dispatch cannot complete in 115 ms; the outer run was green while no
agent was spawned. That is the defect class this project has paid for repeatedly: a success that proves nothing.

## Requirements
1. A run whose graph contains a `coder` node is SUCCEEDED only if that node's `coder.spawned` AND `coder.exited` events are
   present in the journal for the same pid. Otherwise the run is `failed` with `reason = coder.no_call_events`, naming the node.
2. A refusal is a legitimate terminal state: `worker.start.refused` (no pid) makes the run `refused`, a distinct status from
   failed and from succeeded.
3. The duration is not a criterion — a fast run is not proof of anything, and a slow one is not proof of work.
4. `pii run get <id>` prints, for each coder node: `spawned=<pid|none> exited=<rc|none> refused=<reason|none>`.
5. The graph may declare `expect_spawn = false` for a deliberately refusing demo; the run is then `refused` and says so.

## Exit Criteria
- [ ] `cargo test -p pii-server --test coder_run_needs_its_events`
- [ ] `cargo test -p pii-server --test refusal_is_its_own_status`
- [ ] `cargo test -p pii-cli --test run_get_names_the_coder_call`
- [ ] a run of graphs/coder-selffix reports status succeeded/refused ONLY with the matching events present

## Guardrails
- Do NOT infer a spawn from a node's own output; the journal is the only source.
- Do NOT collapse `refused` into `failed`: the operator must be able to tell "it would not start" from "it started and broke".
- Do NOT weaken the check to make an existing demo graph pass.

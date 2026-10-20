# Spec: a fragment-woven graph can actually be STARTED

## Overview
Measured 2026-10-20 on the deployed revision. `pii run start coder-demo` refuses with:

    error: -32603: invalid: fragment.attach.unwoven: this graph attaches fragments; load it through fragment::weave
           (pii graph check), never directly

Spec 212 (fragments) added a load-time weaving requirement, and the daemon's run-start path does not weave. The result is a
graph that lints, checks and explains — and cannot run. That is worse than a graph that refuses to load: the operator reads
"valid" and then cannot start it.

## Requirements
1. `pii run start <graph>` weaves a graph that declares `[[attach]]` BEFORE it validates or runs it: the run executes the
   woven form, and the journal records each woven node with `woven-from: <fragment>#<node>`.
2. The pin is verified before the spawn of anything: a drifted fragment is `fragment.pin.drift` and the run is REFUSED.
3. `pii graph check <graph>` and the daemon's weave must share ONE implementation — not two, so "check passed" and "run
   started" can never disagree.
4. `pii run trace <id>` prints the woven nodes with their origin, so the graph you read equals the graph that ran.
5. A graph with no `[[attach]]` is unaffected: byte-identical behaviour to today.

## Exit Criteria
- [ ] `cargo test -p pii-server --test woven_graph_can_start`
- [ ] `cargo test -p pii-server --test woven_pin_drift_refuses`
- [ ] `cargo test -p pii-cli --test trace_names_woven_nodes`
- [ ] `pii run start coder-demo` returns a run id on the deployed daemon
- [ ] `pii run trace <id>` lists at least one node with `woven-from: podman-guard`

## Guardrails
- Do NOT relax `fragment.attach.unwoven` into a warning: the refusal is right, the missing weave is the defect.
- Do NOT weave at graph AUTHORING time only — a fragment edited after the graph was written must still be pinned and verified.
- Do NOT let the weave path bypass the capability scope: woven nodes inherit the run's grants and nothing more.

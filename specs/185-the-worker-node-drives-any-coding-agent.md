# Spec: the worker node drives any coding agent

## Overview

`worker` exists as a node kind but is bound to no coding agent: nothing in `crates/` spawns `pi` or `claude`, even though
`pi 0.2.0` sits installed at `~/.local/bin/pi` and the autonomy loop has been driving `claude --resume … -p` from shell
scripts all along.

Directive (user, 2026-10-18, verbatim): **"that graph should use any coding agent, pi or claude (and set its model). it
should learn and heal, sdd+tdd approach we are using so far."** The Claude account is limited; pi is free. A heal or
exercise graph must therefore pick its worker per node — agent AND model as data — and learn from the outcome.

**Project**: <repo>

## Requirements

### Functional

1. A `worker` node carries `agent = "pi" | "claude"`, `model = "<id>"`, `spec = "<path>"`, `workdir = "<path>"`.
2. `agent = "pi"` spawns `pi --session <file> -p <brief>` — resumable; never `--session-dir` with a fresh start.
   `agent = "claude"` spawns `claude --resume <sid> -p <brief>`, or a first dispatch with no sid.
3. The node's RESULT is the worker's own report **plus the run of its criteria**. A worker that compiles but fails a
   criterion is a FAILED node — the defect that produced rows closing over uncommitted work.
4. The model is DATA, not code: changing `model` changes the billed provider without a rebuild.
5. An unavailable provider (no credentials, rate limited, no session) produces a REFUSAL EVENT naming agent and model —
   never a silent empty result.
6. `learn`: each outcome (spec, agent, model, verdict, failing criterion) is written to memory, so a repeated failure reads
   as a pattern instead of a fresh surprise.
7. `heal`: a heal graph may choose its worker per attempt from a LADDER — pi with a free model first, Claude last — and
   escalates only on evidence from the previous attempt.

### Non-Functional

- No credentials in the graph file; they come from the environment file and are never echoed.
- Every dispatch obeys HARD RULE 0d: the spawn goes through a named script that sets PATH and provider env **inside** it.
- Sessions are re-attachable: a worker killed mid-run resumes; it does not start over.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| worker selects pi | `agent = "pi"`, model `deepseek-v4-flash` | argv begins `pi --session`; no `claude` anywhere |
| worker selects claude | `agent = "claude"`, model `haiku-5-5` | argv begins `claude --resume` or `claude -p`, with `--model` |
| model is data | two runs, different `model` | two different argv, one binary |
| missing credentials | `agent = "claude"`, empty env | `worker.start.refused` event, no spawn |
| criteria decide | worker exit 0, one criterion rc 1 | node FAILED, reason names that criterion |
| learn | a FAILED node | one memory item per outcome, carrying agent and model |
| escalate | ladder `[pi-free, claude]` | second attempt only after the first's evidence — never both at once |

## Exit Criteria

- [ ] `cargo test -p pii-server --test worker_agent_selection`
- [ ] `cargo test -p pii-server --test worker_criteria_decide`
- [ ] `grep -q 'worker.start.refused' crates/pii-observe/src/names.rs`

## Guardrails

- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT hardcode an agent or a model in Rust: both are graph data.
- Do NOT spawn an agent without a script that sets its own environment.
- Do NOT let a worker commit; the orchestrator commits.

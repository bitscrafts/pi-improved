# Spec: the coder node spawns an agent

## Overview
User directive 2026-10-19: "i want to see pii managing agents by today. this should be a point where we increase the version minor (when fully functional)." Measured state: agent_worker.rs builds argv for pi/claude; nothing spawns; the kind is declared (nodes/mod.rs:7) but not registered; and 074 found seven blockers between the plan and a real dispatch (M1-M7). This spec is the row that closes them, in one pass, with the reading that proves each.

**Project**: <repo>

## Verified premises (074, read from the tree)
- `pii-core/src/childproc.rs` `run_child` does `.env_clear().envs(scrub(..))` — every `*_API_KEY` is stripped, so a child gets no credentials unless the caller passes them.
- `run_scope.rs:113` pins `proc_argv0` to `["/bin/sh"]` — the run's capability scope refuses an agent binary.
- `worker` is already a node kind; a second kind must have a different name (074 proposes `coder`).
- The scheduler RETRIES a retryable `Err`, so a refusal must be a PORT, not an Err.
- Spec 143's resume re-runs an incomplete node — a coder node killed mid-edit would be dispatched twice unless it is marked effectful.

## Requirements
### Functional
1. A node kind **`coder`** (not `worker`), registered, with config: `agent = "pi"|"claude"`, `model`, `brief`, `workdir`, `spec`, `env_pass = [NAME,...]`.
2. The node SPAWNS the agent directly: `env_clear()`, then only the names in `env_pass` copied from the daemon's own environment (values never logged, never in the graph).
3. The capability scope admits the agent argv: `proc_argv0` must allow `~/.local/bin/{pi,claude}` for a coder node, and nothing else new.
4. The session handle is recorded on the run BEFORE the spawn, so attempt 2 resumes (`--session <file>` / `--resume <sid>`) instead of restarting.
5. The node's verdict is the run of its criteria, not the worker's exit code: criteria run after the spawn, and the first non-zero makes the node FAILED naming that criterion.
6. Refusal (missing credentials, unknown agent, no spec) emits `worker.start.refused` naming agent+model and routes to an `err` PORT — never an `Err` that the scheduler retries.
7. A coder node is marked EFFECTFUL so resume does not re-dispatch a killed node (spec 143/184 semantics).

### Non-Functional
- No secret in the graph, in the journal or in the log: only names are passed.
- One reading per requirement, printed by the run: pid, argv, session, criteria rcs.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| coder_spawns_pi | agent=pi, echo in place of pi | journal has `coder.spawned{pid,argv}`; argv begins `~/.local/bin/pi` |
| scope_admits_agent | coder node in a run scope | spawn succeeds; a non-coder node trying the same argv is DENIED |
| env_passes_names_only | env_pass=["PII_TEST_KEY"] | child sees that var; every other `*_KEY` is absent |
| session_recorded_before_spawn | a spawn that hangs 2 s | `pii run get` shows worker_session while the child is alive |
| criteria_decide | child exits 0, criterion rc 1 | node FAILED, reason names the criterion |
| refusal_is_a_port | no credentials | `worker.start.refused`, port `err`, no pid |
| resume_does_not_redispatch | kill -9 mid-spawn, resume | zero additional spawns for that node |

## Exit Criteria
- [ ] `cargo test -p pii-server --test coder_spawns_a_process`
- [ ] `cargo test -p pii-server --test coder_scope_admits_agent`
- [ ] `cargo test -p pii-server --test coder_env_passes_names_only`
- [ ] `grep -q 'coder.spawned' crates/pii-observe/src/names.rs`
- [ ] `cargo run -q -p pii-cli -- graph check graphs/coder-demo.toml`  (loads, one coder node, no refusal)

## Guardrails
- Do NOT reuse the name `worker` for the agent-driving kind.
- Do NOT weaken run_scope for non-coder nodes.
- Do NOT let the worker's report decide success.
- Do NOT commit; the orchestrator commits.

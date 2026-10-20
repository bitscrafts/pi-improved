# Spec: an agent call is observable — identity, pid, digest, duration, verdict, orphans

## Verified Premises

Measured on 2026-10-19 at HEAD `85d599b` (commands and outputs pasted, not paraphrased):

1. CONFIRMED — the spawn point already journals a pid.
   `crates/pii-server/src/node_kinds/nodes/coder.rs:290` → `let pid = child.pid();`, then `.field("pid", i64::from(pid))`.
   So `coder.spawned` carries a pid TODAY.

2. CONFIRMED — the event names exist as constants.
   `crates/pii-observe/src/names.rs:68` `CODER_SPAWNED = "coder.spawned"`
   `:66` `WORKER_START_REFUSED = "worker.start.refused"`, `:70` `CODER_IMAGE_MISSING = "coder.image.missing"`,
   `:72` `CODER_HOST_SPAWN_DEPRECATED = "coder.host_spawn.deprecated"`.

3. CONFIRMED — there is NO exit event.
   `grep -c 'CODER_EXITED' crates/pii-observe/src/names.rs` → 0. A run that spawned and a run that exited are
   distinguishable only by reading the node's own report, which is exactly what the loop refuses to trust.

4. CONFIRMED — the journal has one append path.
   `crates/pii-graph/src/journal.rs:200` `pub fn append(&self, event: &RunEvent) -> Result<u64, PiiError>`.

5. CONFIRMED — the operator surface already has the verbs, with thinner content.
   `crates/pii-cli/src/main.rs:273` `"trace" => cmd_trace::cmd_trace(args)`;
   `crates/pii-cli/src/cmd_health.rs:70` already prints `"armed"` and per-status counts at `:80`.

6. CONFIRMED — a refusal exists before a spawn for images, but not as a *pre-spawn event with no pid* in the general case.
   `coder.rs:123` `fn deferred_verdict(code, pid, agent, model)` returns `{"deferred": true}` — a verdict recorded before
   criteria ran, i.e. the shape this spec must make explicit rather than implicit.

7. INFERRED — cost/tokens per call are not available uniformly across agents (`claude -p` reports `total_cost_usd`,
   `pi` does not). Therefore cost is OPTIONAL and must never gate a node's verdict; duration and rc are mandatory.

**Project**: `/projects/pi-improved`
**Depends on**: 207 (coder node), 209 (podman, no installs), 215 (criteria are named, not shell)

## Overview

A coder node spends money and edits a working tree. Tonight's own failures show why the call must be an event: a
worker that never started and a worker that finished were both reported as "the cycle ran", and a criterion that never
executed passed as green. Observability here is not dashboards — it is the instrument the 0.1.1 bar is measured with:

- bar reading 2 (`journal shows coder.spawned{pid, argv}`) needs the event to carry agent, model, argv identity;
- reading 3 (a failing criterion turns a green report red) needs `criteria_failed` on the exit event;
- reading 5 (kill mid-spawn → resumve → ZERO additional spawns) needs an ORPHAN counter, because a second spawn and a
  resumed run look identical without one.

## Requirements

### Functional

R1. Two new event constants in `crates/pii-observe/src/names.rs`:
    `CODER_EXITED = "coder.exited"` and `CODER_ENV_NAMES = "coder.env.names"` (see R3).

R2. `coder.spawned` fields, all required except where marked:
    `agent` (string), `model` (string), `image` (string, tag or digest as declared in the graph),
    `image_digest` (string, empty when the runtime cannot report one), `argv0` (string), `argv_len` (int),
    `argv_digest` (hex string), `pid` (int), `session` (string, empty when the agent has no session concept).
    Existing fields keep their names so today's readers keep working.

R3. NEVER a secret in the journal: `argv` is recorded as `argv_digest` + `argv0` + `argv_len` only. Environment is
    declared by NAME in the graph and journalled as `coder.env.names` with the names joined by `,` and NO values.
    A field whose name matches `(?i)key|token|secret|password` must not appear with a value anywhere in a run event;
    the node refuses to journal it (`coder.env.refused`) instead of redacting silently.

R4. `coder.exited` fields, all required:
    `pid` (int, must equal the pid in the matching `coder.spawned`), `rc` (int), `dur_ms` (int),
    `stop_reason` (string: `end_turn` | `timeout` | `killed` | `signalled` | `unknown`),
    `criteria_run` (int), `criteria_passed` (int), `criteria_failed` (int),
    `first_failure` (string, the NAME of the first failing criterion, empty when none),
    `cost_usd` (float, OPTIONAL — omitted when the agent does not report it).

R5. A refusal is an event BEFORE it is an error: `worker.start.refused` is emitted with `agent`, `model`, `reason`,
    and **no pid**, and the node routes it to the heal port. A refused dispatch must never emit `coder.exited`.

R6. `pii run trace <run_id>` — one line per node, in journal order:
    `<seq> <node> <kind> <start_ms>+<dur_ms> pid=<pid|-> image=<digest8|-> crit=<passed>/<run> fail=<name|-> rc=<rc|->`
    and a final summary line `trace: <n> nodes · <k> agent calls · <o> orphans`.
    Built ONLY from journal events; the node's own report is never read by trace.

R7. `pii run explain <run_id>` — one paragraph derived from the same events: what ran, which agent and model, which image
    digest, how long, what the criteria said, and — if it failed — the first failing criterion by name.

R8. `pii health` reports IDENTITY, not liveness. Added to the existing output (do not remove `armed` at `cmd_health.rs:70`):
    `version`, `graph_digest`, `store_path`, `armed` (exists), `agents_in_flight`, `orphans_since_start`.

R9. `pii status` counters: `dispatches_total`, `refusals_total`, `orphans_total`, `criteria_failures_total`.
    An ORPHAN is a `coder.spawned` whose pid has no `coder.exited` in the same run when the run reaches a terminal state.

R10. `agents_in_flight` is derived from live journal state (spawned − exited), not from a counter that can drift.

### Non-Functional

N1. Zero new dependencies. Events go through `journal.append` (`journal.rs:200`); no new transport.
N2. Event field names are additive and stable; renaming an existing field requires a spec.
N3. Writing an event must not fail a run: a journal error is logged and the node continues (the run's work matters more
    than its log), except for `coder.spawned`, which must succeed BEFORE the child is waited on — a spawn nobody can
    see is the defect this spec exists to remove.

## Architecture

```
  graph node kind "coder"
     │
     ├─ plan: agent, model, image, env_pass(names only)            ← from the graph, never hardcoded
     ├─ refuse?  ──► worker.start.refused{agent,model,reason}      ← no pid, routed to the heal port
     │
     ├─ journal coder.spawned{agent,model,image,image_digest,
     │                        argv0,argv_len,argv_digest,pid,session}
     ├─ journal coder.env.names{names}                             ← names, never values
     │
     ├─ wait(child)  ──► rc, dur_ms, stop_reason
     ├─ run criteria (spec 215: NAMED checks, no shell)
     │
     └─ journal coder.exited{pid,rc,dur_ms,stop_reason,
                             criteria_run,criteria_passed,criteria_failed,
                             first_failure,cost_usd?}

  readers (no worker self-report anywhere in this path):
     pii run trace    ─┐
     pii run explain  ─┼─► journal only
     pii health       ─┤   version · graph_digest · store_path · armed · agents_in_flight · orphans
     pii status       ─┘   dispatches · refusals · orphans · criteria_failures
```

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `coder_call_is_observable` | coder node with a stub child that exits 7 | `coder.spawned` then `coder.exited` with the SAME pid, `rc=7`, `dur_ms>0` |
| `refusal_precedes_the_spawn` | node with an image that is absent | one `worker.start.refused` with `reason`, NO `coder.spawned`, NO `coder.exited`, and the heal port receives it |
| `spawned_event_precedes_the_wait` | journal that fails on the SECOND append | the run FAILS before the child is waited on (a spawn nobody can see must not proceed) |
| `argv_is_digested_not_quoted` | argv containing `--api-key sk-live-XYZ` | journal holds `argv_digest`, `argv0`, `argv_len`; the literal `sk-live-XYZ` appears in NO event |
| `env_names_only` | env_pass `["ANTHROPIC_API_KEY"]` | `coder.env.names` holds the NAME; no event holds a value |
| `criteria_counts_are_named` | two criteria, the second failing | `criteria_failed=1`, `first_failure` equals that criterion's name |
| `trace_names_the_call` (pii-cli) | a recorded run | trace prints pid, image digest, `crit/<run>`, `fail=<name>`, and a summary with orphan count |
| `orphan_is_countable` | run with `coder.spawned` and no exit, then terminal | `orphans_total` increments by 1 and health shows `orphans_since_start` |
| `agents_in_flight_is_derived` | spawned, then exited | `agents_in_flight` is 1 between the events and 0 after, computed from events not a counter |

## Exit Criteria

- [ ] `cargo test -p pii-server --test coder_call_is_observable`
- [ ] `cargo test -p pii-server --test refusal_precedes_the_spawn`
- [ ] `cargo test -p pii-server --test coder_secret_never_journalled`
- [ ] `cargo test -p pii-cli --test run_trace_names_the_call`
- [ ] `cargo test -p pii-server --test orphan_is_countable`
- [ ] `grep -c 'CODER_EXITED' crates/pii-observe/src/names.rs` is 1
- [ ] `cargo run -q -p pii-cli -- health` prints `agents_in_flight` and `orphans_since_start`
- [ ] a live run on lnx: `pii run trace <id>` shows one line with `pid=P image=D` and the summary line `trace: … orphans: 0`

## Guardrails

- Do NOT journal an env VALUE, ever — names only, and a sensitive name is refused rather than redacted.
- Do NOT emit `coder.exited` for a process that never started; that is `worker.start.refused`.
- Do NOT derive `agents_in_flight` from an in-memory counter — derive it from the journal so a restart cannot lie.
- Do NOT let `trace` or `explain` read the worker's report: they are journal-only, by construction.
- Do NOT make cost mandatory; an agent that does not report cost is normal (`pi`), and cost must never gate a verdict.
- Do NOT add a telemetry dependency, a metric server, or a new port. This is journal + two CLI verbs.

## Files to Modify

**Create**
- `crates/pii-server/tests/coder_call_is_observable.rs`
- `crates/pii-server/tests/refusal_precedes_the_spawn.rs`
- `crates/pii-server/tests/coder_secret_never_journalled.rs`
- `crates/pii-server/tests/orphan_is_countable.rs`
- `crates/pii-cli/tests/run_trace_names_the_call.rs`

**Modify**
- `crates/pii-observe/src/names.rs` — add `CODER_EXITED`, `CODER_ENV_NAMES`, `CODER_ENV_REFUSED`, metric names
- `crates/pii-server/src/node_kinds/nodes/coder.rs` — emit `coder.exited`; digest argv; env names event; refusal before spawn
- `crates/pii-cli/src/cmd_trace.rs` — per-line call detail + summary with orphan count
- `crates/pii-cli/src/cmd_health.rs` — `version`, `graph_digest`, `store_path`, `agents_in_flight`, `orphans_since_start`
- `crates/pii-cli/src/cmd_sys.rs` — the four counters on `status`

**Forbidden**
- anything under `specs/`, `deploy/`, `/root/.config/pii/` — a worker never edits its own contract or deployment
- a new crate, a new dependency, a metrics endpoint, or a log shipper

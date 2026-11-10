# Spec: 244 — the agent's session id is chosen by pii and journalled before the agent starts

**Project**: <repo>
**Phase**: filed 2026-10-23T11:40Z by the orchestrator (defect 21 in `_docs/05-operations/agent-management-findings-2026-10-23.md`).
Premises read, not assumed:

- A finished run is never resumed (spec 196, `runs.rs` `resume` refuses `Succeeded|Failed|Refused` with
  `resume <id>: refused (run already finished: <status>)`), and a run with an effect started and never confirmed is refused too
  (spec 186, `pii_graph::effect::refusal`). Both are deliberate and stay. After a crash, `reconcile_boot` closes the run `Failed`.
  So the way to continue interrupted agent work is a NEW run that continues the agent's own session — which needs the session
  handle to survive the crash.
- pi: `coder.spawned` already journals `session` (`session_of(&s.argv)`, `coder.rs:496`) — the session file path, which survives.
- claude: the session id is chosen by claude and appears only on the stream's `init` line. `run_facts` reads it on
  `coder.exited` (`claude_session_id`), which a crash prevents. A run that dies mid-agent therefore journals NO claude handle.
- `claude --session-id <uuid>` exists (checked with `claude --help`, no call made): the caller picks the id. The existing
  `agent_claude::argv(sid, model, max_budget_usd, brief)` already emits `--resume <sid>` when `sid` is set.
- `agent_worker` already has `uuid_v4()` (OS random source) used for the pi session header.

## Overview

For claude, pii will generate the session id itself, pass it as `--session-id`, and journal it in `coder.spawned` before the
process starts. After any crash the journal then names the session, and the operator (or a follow-up row) can start a new run
with `sid = <that id>`, which the claude arm already turns into `--resume`. No run is resumed against the rules above.

## Requirements

- **R1** `agent_claude::argv` gains the fresh-session form: when the node has no `sid` (a first run) and the caller supplies a
  `new_session` id, the argv carries `--session-id <new_session>`; when `sid` is set the argv carries `--resume <sid>` as today
  and NO `--session-id` (the two flags are never both emitted).
- **R2** The coder generates the id with the existing `uuid_v4` for a claude node with no `sid`, passes it through, and the id is
  the one in the argv. A pi node is unaffected.
- **R3** `coder.spawned` carries the claude session id in its existing `session` field (today empty for claude, because
  `session_of` reads pi's `--session`). For a `--resume <sid>` run it carries `<sid>`.
- **R4** `session_of(argv)` recognises `--session` (pi), `--session-id` and `--resume` (claude), in that order of the argv.
- **R5** The id is a valid lowercase hyphenated UUID v4; a malformed `sid` supplied by the graph is refused before spawn with a
  reason naming the field (a bad handle must not reach `claude`).
- **R6** `coder.exited.claude_session_id` still reports what claude printed; a test asserts it equals the journalled one for a
  normal run (a mismatch is a finding).

## Architecture

A pure function `fresh_session_id() -> String` is NOT added; reuse `uuid_v4`. `agent_claude::argv` takes one more parameter
(`new_session: Option<&str>`); `agent_worker::plan` supplies it. The journalled value comes from `session_of`, so there is one
source of truth: the argv that was actually spawned.

Rejected: parsing the id from the stream while the agent runs (needs incremental capture the coder does not have); a sidecar file
(a second place that can disagree with the argv).

## TDD Contract

Extend `crates/pii-server/tests/agent_claude_arm.rs` is NOT allowed (existing tests stay as they are); new file
`crates/pii-server/tests/agent_claude_session_handle.rs`.

| test | given | expects |
|---|---|---|
| `a_first_claude_run_carries_a_session_id` | no `sid`, `new_session = Some(id)` | argv has `--session-id` then `id` |
| `a_resumed_claude_run_carries_resume_and_no_session_id` | `sid = Some(x)` | argv has `--resume x` and no `--session-id` (CONTROL) |
| `the_two_flags_are_never_both_present` | every combination of sid/new_session | never both |
| `the_brief_is_still_last` | any | last element is the brief (CONTROL) |
| `pi_never_gains_the_flag` | pi node | no `--session-id` (CONTROL) |
| `session_of_reads_claude_flags` | argv with `--session-id u`, with `--resume u`, with pi `--session f` | `u`, `u`, `f` |
| `session_of_an_argv_without_one_is_empty` | argv with none | empty (CONTROL) |
| `a_malformed_sid_is_refused_before_spawn` | `sid = "../etc"` | refused with a reason naming `sid`; no `coder.spawned` |
| `the_spawned_event_names_the_session` | stand-in claude run through the node | `coder.spawned.session` is a UUID and equals the id the stand-in received on argv |
| `the_exited_event_agrees` | same run, stand-in prints the same id on its init line | `coder.exited.claude_session_id` equals the spawned one |
| `a_run_killed_mid_agent_still_names_its_session` | stand-in `hang` mode with `timeout_ms`, no init line printed | `coder.spawned.session` is present although `claude_session_id` is empty |

The requirement a plausible implementation gets wrong is the last row: journalling the id from the stream's init line (as
`claude_session_id` does) leaves a killed run with no handle, which is the defect. The test must read `coder.spawned`.

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test agent_claude_session_handle 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-server --test agent_claude_arm 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-server --test agent_claude_budget 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `grep -q "session-id" crates/pii-server/src/node_kinds/nodes/agent_claude.rs`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`

## Guardrails

- `specs/` are READ-ONLY for the implementer. If `agent_claude::argv` or `session_of` is not shaped as described, STOP and report.
- `agent_claude_arm.rs` and `agent_claude_budget.rs` call `argv` with its current arguments; adding a parameter forces their call
  sites to change. That is the ONE permitted edit to them: add `None` for the new parameter and change nothing else. Say so in the
  Closure. Never weaken an assertion.
- No real `claude` call, no network, no spend: every test uses the stand-in script under `_tmp/`.
- Do not make any run resumable and do not touch `resume`, `reconcile_boot` or the spec 186/196 refusals.
- Tests write under `_tmp/`. 450-effective-line file gate and `#![forbid(unsafe_code)]` apply. Do NOT commit.

## Closure

### The implementer's readings

Left to the orchestrator — the cycle committed `1675120` with `gate green`, and the reading below is the orchestrator's.

### The orchestrator's independent reading (2026-10-23T12:20Z, by hand, not from the worker's report)

Two trees, because they disagree:

**A clean worktree of the commit — `git worktree add --detach _tmp/verify-244-<ts> 1675120`:**
- criterion 4 (`grep -q "session-id" …/agent_claude.rs`): **PASS**.
- criteria 1, 2, 3, 5 (`cargo test` ×3, `cargo clippy`): **NOT MEASURABLE** — the commit does not compile.
  `cargo check -q -p pii-server` reports 7 errors, none of them in this spec's paths (`agent_claude.rs`,
  `agent_worker.rs`, `coder/helpers.rs`, the three test files): `E0425 kill_group`, `E0432 Listed` /
  `coder_runtime_api`, `E0599 list_all` / `force_remove`, `E0050 run` 4 vs 5 parameters. They belong to the API
  specs 241-243 call and left uncommitted (finding defect 24; fix queued as spec 245 / row A113).

**The working tree, which holds that API uncommitted (HEAD's code + that API, `CARGO_TARGET_DIR` separate from the gate's):**
- `agent_claude_session_handle` — **13 passed, 0 failed**
- `agent_claude_arm` — **12 passed, 0 failed** (the spec's permitted change: one `None` for the new parameter)
- `agent_claude_budget` — **7 passed, 0 failed**
- `cargo clippy -q -p pii-server --all-targets -- -D warnings` — **rc 0**
- criterion 4 — PASS

So this spec's content reads green wherever it can compile; what is false is the *commit's* claim, not the code. The
item stays `done` on its own criteria and is **not** verified end to end, because its HEAD cannot be built.

**Owed, not done — the mutation.** The red-before-green mutation this lane owes for a defect fix was NOT run: it
requires the criteria green on the commit first, and at HEAD they cannot compile. It is owed by row A113 (spec 245)
the moment HEAD builds: mutate `let new_session = (spec.agent == Agent::Claude && claude_sid.is_none()).then(uuid_v4);`
(`agent_worker.rs:330`) to `None`, expect `the_spawned_event_names_the_session` and
`a_run_killed_mid_agent_still_names_its_session` red, then restore byte-identical (`sha256sum` before/after, then
`touch`, per the `cp -p` false-RED trap). Recorded here as owed so it is not mistaken for a reading.


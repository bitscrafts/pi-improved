# Spec: 242 — a restart finds the agents a dead daemon left running, and the run says which

**Project**: /projects/pi-improved
**Phase**: filed 2026-10-23T11:20Z by the orchestrator (defects 20 and 21 in
`_docs/05-operations/agent-management-findings-2026-10-23.md`). Stage 2: the daemon runs on the host as a systemd service
(`deploy/pii.service`, restarted by the deploy and by crashes) and the agent runs in a container. A restart is routine there.
Premises read, not assumed:

- `crates/pii-server/src/engine/session_durability.rs` `reconcile_boot` (line ~311): every run left `Running` is marked
  `Failed` with the error `interrupted by daemon restart` and emits `ObsEvent::RunReconciled { run, reason }`. It never
  resumes (G8) and never looks for the run's agent.
- Measured 2026-10-23 on the stand-in: SIGKILL of the daemon during a hung agent left the agent running (pid survived, ppid 1);
  after restart the run was `failed`, `orphan agent procs: 1`, and `pii run resume` answered
  `refused (run already finished: Failed)`.
- Every container the coder starts carries the labels `io.pii=1` and `pii.run=<run id>`
  (`coder_runtime.rs` `create_body`, line ~415), so an orphaned container is findable by label.
- A host-path agent is the child of the daemon's process group (`ChildProc::reap_group`); after a daemon SIGKILL it is
  re-parented to init and nothing owns it.
- `coder.spawned` journals `pid`, `container` and `session`; `claude_session_id` is journalled on `coder.exited` only, i.e.
  never for a run that died mid-agent.

## Overview

A daemon crash today leaves the agent running with no owner, writing to the workdir and spending money, while its run reads as
failed and cannot be resumed. The operator decided (2026-10-23, "proceed with recommendations"): **kill what the dead daemon
started, and say so on the run.** Adopting a live agent was rejected: the daemon that would adopt it has lost the output pipe,
the criteria and the node's position, so it could not honestly finish the node.

This spec adds the kill and the record. It does NOT make the run resumable (spec 243 does).

## Requirements

- **R1** At boot, before `reconcile_boot` closes the runs, a new step `reap_orphans` finds agents belonging to runs that were
  `Running`: (a) containers labelled `pii.run=<id>` for each such run id, listed through the same `ContainerRuntime` the coder
  uses; (b) for the host path, the pid recorded in that run's `coder.spawned` event, only if `/proc/<pid>/cmdline` still
  starts with the recorded `argv0` (a recycled pid is NOT killed).
- **R2** Each found container is force-removed; each found host process has its process group killed (`killpg`, as
  `ChildProc::reap_group` does). Nothing else is touched: a container without the `io.pii` label, a pid whose cmdline differs,
  and a run that was not `Running` are left alone.
- **R3** Each reap is journalled as `coder.orphan_reaped` with fields `run`, `kind` (`container`|`process`), `id`, `result`
  (`killed`|`already_gone`|`failed`). A reap that fails does not stop boot; the failure is the event.
- **R4** The run's failure text becomes `interrupted by daemon restart; agent reaped` when at least one agent was killed for
  it, and stays `interrupted by daemon restart` otherwise. `RunReconciled.reason` carries the same text.
- **R5** `reap_orphans` is idempotent: a second boot over the same store kills nothing and journals nothing new.
- **R6** A run whose `coder.spawned` has no matching `coder.exited` is the only run searched; a run that never reached a coder
  node costs nothing.

## Architecture

`reap_orphans(store, runtime, proc) -> Vec<Reaped>` in a new file `crates/pii-server/src/engine/orphans.rs`, called from the
engine's boot sequence immediately before `reconcile_boot`. It takes the container runtime and a process probe as parameters
(traits-first) so tests inject fakes. The pid check reads `/proc/<pid>/cmdline` through the injected probe.

Rejected: killing every `io.pii=1` container at boot without checking the run (it would kill a healthy run's container if two
daemons share a runtime socket — the topology is unresolved); adopting the agent.

## TDD Contract

New test file `crates/pii-server/tests/orphans_are_reaped.rs`, with fakes for the runtime (a recorded list of containers with
labels) and the process probe (a map pid → cmdline), and an in-memory or `_tmp/` store seeded with runs and `coder.spawned`
events.

| test | given | expects |
|---|---|---|
| `a_running_runs_container_is_removed` | run `r1` Running, container labelled `pii.run=r1` | removed; one `coder.orphan_reaped` `container`/`killed` |
| `a_container_of_another_run_is_left` | container labelled `pii.run=r9`, no such Running run | untouched (CONTROL) |
| `a_container_without_the_pii_label_is_left` | no `io.pii` label | untouched (CONTROL) |
| `a_live_host_agent_is_killed` | `coder.spawned` pid 4242 argv0 `claude`, probe says cmdline starts `claude` | killpg(4242); `process`/`killed` |
| `a_recycled_pid_is_not_killed` | same pid, probe says cmdline `sshd` | NOT killed; `already_gone` (CONTROL) |
| `a_run_with_coder_exited_is_not_searched` | spawned AND exited both journalled | nothing reaped (CONTROL) |
| `a_run_that_never_reached_a_coder_is_free` | Running run, no `coder.*` events | no runtime call at all (CONTROL) |
| `a_failed_reap_does_not_stop_boot` | runtime returns an error for one container | boot continues; one `failed` event; the other run still reconciled |
| `the_failure_text_says_the_agent_was_reaped` | one agent killed | run error is `interrupted by daemon restart; agent reaped` |
| `the_failure_text_is_unchanged_without_an_agent` | nothing killed | run error is exactly `interrupted by daemon restart` (CONTROL) |
| `a_second_boot_reaps_nothing` | boot twice over one store | second pass: zero kills, zero new events |
| `a_real_orphan_process_is_killed` | spawn a real `sleep 100000` in its own group from the test, record it as `coder.spawned`, run `reap_orphans` with the real probe | the process is gone within 2 s; a second real `sleep` NOT recorded is still alive |

The requirement a plausible implementation gets wrong is R1(b): killing the recorded pid without checking `/proc/<pid>/cmdline`
kills whatever reused the pid. The recycled-pid row catches it, and the last row proves the kill is real and not only recorded.

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test orphans_are_reaped 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `grep -q "reap_orphans" crates/pii-server/src/engine/session_durability.rs`
- [ ] `grep -q "orphan_reaped" crates/pii-observe/src/names.rs`
- [ ] `cargo test -q -p pii-server --lib reconcile 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`

## Guardrails

- `specs/` are READ-ONLY for the implementer. If `reconcile_boot` or the boot sequence is not shaped as described, STOP and
  report the mismatch.
- Never weaken an existing reconcile test; the two that assert the exact text `interrupted by daemon restart`
  (`reconcile_tests.rs`, `tests/session.rs`) must stay green unchanged — the new text appears only when an agent was killed.
- Real processes in tests are `sleep`, started by the test, in their own group, and killed by the test in a `Drop` guard even
  when an assertion fails. A test that leaves one behind is a defect.
- Never kill a pid without the cmdline check. Never touch a container without the `io.pii` label.
- Add the new event name to `crates/pii-observe/src/names.rs` like the existing ones (the observability lint checks it).
- Tests write under `_tmp/`. Do NOT commit; the orchestrator commits.

## Closure

_(empty — the implementer's readings go here)_

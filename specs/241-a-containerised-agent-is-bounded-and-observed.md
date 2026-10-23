# Spec: 241 — a coder agent that runs in a container is bounded by the node's timeout_ms and its output reaches the journal

**Project**: /projects/pi-improved
**Phase**: filed 2026-10-23T11:20Z by the orchestrator. Stage 2 of the operator's path: pii runs ON THE HOST (podman lives
there) and manages development inside a container (operator, 2026-10-23: "the host runs podman and the solution must run
from there"). Premises read in `crates/pii-server/src/node_kinds/nodes/`, not assumed:

- `coder/graph_node.rs:95` calls `self.launch_in_container(ctx, image, &spec, env, &dispatch.argv, &skills, &runtime)?`
  with NO timeout; line 101 calls `self.launch_on_host(ctx, &dispatch.argv, &spec, env, super::agent_timeout(cfg))?`.
  Spec 195/`ea2976f` bounded only the host path.
- `coder_runtime.rs` `PodmanApi::start_and_wait` (lines ~217-300) starts the container, reports `Spawn`, then POSTs
  `/containers/{id}/wait` with `ctx.check()` as its only interrupt, and returns `Launch::Exited { code, pid, dur_ms,
  output: String::new(), stderr: String::new() }` (lines 298-299). `PodmanApi::run` force-deletes the container after the
  outcome on every path (`DELETE /containers/{id}?force=true`).
- `PodmanCli::run` (same file, `impl ContainerRuntime for PodmanCli`, line ~135) is the other implementation behind the
  `ContainerRuntime` trait.
- Everything built for the host path reads `Launch::Exited.output` / `.stderr`: redaction, the failure cause in
  `explain`, `reported_routes`, `claude_*` facts, `quota_signal`, `route_drift`. In a container all of it sees empty text.
- `TIMEOUT_EXIT_CODE` (124) and `stop_reason` already map a timeout exit to `stop_reason = timeout`.

## Overview

Two gaps make the container path blind and unbounded, and the container path is the production path:

1. A hung agent in a container runs until something outside pii stops it. `timeout_ms` is read only on the host path.
2. A container run reports no output, so a failed container run explains itself as `worker exited N` with no cause, a claude
   run in a container never journals its session, cost or route, and a spend-limit message is invisible.

## Requirements

- **R1** `launch_in_container` takes the node's timeout (`super::agent_timeout(cfg)`, as the host path does). When it elapses,
  the container is stopped and force-removed, and the launch returns `Launch::Exited` with code `TIMEOUT_EXIT_CODE`.
- **R2** The bound applies to BOTH `PodmanApi` and `PodmanCli`. The wait is interrupted at the deadline, not after it.
- **R3** After the container exits (including a timeout), its stdout and stderr are read from the runtime
  (`GET /containers/{id}/logs?stdout=1&stderr=1` for the API; `podman logs` for the CLI) BEFORE the container is removed,
  bounded to `OUTPUT_KEEP_BYTES` per stream, and returned in `Launch::Exited.output` / `.stderr`.
- **R4** The logs go through the same path as the host's: redaction (`redact::redact`) and the existing journal fields run on
  them unchanged. No new journal field is added by this spec.
- **R5** A node with no `timeout_ms` is unbounded as before. A container run that fails to read its logs still returns its
  exit code; the missing logs are an empty string plus a `Level::Warn` event `coder.logs_unreadable` naming the container.
- **R6** Nothing changes for the host path.

## Architecture

The deadline is a parameter of the `ContainerRuntime::run` trait method (`Option<Duration>`), checked in the wait loop beside
`ctx.check()`, so both implementations honour it. The API wait is a blocking `POST /wait`; to interrupt it, issue
`POST /containers/{id}/stop?t=2` (then the force `DELETE` that already follows) from the same thread when the deadline
passes, by polling `GET /containers/{id}/json` for `State.Running` at the existing `POLL` interval instead of one blocking
`/wait`. Rejected: a watcher thread (a second owner of the container id), and relying on `podman run --timeout` (not honoured
by the API path, which is the one used under the landlock domain).

## TDD Contract

New test file `crates/pii-server/tests/coder_container_bounded.rs`. The API tests run against a fake runtime socket: a Unix
listener under `_tmp/` that speaks just enough of the libpod REST protocol (`create`, `start`, `json`, `wait`, `logs`, `stop`,
`DELETE`) — read `coder_runtime_api.rs` or the neighbouring `PodmanApi` tests first and reuse that fake; if there is none,
write one in the test file. It counts the calls it receives.

| test | given | expects |
|---|---|---|
| `a_container_that_outlives_its_timeout_is_stopped` | fake runtime whose container never exits, `timeout = 300 ms` | launch returns code 124 in under 3 s; the fake saw `stop` then `DELETE` |
| `a_container_that_exits_in_time_is_untouched` | container exits 0 after 50 ms, `timeout = 5 s` | code 0; the fake saw NO `stop` (CONTROL) |
| `no_timeout_waits_for_the_container` | container exits after 400 ms, `timeout = None` | code 0 (CONTROL) |
| `a_failing_container_keeps_its_exit_code` | exits 3, timeout set | code 3, not 124 (CONTROL) |
| `the_containers_stdout_and_stderr_are_returned` | fake logs `out-line` and `err-line` | `output` contains `out-line`, `stderr` contains `err-line` |
| `the_logs_are_read_before_the_container_is_removed` | record call order | `logs` precedes `DELETE` |
| `a_timeout_still_returns_the_logs` | hung container with a log line, timeout | code 124 AND the log line present |
| `the_logs_are_bounded` | 5 MB of output | returned length is at most `OUTPUT_KEEP_BYTES` |
| `unreadable_logs_do_not_hide_the_exit_code` | `logs` returns 500 | code intact, empty output, one `coder.logs_unreadable` warning |
| `a_secret_in_the_container_log_is_redacted_in_the_journal` | the log holds a planted `sk-live-…` token | the `coder.exited` event never contains it |

The requirement a plausible implementation gets wrong is R3's ORDER: reading logs after the force-remove returns nothing, and
a test that only checks "output is non-empty" passes against a fake that serves logs after removal. The ordering row and the
timeout-still-returns-logs row catch it.

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test coder_container_bounded 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `grep -A3 'launch_in_container(' crates/pii-server/src/node_kinds/nodes/coder/graph_node.rs | grep -q 'agent_timeout'`
- [ ] `grep -q "containers/{id}/logs\|/logs?" crates/pii-server/src/node_kinds/nodes/coder_runtime.rs`
- [ ] `cargo test -q -p pii-server --test coder_agent_timeout 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-server --test coder_output_redacted 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`

## Guardrails

- `specs/` are READ-ONLY for the implementer. If the container path is not shaped as described (no `ContainerRuntime` trait, a
  `launch_in_container` that already takes a timeout), STOP and report the mismatch instead of forcing the change.
- Never weaken an existing test. `coder_agent_timeout` and `coder_output_redacted` must stay green unchanged.
- No real podman, no network, no spend: every test uses the fake socket or a fake `podman` script under `_tmp/`.
- Do not change the host path, `wait_child`, `stop_reason`, `TIMEOUT_EXIT_CODE` or any existing `coder.exited` field.
- The container is ALWAYS force-removed, on every path including a timeout and an unreadable log (the existing contract).
- Tests write under `_tmp/`. 450-effective-line file gate and `#![forbid(unsafe_code)]` apply; `coder_runtime.rs` is near the
  limit — if the change does not fit, split the API client into `coder_runtime_api.rs` and say so in the Closure.
- Do NOT commit; the orchestrator commits.

## Closure

_(empty — the implementer's readings go here)_

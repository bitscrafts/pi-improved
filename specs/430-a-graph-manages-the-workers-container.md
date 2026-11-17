# Spec: a graph manages the worker's container
#: depends=429 status=pending block=-

## Overview
Measured 2026-11-15 on the devbox: the worker lane aborted with
`container preflight failed: can only create exec sessions on running containers: container state improper`
and the queue recorded the ROW as `needs-review` / `lane-not-started rc=9`. Rows 414, 416, 417, 418 and 429 were
implemented by hand for that reason, none of them defective.

Owner's decision (2026-11-15): there is no "dev mode". The container is reusable and its lifecycle is pii's own
responsibility, expressed as a **graph** — the policy lives in `graphs/worker-runtime.toml`, and the primitive it composes
already exists in Rust (`crates/pii-server/src/node_kinds/nodes/coder_runtime_api.rs`: image inspect `:47`,
`/containers/create` `:67`, delete `:88`).

## Requirements
R1. A graph `graphs/worker-runtime.toml` decides, in this order, and its outcome is data:
    `unavailable` (no runtime) | `ready` (container id, image digest) | `refused` (named reason).
R2. **Identity**: the container is found by label (`pii.role=worker`, `pii.instance=<instance>`), never by name guess.
R3. **Life**: absent -> create + start; present and stopped -> start; present and running -> reuse.
R4. **Digest**: if the image digest now differs from the digest the container was created from, the container is
    recreated. Running an older image is the same defect as the stale installed binary.
R5. **Tools**: `ready` is only reported after an exec inside the container proves the toolchain is present; the probe
    list is data in the graph (at minimum `cargo`, `rustc`, `git`), not a hard-coded script.
R6. **Mounts**: mounts are computed from a configured host root, never from pii's own view of the tree. A path that does
    not exist under the host root refuses BEFORE any container is created.
R7. **Property**: the graph never removes a container it did not create (its own are labelled and listed by label).
R8. **Failure semantics**: no runtime -> the row stays `pending` with a named reason. An environment limitation is never
    recorded as a row failure.
R9. The podman socket path is explicit: `PII_PODMAN_SOCKET` wins, `XDG_RUNTIME_DIR/podman/podman.sock` is the default,
    and an unset XDG without the override refuses with a message naming both.

## Design
The graph is policy; the primitive is Rust. One node kind `container` is added, wrapping the API client that already
exists, with actions `inspect | find | start | create | exec | digest`. The graph:

    socket -> probe -> find -> decide -> act -> tools -> end
                        \\-> unavailable (named)         \\-> refused (named)

`decide` is a pure transform over (found?, state, created_digest, current_digest) with the four outcomes of R2-R4, so
every branch is testable without podman.

## TDD Contract
| Test Name | Input | Expected Output |
| --- | --- | --- |
| decide_reuses_a_running_container | found, running, same digest | `reuse` |
| decide_starts_a_stopped_container | found, exited, same digest | `start` |
| decide_creates_when_absent | not found | `create` |
| decide_recreates_on_digest_change | found, running, digest differs | `recreate` |
| decide_control_same_digest_never_recreates | found, running, same digest | not `recreate` |
| graph_refuses_before_creating_when_the_host_path_is_missing | host_root without the repo | no `/containers/create` call |
| graph_never_removes_a_foreign_container | unlabelled container present | no delete call |
| socket_uses_the_explicit_override_first | both set | the override path is used |
| socket_refusal_names_both_paths | neither set | message names `PII_PODMAN_SOCKET` and `XDG_RUNTIME_DIR` |
| unreachable_leaves_the_row_pending | no runtime | row `pending`, reason `no container runtime` |

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test worker_runtime 2>&1 | grep -qE 'test result: ok\. ([5-9]|1[0-9]) passed'`
- [ ] `pii graph lint graphs/worker-runtime.toml`
- [ ] `bash scripts/checks/worker-runtime-graph.sh`

## Files
- `graphs/worker-runtime.toml`
- `crates/pii-server/src/node_kinds/nodes/container.rs`
- `crates/pii-server/src/node_kinds/nodes/container/decide.rs`
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-server/tests/worker_runtime.rs`
- `crates/pii-tools/src/sandbox.rs`
- `scripts/checks/worker-runtime-graph.sh`
- `gate-steps.txt`

## Notes
The lane consumes this graph's outcome and nothing else: `ready` -> dispatch the worker into that container;
`unavailable`/`refused` -> leave the row `pending` and print the name. The same graph runs on the lnx (socket at the
default path, host root `/projects`) and on the devbox (explicit socket, host root configured) — one path, two
configurations.

Risk noted at write time: this graph has side effects (`create`, `start`), so it is single-writer by construction —
the queue lock already serialises cycles, and R7 keeps it from touching anything it did not create.

## Guardrails
- Do NOT add a "dev mode" that bypasses the container; the owner rejected it (2026-11-15).
- Do NOT remove existing containers, images or volumes.
- Do NOT let the graph construct mounts from the daemon's own filesystem view.

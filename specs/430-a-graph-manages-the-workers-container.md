# Spec: a graph manages the worker's container
#: depends=429 status=pending block=-

## Overview
Measured 2026-11-15 on the devbox: the worker lane aborted with
`container preflight failed: can only create exec sessions on running containers: container state improper`
and the queue recorded the ROW as `needs-review` / `lane-not-started rc=9`. Rows 414, 416, 417, 418 and 429 were
implemented by hand for that reason, none of them defective.

Owner's decision (2026-11-15), in his words: there is no "dev mode"; the container is reusable and pii owns its
lifecycle; **the container is another graph** — no container-specific node kind — the projects path is an argument,
and one instance is enough ("simpler better").

So the runtime is two graphs composed of primitives that already exist:

    graphs/podman.toml          the runtime surface: ping, find, start, create, exec, digest — policy only
    graphs/worker-runtime.toml  composes it for the worker: takes {instance, image, host_root, probe}, returns the verdict

## Requirements
R1. `graphs/worker-runtime.toml` returns, as data on its `end` node:
    `status` (`ready|unavailable|refused`), `container_id`, `digest`, `reason`.
R2. **Identity**: the container is found by label (`pii.role=worker`, `pii.instance=<instance>`), never by a guessed name.
R3. **Life**: absent -> create + start; present and stopped -> start; present and running -> reuse.
R4. **Digest**: if the image digest now differs from the one the container was created from, it is recreated. Running an
    older image is the same defect as the stale installed binary.
R5. **Tools**: `ready` is reported only after an exec inside the container proves the toolchain is present; the probe
    list (`probe`) is an input, not a hard-coded script.
R6. **Mounts**: mounts are computed from `host_root` (an input; `projects` by default for the repo) and never from pii's
    own view of the tree. A path missing under `host_root` refuses BEFORE any container is created.
R7. **Property**: never remove a container, image or volume the graph did not create. Its own are labelled.
R8. **Failure semantics**: no runtime -> the row stays `pending` with a named reason. An environment limitation is never
    recorded as a row failure.
R9. **One instance.** A single `instance` (default `worker`) serves the lane, the judge and any coder graph; lanes are
    not given separate containers until something actually needs it.

## Composition
The graph is a unit other graphs compose, not a step bolted onto the lane. That fixes its shape:

R10. **Declared interface, both directions.** Inputs: `instance`, `image`, `host_root`, `probe`, `repo_subpath`.
     Outputs: `status`, `container_id`, `digest`, `reason`. A caller branches on `status`; it never parses prose.
R11. **No implicit state.** Every input is a node input. The socket path and host root arrive as inputs, so a caller on
     the same machine can ask for a different container without the graph knowing anything global.
R12. **Idempotent for a given input.** Running it twice in a row reports `ready` the second time without creating,
     starting or recreating anything, returning the same `container_id` and `digest`.
R13. **Composable by callers that know nothing about podman.** The worker lane, a `judge` graph and any future coder
     graph invoke the same unit with their own inputs; none of them contains a podman decision.
R14. **Pure where it can be.** `decide` is a pure transform over (found?, state, created_digest, current_digest), so a
     caller's branch on `status` is exercised by feeding the four outcomes — no runtime needed.

## Design
No new node kind. The graphs use the primitives that already exist — a command node and a transform — and podman's own
`--format json`, which is structured and stable, so nothing is parsed out of prose:

    probe   podman version --format json                 -> runtime present?
    find    podman ps -a --filter label=pii.role=worker --format json
    digest  podman image inspect <image> --format '{{.Digest}}'
    decide  transform: reuse | start | create | recreate
    act     podman start <id> | podman create ... | podman rm -f <own id only>
    tools   podman exec <id> <probe> (one per entry of `probe`)
    end     transform -> {status, container_id, digest, reason}

## TDD Contract
| Test Name | Input | Expected Output |
| --- | --- | --- |
| decide_reuses_a_running_container | found, running, same digest | `reuse` |
| decide_starts_a_stopped_container | found, exited, same digest | `start` |
| decide_creates_when_absent | not found | `create` |
| decide_recreates_on_digest_change | found, running, digest differs | `recreate` |
| decide_control_same_digest_never_recreates | found, running, same digest | not `recreate` |
| graph_refuses_before_creating_when_the_host_path_is_missing | host_root without the repo | no create call |
| graph_never_removes_a_foreign_container | unlabelled container present | no `rm` |
| a_second_run_is_ready_and_creates_nothing | same input twice | `ready` twice, same ids, no create |
| unreachable_reports_unavailable_not_refused_by_the_row | no runtime | `status=unavailable`, row `pending` |

## Exit Criteria
- [ ] `pii graph lint graphs/podman.toml && pii graph lint graphs/worker-runtime.toml`
- [ ] `cargo test -q -p pii-server --test worker_runtime 2>&1 | grep -qE 'test result: ok\. ([7-9]|1[0-9]) passed'`
- [ ] `bash scripts/checks/worker-runtime-graph.sh`
- [ ] `pii graph run graphs/worker-runtime.toml --input '{"instance":"worker","image":"x","host_root":"/tmp","probe":["true"],"repo_subpath":"."}' 2>&1 | grep -q '"status"'`

## Files
- `graphs/podman.toml`
- `graphs/worker-runtime.toml`
- `crates/pii-server/tests/worker_runtime.rs`
- `crates/pii-tools/src/sandbox.rs`
- `scripts/checks/worker-runtime-graph.sh`
- `gate-steps.txt`

## Notes
The lane consumes this graph's verdict and nothing else: `ready` -> dispatch the worker into that container;
`unavailable`/`refused` -> leave the row `pending` and print the reason.

The same graphs run on the lnx (default socket, `host_root=projects`) and on the devbox (explicit socket, `host_root`
an argument): one path, two configurations. Choosing the CLI over the HTTP API is deliberate — no new client to keep in
step with podman's API version, and `--format json` is a documented, stable surface.

Risk noted at write time: this graph has side effects (`create`, `start`), so it is single-writer by construction — the
queue lock already serialises cycles, and R7 keeps it from touching anything it did not create.

### New in this row
- [ ] `bash scripts/checks/worker-runtime-graph.sh`

## Guardrails
- Do NOT add a container-specific node kind: the owner's decision is that the container is a graph.
- Do NOT add a dev mode that bypasses the container.
- Do NOT remove existing containers, images or volumes.
- Do NOT construct mounts from the daemon's own filesystem view.

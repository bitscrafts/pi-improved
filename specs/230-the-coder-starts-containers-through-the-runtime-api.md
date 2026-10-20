# Spec: the coder node starts its container through the runtime's API, not by exec'ing podman

## Overview
M0 measured on lnx (2026-10-20, `_docs/05-operations/M0-2026-10-20.md`) closes the direct path:
rootless podman cannot initialise inside pii's landlock domain. Three gates, each reproduced under the
daemon's exact grant with a control: the `/dev/shm` rootless lock (spec 229), the pause-process re-exec
(`cannot re-exec process to join the existing user namespace`, rc 1), and — with a fresh podman state —
the subordinate-uid mapping (`newuidmap: write to uid_map failed: Operation not permitted`, rc 125).
Granting `/proc` read-write moves the failure from landlock's `open` denial (EACCES) to that credential
denial (EPERM): landlock mandates `no_new_privs`, and rootless podman's user-namespace setup does not
survive it. No further path grant can buy this.

The same measurement found the working substrate. `podman.socket` is active on the host
(`/run/user/1000/podman/podman.sock`), and a process confined by the daemon's exact grant — its
must-fail write-outside probe printing `RESTRICT OK` on every call — talked to it and started a
container: `POST /containers/create` → id, `/start` → 204, `/wait` → 0, `/json` → `"ExitCode":0`,
`DELETE` → 200. Landlock does not mediate `connect()`, so the socket is reachable while the domain stays
intact. The container, not the landlock domain, becomes the isolation boundary.

This spec makes the coder node use that path.

## Requirements
R1. A `ContainerRuntime` seam (traits-first) with the current `PodmanCli` implementation and a
    `PodmanApi` implementation that speaks the podman REST API over the user's runtime socket. The coder
    node selects it by graph data (`container_runtime = "socket"`), never by host introspection, and the
    resolved runtime is named in the journal.
R2. The API path is create → start → wait → inspect `ExitCode`, with the container removed afterwards.
    The node's success is the container's `ExitCode`, journalled as `coder.exited` with that rc.
R3. Refusals are honest and named: an absent socket, a refused connect and a non-2xx reply each fail the
    node before any spawn with `coder.runtime.unavailable` naming the socket path and, when there is one,
    the HTTP status — never `coder.image.missing` for a runtime failure (spec 228 R2's rule, extended).
R4. The socket path is the sandbox's one requirement: no new grant is added to the `workspace` profile by
    this path, and a test pins that the grant list and `grant_policy` text are unchanged by this spec.
R5. M0 is re-run on lnx after the change: `M0 PASS <pid>` with `coder.spawned` and `coder.exited` for one
    pid, or `M0 REFUSED <reason>` naming what still refuses. The verdict and raw output go to
    `_docs/05-operations/M0-2026-10-20.md`.

## TDD Contract
- pii-server `tests/coder_starts_through_the_runtime_socket.rs`: a fake runtime over a unix socket the
  test itself creates under `_tmp` (via `pii_store::testing::tmp_scratch`) answers `/_ping`, create,
  start, wait and json; the node must emit `coder.spawned` with the container id and `coder.exited` with
  its rc. Controls in the same file: (a) no socket at the named path → `coder.runtime.unavailable`
  before any spawn, with the path in the event; (b) a 500 on create → the node fails, no spawn, status
  quoted.
- pii-server `coder_runs_in_podman.rs` stays green: the CLI implementation is kept, the seam does not
  change its behaviour.
- pii-tools: `runtime_socket()` resolves `XDG_RUNTIME_DIR/podman/podman.sock`, refuses when the variable
  is unset (must-fail), and is pinned in `grant_policy`'s sibling text test as "the socket, not a grant".
- RED first: on this HEAD the seam and the two tests do not exist.

## Exit Criteria
- [ ] cargo test -p pii-server --test coder_starts_through_the_runtime_socket
- [ ] cargo test -p pii-server --test coder_runs_in_podman
- [ ] cargo test -p pii-tools
- [ ] grep -q 'coder.runtime.unavailable' crates/pii-observe/src/names.rs
- [ ] on lnx: bash scripts/m0-daemon-starts-a-container.sh -> M0 PASS <pid>, and the run's journal holds both coder.spawned and coder.exited for that pid
- [ ] _docs/05-operations/M0-2026-10-20.md carries that verdict with its raw output

## Guardrails
- Do NOT widen the sandbox, add a grant, or turn the profile off to make M0 pass; the measurement above
  is the proof that a grant cannot help.
- Do NOT reach the runtime through the podman CLI's remote-client mode (`--remote` / `--url`); pii holds
  ONE client for the socket protocol, so the refusal paths are ours and testable.
- Do NOT build an image, pull one, or retry a refused start.
- Do NOT claim the container design works before M0 prints PASS with both events for one pid.

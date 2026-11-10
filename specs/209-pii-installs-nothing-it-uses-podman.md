# Spec: pii installs nothing — podman is the tool

## Overview
User directive 2026-10-19: "pii should install nothing! and as it has access to podman, that will be the tool pii will work with!" Today the coder node spawns a binary on the host (`~/.local/bin/pi`, `claude`) and the deploy installs three binaries into `~/.local/bin`. That makes the host a mutable dependency: a toolchain, an agent version and a PATH all have to be installed and maintained, and every install is a state change nobody records. Podman is already on the host and already the way this workspace runs services. So pii's execution substrate is a CONTAINER, named in the graph, pulled or prebuilt — never installed by pii.

**Project**: <repo>

## Verified premises
- 207 added a `coder` kind whose scope admits an agent binary (`coder_scope_admits_agent` passes today).
- The installer ships three host binaries from a clean export; the deploy step is therefore an install by definition.
- Podman is present on the host and is how the workspace already runs daemons (quadlets, `podman exec`).

## Requirements
### Functional
1. A coder node names its runtime: `container_image = "<image:tag>"` in the GRAPH, not in Rust. Absent image = the host path (deprecated, must log `coder.host_spawn.deprecated`).
2. The spawn is `podman run --rm --workdir /work -v <workspace>:/work --env-file <scrubbed> <image> <agent-argv>` — the agent binary lives in the image, never on the host.
3. The capability scope admits `podman` and nothing new: a coder node may run podman; it may not run arbitrary host binaries.
4. Environment is passed by NAME (`env_pass`) through a temporary env file with mode 0600, deleted after the run; values never appear in argv (podman `--env` from argv leaks through `ps`).
5. No network unless the graph grants it: `--network none` by default, `--network <grant>` only when the graph declares one.
6. The image is prebuilt and immutable: pii never runs `apt`, `apk`, `pip`, `npm`, `cargo install` or `podman build` as part of a run. A missing image is a REFUSAL (`coder.image.missing`), not an install.
7. The workspace mount is the only writable path; `/root`, `/var/run` and the podman socket are never mounted.

### Non-Functional
- `podman run --rm` guarantees no container outlives the node; a killed node leaves no container (checked with `podman ps -a --filter label=pii.run=<id>`).
- The image digest is recorded in the run's journal, so a verdict names exactly which image produced it.
- No privileged flag, no `--userns=host`, no docker socket.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| coder_runs_in_podman | coder node with container_image | argv begins `podman run --rm`; no host agent binary in argv |
| image_is_graph_data | two runs, two images | two different argv, same binary |
| missing_image_refuses | image that does not exist | `coder.image.missing`, no container started, heal port |
| no_install_in_a_run | any coder run | argv contains no `apt|apk|pip|npm|cargo install`, no `podman build` |
| network_is_none_by_default | graph without a grant | `--network none` present |
| container_is_cleaned | kill the node mid-run | `podman ps -a --filter label=pii.run=<id>` empty |

## Exit Criteria
- [ ] `cargo test -p pii-server --test coder_runs_in_podman`
- [ ] `cargo test -p pii-server --test coder_missing_image_refuses`
- [ ] `cargo test -p pii-server --test coder_no_install_in_a_run`
- [ ] a live run on the host shows `podman ps -a --filter label=pii.run=<id>` empty after the node completes
- [ ] journal records the image digest for that run

## Guardrails
- Do NOT install anything on the host from pii: no package manager, no toolchain, no agent binary at run time.
- Do NOT mount the podman socket into a coder container.
- Do NOT pass secrets on the command line: env file only, mode 0600, removed after.
- Do NOT keep the host-spawn path as the default; it is a deprecated fallback that must announce itself.

## Depends on
- 207 (a registered coder kind that spawns) — this spec changes WHAT it spawns, not whether it can.

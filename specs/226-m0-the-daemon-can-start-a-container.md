# Spec: M0 — the sandboxed daemon can start a container at all

## Overview
Review 078 §0/§5: before any design that mounts a project into an image, one measurement decides everything:

    M0: can the pii daemon — running under its own landlock sandbox (daemon.sandbox.in_force) — start a podman container?

Every confinement claim tonight was about code, not behaviour: the coder has never successfully spawned, so the container path
has never been exercised. If M0 fails, "one default image + mount the project" is moot and the design must change.

## Requirements
1. A script `scripts/m0-daemon-starts-a-container.sh` that, on a host with podman and a running daemon:
   a. asks the daemon for a coder node run whose ONLY action is a trivial container (`podman run --rm localhost/pii-agent:1 true`);
   b. records the journal events for that run;
   c. prints exactly one verdict line: `M0 PASS <pid>` / `M0 REFUSED <reason>` / `M0 FAILED <rc> <stderr tail>`.
2. The reason must come from the journal or the daemon log, never from a summary.
3. If the sandbox blocks the container runtime, the refusal must name the syscall/path — a bare EPERM is a failed measurement.

## Exit Criteria
- [ ] `bash scripts/m0-daemon-starts-a-container.sh` prints one of the three verdict lines above
- [ ] the run's journal contains `coder.spawned` with a real pid when M0 PASSes
- [ ] the verdict is filed in `_docs/05-operations/M0-<date>.md` with the command and the raw output

## Guardrails
- Do NOT "fix" M0 by disabling the sandbox; a pass with the sandbox off measures nothing.
- Do NOT accept a run that reports success without `coder.spawned` (spec 221).
- M0 is read-only for the project: mount nothing rw.

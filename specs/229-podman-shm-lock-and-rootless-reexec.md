# Spec: podman's shared-memory lock and its rootless re-exec, inside the sandbox

## Overview
Spec 228 closed the first two refusals on lnx (bolt_state.db and the /run/user/1000/libpod runroot
lock, plus the honest refusal text). With 228 deployed (0.1.0, df0fdae), M0 measures ONE gate further
and stops there. The chain, each step reproduced under pii's exact landlock grant with a control:

1. `bash scripts/m0-daemon-starts-a-container.sh` on lnx (run r000000000209) prints
   `M0 REFUSED Error: failed to open 2048 locks in /libpod_rootless_lock_1000: permission denied`.
   The refusal now quotes podman, so the measurement is the runtime's own words.
2. That lock file lives in `/dev/shm` (podman issue #16119: `/dev/shm/libpod_rootless_lock_<uid>`);
   the `workspace` profile grants `/dev` READ-ONLY and only four device files read-write, so podman's
   shm path fails and its fallback base is empty, producing `/libpod_rootless_lock_1000`.
3. A C reproducer of the daemon's exact grant (kept at `_tmp/m0-landlock-repro.c`, built on lnx)
   reproduces step 1 bit-for-bit; adding `rw /dev/shm` to that grant moves podman to its NEXT gate:
   `Error: invalid internal status, try resetting the pause process with "/usr/bin/podman system migrate":
   cannot re-exec process to join the existing user namespace` (rc=1). Control: the same grant without
   `/dev/shm` reproduces the lock error; with the sandbox off podman runs the image (rc=0).
4. The re-exec failure is podman's rootless pause-process machinery: every invocation re-execs itself
   into the user namespace the pause process holds, and under the sandbox that join fails. Whether this
   is a landlock denial (a path the re-exec opens) or a namespacing restriction the profile cannot
   express is NOT yet measured; this spec measures it and decides.

## Requirements
R1. `/dev/shm` is granted read-write by the daemon's sandbox grant (beside the three container state
    dirs of spec 228), created/inspected at daemon start, and named in `grant_policy`'s text with the
    pinned behaviour test updated in the same edit. It is a tmpfs belonging to the same user; the
    grant is the shm directory only, never `/dev` as a whole.
R2. The rootless re-exec failure is MEASURED, not guessed: a reproducer (the C grant replica extended
    with /dev/shm) runs `podman run --rm <image> true` under the exact daemon grant and captures
    podman's stderr; the spec's report states which step fails (re-exec, clone/unshare, a named path,
    or a kernel refusal), with the raw line. If the re-exec needs a path, name it and file the grant;
    if it is a namespace/ptrace restriction landlock cannot express, say so plainly and mark the
    container-inside-the-daemon design BLOCKED on an architecture decision (review 078), not on a
    missing grant.
R3. M0 is re-run on lnx after the fix lands: `M0 PASS <pid>` with `coder.spawned` journalled, or
    `M0 REFUSED <reason>` naming the remaining syscall/path. The verdict and the raw output are filed
    in `_docs/05-operations/M0-2026-10-20.md`.
R4. Control: with the sandbox off the same image starts (already measured, rc=0); the fix may not be
    declared by removing or weakening the sandbox.

## TDD Contract
- pii-tools: the grant list unit test gains `/dev/shm` (exact-members control unchanged: the list is
  the dirs the helper returns, nothing invented), and `grant_policy`'s pinned text test names it.
- pii-cli/tests/sandbox_grant.rs: the source-reading pin gains the `/dev/shm` clause in the same edit.
- Behaviour control in the same file: a `sandbox-exec` child writing inside /dev/shm succeeds, while a
  write outside the grant still fails (the existing denial test stays green).
- RED first: on 475a-HEAD the new assertions fail (no /dev/shm in the list/text).
- Live: the M0 verdict on lnx moves PAST the lock error; whatever it reports next is the measurement.

## Exit Criteria
- [ ] cargo test -p pii-tools
- [ ] cargo test -p pii-cli --test sandbox_grant
- [ ] CARGO_TARGET_DIR=_tmp/tools-gate cargo run -q -p xtask -- lint-observability
- [ ] on lnx: bash scripts/m0-daemon-starts-a-container.sh -> a verdict whose reason is podman's own stderr
- [ ] _docs/05-operations/M0-2026-10-20.md carries that verdict and the raw output

## Guardrails
- Do NOT grant `/dev` or `/` write; `/dev/shm` only.
- Do NOT disable the sandbox, add a retry, or extend the probe timeout to make M0 look better.
- Do NOT claim the container design works while the re-exec is unresolved; the honest M0 line is the
  deliverable.

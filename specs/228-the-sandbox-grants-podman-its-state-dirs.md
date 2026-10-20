# Spec: the sandbox grants the container runtime its state dirs, and every refusal tells the truth

## Overview
Spec 226 asked M0: can the sandboxed daemon start a container at all. Measured 2026-10-20 on lnx
(10.10.10.217), podman installed, image localhost/pii-agent:1 present, digest
sha256:2684dee9d0fab97d03fac3f90e7fb774ede93297dbb52b5507142f337d06e7b8:

- Three daemon runs (r000000004552, r000000004553, r000000004554) refused with coder.image.missing,
  image not present; build it outside pii.
- Control, the same probe with the sandbox off: the digest prints, rc=0. The image IS present; the
  journal names a false cause.
- A C reproducer of pii own landlock grant (pii-tools/src/sandbox.rs profile=workspace: handled bits
  0..13 incl REFER, ro=/ and /dev, rw=/tmp, /run/user/1000/pii, /home/mvcorrea) was built on lnx
  (/tmp/m0-landlock-repro.c, kept at _tmp/m0-landlock-repro.c in the repo). It printed RESTRICT OK (its
  must-fail probe, writing outside the grant, failed as required), then podman failed with:
  Error: acquiring runtime init lock: open /run/user/1000/libpod/tmp/alive.lck: permission denied (rc 125).
- A second podman state dir fails the same way (pii sandbox-exec, workspace=/tmp):
  Error: opening database /home/mvcorrea/.local/share/containers/storage/libpod/bolt_state.db:
  permission denied (rc 125).

Root cause, two findings in one:
1. The workspace profile grants RW only to the workspace, store dir, runtime dir, log parent and
   TMPDIR (pii-tools/src/sandbox.rs rw_paths; pii-cli/src/daemon_log.rs writable_dirs feeding
   pii-cli/src/cmd_daemon.rs extra_rw). Podman keeps state in HOME/.local/share/containers (graph
   root, bolt_state.db) and XDG_RUNTIME_DIR/containers plus XDG_RUNTIME_DIR/libpod (runroot, locks).
   None is granted, so a sandboxed daemon cannot even inspect an image that exists.
2. coder.rs launch_in_container maps a nonzero probe exit to refuse_image("image not present;
   build it outside pii") and never surfaces the captured stderr (pii-core/src/childproc.rs
   ProcOutput.stderr). The journal then carries a false cause: the journal cannot say why
   podman failed. The refusal lies.

Landlock is inherited by descendants (no_new_privs propagates), so conmon, crun and the container
itself run under the same rule set; the container rootfs lives under the graph root, which is why the
grants in R1 are what makes the whole container tree possible under the workspace profile. Whether
anything else still refuses is exactly what M0 measures after this spec.

## Requirements
R1. A new helper (pii-tools, beside sandbox.rs) returns the container runtime state dirs derived from
    the environment at daemon start: HOME/.local/share/containers, XDG_RUNTIME_DIR/containers,
    XDG_RUNTIME_DIR/libpod (create_dir_all them like writable_dirs does; a dir that cannot be created
    is not granted and podman fails loudly). cmd_daemon appends the helper result to extra_rw before
    apply_workspace. Nothing is hardcoded beyond those three documented defaults. grant_policy gains
    one clause naming the podman state dirs, and the pinned behaviour test in
    pii-cli/tests/sandbox_grant.rs asserts the new text.
R2. The probe is honest: on a nonzero probe exit, launch_in_container reads ProcOutput.stderr and the
    refusal clause carries a trimmed tail of it (first 240 chars, whitespace collapsed). The
    coder.image.missing event gains a probe_stderr field with that tail. The Err branch (probe could
    not run) keeps naming its own error. A refusal may only say image not present when the stderr
    actually says the image is absent (podman exits 0 on inspect only when present; any other nonzero
    exit carries its stderr).
R3. The sandbox self-verification stays fail-closed and its probe stays: a granted-dir probe is ADDED as a
    control (a write INSIDE one granted runtime-state dir must SUCCEED; if it cannot, the daemon exits 4
    with the dir named) so a future regression cannot pass by making the whole grant inert.
R4. M0 re-measures on lnx after a deploy carrying R1+R2: scripts/m0-daemon-starts-a-container.sh must print
    M0 PASS <pid> with coder.spawned journalled, or M0 REFUSED <reason> where the reason is podman own
    stderr, never an invented cause. The M0 doc (_docs/05-operations/M0-2026-10-20.md) gains a 2026-10-20
    late entry quoting both verdicts (the pre-fix FAILED and the post-fix one).

## TDD Contract
- pii-tools/src/sandbox.rs: a unit test builds the grant list from a env (HOME, XDG_RUNTIME_DIR, TMPDIR
  set deliberately) and asserts the three runtime dirs are present; a second test with the env EMPTY
  asserts the list still contains HOME/.local/share/containers via the HOME default and nothing else
  invented. Control: the list does NOT gain arbitrary paths (assert its length and members exactly).
- pii-cli/tests/sandbox_grant.rs: the pinned grant text assertion updated in the same commit as
  grant_policy (one edit, both sides) - the test must FAIL on the old text and pass on the new, and a
  negative control (a grep for the old clause) greps the SOURCE, not a copy.
- pii-server/src/node_kinds/nodes/coder.rs tests (coder_runs_in_podman.rs): the fake podman now exits 1
  on image inspect with a stderr line; the test asserts refuse_image carries that line verbatim (trimmed)
  in both the Launch::Refused string and the coder.image.missing probe_stderr field. Control: an inspect
  that exits 0 must never carry probe_stderr.
- RED first: on HEAD, the sandbox unit test fails (no runtime dirs in the list) and the coder test fails
  (probe_stderr absent). No test may hand-write the stderr it asserts; it reads Launch/event output only.

## Exit Criteria
- [ ] cargo test -p pii-tools
- [ ] cargo test -p pii-server --test coder_runs_in_podman
- [ ] cargo test -p pii-cli --test sandbox_grant
- [ ] CARGO_TARGET_DIR=_tmp/tools-gate cargo run -q -p xtask -- lint-scratch
- [ ] CARGO_TARGET_DIR=_tmp/tools-gate cargo run -q -p xtask -- lint-observability
- [ ] bash quality-gate.sh --routine
- [ ] on lnx after deploy: re-run M0; verdict pasted in _docs/05-operations/M0-2026-10-20.md

## Guardrails
- Do NOT grant / or /home write broadly; the three runtime dirs are the narrow fix.
- Do NOT weaken the fail-closed probe; R3 ADDS the in-grant control, it never removes the out-grant one.
- Do NOT translate or paraphrase podman stderr into a prettier reason; quote it.
- Do NOT run M0 on this container (no podman); lnx only.

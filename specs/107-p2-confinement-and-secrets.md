# Spec: 107 — P2: make the confinement real, and make the secrets not leak

**Project**: <repo>
**Phase**: P2 of the road to production (`_docs/06-research/052-production-readiness-2026-10-09.md` §8, and
its CORRECTION section — read both; the correction is mine and it changes what P2 is).
**Goal**: the capability exists; the DEFAULT is not safe, and the observability data path is untested for
secrets.

## What is already true (measured by the orchestrator, 2026-10-09 — do not re-derive)
```
kernel 7.1.13 · Landlock ABI v9 (landlock_create_ruleset(NULL,0,VERSION) → 9)
profile workspace: write INSIDE the workspace        → rc=0, created
                   write OUTSIDE the workspace       → rc=1 "Permission denied", not created
                   clobber an existing file OUTSIDE  → rc=1 "Permission denied", original intact
                   read /etc/passwd                  → rc=0   ← reads outside are ALLOWED
profile off (control)                                 → rc=0, permissive
```
Implementation: `crates/pii-tools/src/sandbox.rs` (`landlock` crate) applied process-wide by
`pii sandbox-exec` and the one-shot/daemon paths. So P2 is NOT "implement Landlock" — it is four sharper
items.

## Requirements
R1. **Decide and enforce the default profile.** Today the daemon runs `off` ("sandbox=off: no filesystem
    confinement applied"). Choose the safe default and make it explicit — recommended: the daemon REFUSES to
    start unconfined unless the operator opts in (`--sandbox off` / `[core] sandbox = "off"`), or defaults
    to `workspace`; either way `health` must state the profile IN FORCE and the exact grant. Say which you
    chose and why, and paste the before/after of `pii health --json` (`sandbox_profile`, and what the
    mounts say). A silent `off` is the defect: the operator cannot tell confinement from its absence today.
R2. **State and test the workspace profile's grant precisely.** It confines writes and not reads (measured).
    Either narrow the reads too, or document the exact rule (what is read-allowed and why, e.g. `/etc` for
    DNS/CA) — and make the health string and the runbook say the same thing as the code. A test must pin the
    read behaviour (a read outside the workspace) and the write behaviour, so a change to either is a
    failing test rather than a silent policy drift.
R3. **Denials must reach the audit trail where pii can see them.** Denials that go through pii's own tools
    (bash deny-list, cwd scope, `fscap`, the http `http:<host>` allowlist) must be emitted AND persisted
    durably (a file the operator can read after the fact, not only the stdout ndjson): name the file, its
    rotation, and paste a denial appearing in it from a real run. State plainly in the runbook that a
    kernel-level denial inside a CHILD process (a raw `echo > /outside`) is invisible to pii — that limit is
    real and must not be papered over.
R4. **A canary-secret test on the observability data path** (the data-safety half of the observability
    mandate): with a secret present in the environment and in a model response, assert that
    (a) the log file, (b) the ndjson stdout stream, and (c) a trajectory file contain NO occurrence of the
    canary value, and that (d) with redaction configured the same field is replaced/dropped. Paste the
    counts (`grep -c <canary>` = 0 in all three artefacts) and one positive control that the canary IS
    present where it must be (e.g. in the raw model response field before the sink), so the test can fail.
R5. **Secrets beyond env vars.** Decide and document: a 0600 file or a keyring, or an explicit statement
    that env-only is accepted for now WITH the reason and the compensating control. Do not silently leave it
    as-is: the decision is the deliverable.
R6. **seccomp: state its absence honestly** (and do not claim parity with Landlock). If a cheap, real
    syscall filter is feasible, say what it would cost; do not build a fake one.

## Exit Criteria
- [ ] R1: the default is decided, implemented, and pasted (`pii health --json` before/after; an unconfined
      start is either refused or loudly reported), with the reason for the choice
- [ ] R2: the read/write grant is documented AND pinned by a test; a test that changes either behaviour goes
      RED (paste the mutation control)
- [ ] R3: a real denial appears in a persistent audit file whose path and rotation are named (paste the
      line), and the runbook states the child-process limit verbatim
- [ ] R4: `grep -c <canary>` = 0 in the log file, in the ndjson stream, and in a trajectory file; the
      positive control shows the canary IS present in the raw field the sink received (paste both)
- [ ] R5: the secret-handling decision is written down with its compensating control, or implemented
- [ ] R6: seccomp's absence is stated in one sentence in the runbook
- [ ] `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate
      --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report lists every file changed; no daemon left running

## Guardrails
- NEVER weaken confinement to make a test pass; if the default is the safe one, an unconfined start must
  FAIL LOUDLY, not warn.
- Do NOT write a canary secret into a tracked file, a doc, or a commit — generate it at test time.
- A test that cannot fail is not a test: every new assertion needs its near-miss or mutation control.
- Emit through `pii-observe`; no new raw prints beyond the file's budget.
- Scratch under `<scratch>/`; delete your own exports (one is ~8G). Do not leave a daemon running.
- Do NOT commit; leave the tree for the orchestrator.

## Result + orchestrator verification (2026-10-09)
**Landed.** The daemon now defaults to `workspace`; `--sandbox off` is an explicit opt-out that warns;
denials go to an audit log; a canary test guards the log/ndjson/trajectory paths; env-only secrets are
accepted WITH compensating controls; seccomp's absence is stated.

**Reproduced by the orchestrator (my own runs):**
```
A. default start (no flag)   → pii health --json: sandbox_profile = workspace
   mount policy (verbatim): "sandbox=workspace (landlock): WRITES confined to the workspace, store dir,
   runtime dir, $TMPDIR and /dev/{null,zero,urandom,tty}; READS are NOT confined (read-only access to / is
   granted, so /etc, /home and every other readable path stay readable)"   ← the honest grant, in the operator's face
B. --sandbox off             → stderr carries "UNCONFINED" (1 occurrence); health: sandbox_profile = off
C. audit trail               → <runtime>/audit.log exists and records BOTH starts:
   {"kind":"daemon.sandbox.in_force","enforced":true,"profile":"workspace","grant":"…"}         level info
   {"kind":"daemon.sandbox.in_force","enforced":false,"profile":"off","grant":"… explicit opt-out"} level WARN
   ← an unconfined start is now a warn-level audit record, not silence
D. tests (my run): canary_secrets 3 passed · sandbox_grant 4 passed · daemon_default_sandbox 4 passed
   gates: quality-gate.sh ALL PASS · fresh-gate PASS · clippy -D warnings · fmt --check
```
**Worker's own RED-first evidence (its report):** the canary leaked 4× into ndjson before the fix; the
default start exited 5 instead of 4; a tool denial emitted no event. Two mutation controls on R2 (a wider
write grant → the deny test RED; a narrower read grant → the read test RED), and it fixed a VACUOUS test of
its own (the deny test passed when the sandbox failed to launch; it now requires an in-workspace write in
the same run) — exactly the discipline this repo requires.

**Honest limits and residuals, kept here:**
1. **`pii -p` (one-shot) still defaults to `off`** — asymmetric with the daemon; the worker deliberately did
   not touch it. It is a real inconsistency: the interactive path is the unconfined one. Next spec item.
2. The **scrubber** replaces values of env vars whose names look secret (exact value match, 8+ bytes) — a
   TRANSFORMED secret is not caught, and the **client event stream to connected UIs is not scrubbed**.
3. **Bash deny-list events are still not emitted** as `tools.*` events; only `core.cap.denied` appears.
4. A **kernel-level denial inside a child process remains invisible** to pii (stated verbatim in the runbook).
5. **seccomp is not implemented**, and env-only secrets remain accepted with a systemd
   `EnvironmentFile=-/etc/pii/pii.env` (0600, root-owned) + the scrubber as compensating controls.

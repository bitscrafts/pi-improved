# Spec: the project scaffold and the per-run skill dir (from review 078)

## Overview
Review 078 (2026-10-20) amended the chat design in three ways that this spec turns into work:

- Q2 **AMEND**: copy NOTHING from `$HOME`. Everything the agent reads is GENERATED from pii's templates and skill store.
- Q3 **AMEND**: materialise skills into a per-run directory bound **READ-ONLY**, not written into the project.
- Q5 **AMEND**: measurement M0 comes first — *can the sandboxed daemon start a container at all?* — and if M0 fails, nothing
  below matters.

## Requirements
1. `pii project init <dir>` writes ONLY generated files: `.pii/{settings.toml,policy.toml,graphs/,skills/}`,
   `.pi/{settings.json,APPEND_SYSTEM.md}`, `.claude/{settings.json,CLAUDE.md}`, and a `.gitignore` carrying
   `.pii/runtime/`, `.pi/sessions/`, `.claude/local/`, `*.env`, `*_env`.
2. It NEVER copies from `$HOME`, and it REFUSES to run if the target already contains any of: `auth.json`, a file matching
   `*_API_KEY=`, `.credentials*`, `*.env` with a value — printing the path and stopping.
3. Skills are materialised **per run** into a temp dir (e.g. `<runtime>/runs/<id>/skills/`), mounted read-only at
   `.pi/skills` and `.claude/skills` for that run only. Nothing is written into the project by a run.
4. Each materialised skill carries `sha256=<hex>` of the memory document it came from; a mismatch is `skill.drift` and the
   node REFUSES before spawning.
5. The contract text (definition of done is the named criterion; tests are read-only; only declared paths may change;
   nothing is installed; a refusal is a result) is written into BOTH `.pi/APPEND_SYSTEM.md` and `.claude/CLAUDE.md` from ONE
   template, so the two agents cannot receive different instructions.
6. `.claude/settings.json` carries the hook pair that enforces the contract: a PreToolUse deny for the read-only test paths,
   and a PostToolUse formatter. `.pi` expresses the same rule through its own project settings.
7. The journal records `skill=<name> sha256=<hex>` on `coder.spawned`, so a verdict names the instructions that produced it.

## Exit Criteria
- [ ] `cargo test -p pii-cli --test project_init_generates_and_never_copies`
- [ ] `cargo test -p pii-cli --test project_init_refuses_a_credential`
- [ ] `cargo test -p pii-server --test skills_are_materialised_read_only`
- [ ] `cargo test -p pii-server --test skill_drift_refuses_before_spawn`
- [ ] a scaffolded directory greps clean: `grep -rE 'auth\.json|_API_KEY=|credentials' .pii .pi .claude` returns nothing
- [ ] on lnx: a coder run whose skill pin matches proceeds to spawn; a deliberately edited skill document refuses

## Guardrails
- Do NOT copy a home directory "so the agent has settings" — that is how an API key reaches a commit.
- Do NOT let a run write into the project's `.pi`/`.claude`; materialisation is per run and read-only.
- Do NOT write two contract templates; one file rendered twice, or the agents drift.
- Do NOT proceed before M0 (see spec 226) has a reading.

## Closure

**Not closed — parked on M0.** The implementation is complete and its five runnable criteria are green on the
tree the worker left (2026-10-20T06:57Z, cycle `20261020T050527Z`, gate `ALL PASS`):

- `project_init_generates_and_never_copies` 1 passed / 0 failed — asserts the exact generated file list,
  that no file carries the `$HOME` marker planted in the fake home, and that `.pi/APPEND_SYSTEM.md` and
  `.claude/CLAUDE.md` are byte-identical (one template, rendered twice).
- `project_init_refuses_a_credential` 2/0 — the refusal names the offending path.
- `skills_are_materialised_read_only` 1/0 — the per-run dir is outside the project and bound read-only.
- `skill_drift_refuses_before_spawn` 2/0 — an edited document is `skill.drift` and refuses before spawn.
- the scaffold greps clean: `pii project init _tmp/orch-225-scaffold-*` wrote the 7 expected files and
  `grep -rE 'auth\.json|_API_KEY=|credentials' .pii .pi .claude` returned nothing (rc=1).

Criterion 6 — "on lnx: a coder run whose skill pin matches proceeds to spawn; a deliberately edited skill
document refuses" — is **not met and cannot be observed on this host yet**. Spec 226's measurement M0 returned
`M0 FAILED 1 core.cap.denied: {"cap":"proc",...}`: the daemon refuses the coder node at its `proc` check before
any podman invocation, and `/usr/bin/podman` does not exist on lnx. A run therefore never reaches a spawn, so
neither half of criterion 6 can be seen. The fake-podman shim inside `skills_are_materialised_read_only` is
coverage of the mount plan, not live acceptance — a mock answers what you send it. The criterion is left
untouched; the item stays parked until the spawn path has a reading.

Two row defects kept it in `needs-review` with `last=gated` and nothing staged: the commit-path column used `|`
where `cycle.sh` splits on `;` (so `git add` failed and the cycle exited 11), and the verify column carried 1 of
the 6 criteria. Both corrected here; the five runnable criteria are now the row's verify, and criterion 6 is
named in this Closure as the reason the row is parked rather than done.

A second gap, measured in the diff and worth more than criterion 6: **the live path never supplies a
`SkillSource` or a runs directory.** `CoderFactory::new` defaults to `skills = NoSkills` and `runs_dir = None`,
and both registrations that build the node kind in production — `crates/pii-server/src/engine/registry.rs:64`
and `:184` — call exactly `CoderFactory::new(Arc::clone(&log))`. `with_skills` / `with_runs_dir` appear ONLY in
the tests (`grep -rn 'with_skills(' --include=*.rs crates/` → `coder.rs` definitions and test files). So R3 and
R7 are a seam nobody wired: a deployed coder node with a pinned skill refuses it as missing (`NoSkills`), and
`coder.spawned` can never carry `skill=<name> sha256=<hex>` in production. The four green tests are real but
they build the factory the way production does not — the fake source answers what the test sends it. This is the
"asserted in a test, never read by the live path" trap (DO-NOT-REPEAT 1.50) and it is the reason this item is
parked instead of closed on five green criteria.

### 2026-10-20 18:17Z addendum — both premises above are now stale; one live reading remains

Measured at HEAD `3258d1e`, by the orchestrator, after specs 227/230/233/234 landed:

- **The live path does supply a `SkillSource` and a runs dir.** `crates/pii-server/src/engine/registry.rs:25`
  (`registry(`, the daemon's own) builds `CoderFactory::new(log).with_podman(podman)
  .with_skills(Arc::new(StoreSkills::new(memory))).with_runs_dir(store_dir.join("runs"))` at lines 72-77 —
  spec 227 R2. Only `lint_registry*` (`:160`, `:200`) keeps `NoSkills`, deliberately and in a comment, because
  lint never reads the store. `grep -rn 'with_skills(' crates --include=*.rs` now returns that registration,
  not only the definitions in `coder.rs:129` and the tests.
- **The spawn path has a reading.** Spec 230's M0 measured PASS on lnx (row A98: `coder.spawned` pid 3463854
  through `podman.socket`, `coder.exited` the same pid), so "the daemon refuses the coder node at its `proc`
  check before any podman invocation" no longer describes the shipped path; 231 and 232 followed it.
- **A skill can be put into and edited in the daemon's store** — spec 233 (`3d32079`) and spec 234 (`622826e`),
  so the drift half of criterion 6 has a way to create both pins without the redb lock.

Criterion 6 is therefore measurable, and it is the SAME reading as spec 227's criterion 7 (row A95): one deploy
of HEAD to lnx, one coder run pinning the stored skill (it must spawn, and `coder.spawned` must carry
`skill=<name> sha256=<hex>`), then an edit of that document (the next run must refuse `skill.drift` before the
spawn). Nothing is left to implement, so no workhorse is dispatched for it on this host — this host has no
podman, so the criterion cannot be seen here at all. The row stays parked until that one deploy, and this
paragraph is its brief.

### 2026-10-21 12:30Z — criteria 1-5 re-measured at HEAD `b4d5bb4`, criterion 6 unchanged

- c1 `project_init_generates_and_never_copies` — **1 passed / 0 failed**
- c2 `project_init_refuses_a_credential` — **2 passed / 0 failed**
- c3 `skills_are_materialised_read_only` — **1 passed / 0 failed**
- c4 `skill_drift_refuses_before_spawn` — **2 passed / 0 failed**, and reddened by the orchestrator's own mutation
  (`coder_skills.rs:128` `if found != pinned` → `if false && …`): **1 passed / 1 failed** on
  `a_drifted_skill_refuses_before_the_agent_spawns` with the untouched-pin control green; restored byte-identical
  (`sha256sum -c` OK, file touched) and green again.
- c5 run for real, not inferred: `pii project init <fresh dir>` exited 0 and wrote exactly the seven generated
  paths (`.pii/policy.toml`, `.pii/settings.toml`, `.pi/APPEND_SYSTEM.md`, `.pi/settings.json`,
  `.claude/CLAUDE.md`, `.claude/settings.json`, `.gitignore`); `grep -rE 'auth\.json|_API_KEY=|credentials'` over
  `.pii .pi .claude` returned nothing (rc 1).

Criterion 6 is the only unmeasured one, unchanged: one deploy of HEAD to lnx (shared brief with spec 227
criterion 7). The deploy binary note is recorded there — `target/release/pii` was built at `7023cd9`, so
`health.build.revision` will read `7023cd9`, not `b4d5bb4`, unless it is rebuilt.

### 2026-10-21 18:20Z — criterion 6's reading is pre-registered, not taken

`_docs/05-operations/DEPLOY-CARD-2026-10-21-skill-pin.md` now carries the exact sequence for this criterion and
spec 227's criterion 7 (one reading, shared): install HEAD with `scripts/deploy-lnx-test.sh` and read the
`READING build:` line; `pii skill add` with `via daemon` on stderr (a `via store <path>` line is a stop — it would
prove the store half and leave the daemon half unmeasured); the pinned graph with
`skills = [{ name, sha256 }]`; `coder.spawned` carrying `skill=`/`sha256=` read from the daemon's JSON event line,
not from `run trace`, which does not print those fields; then `pii skill edit` and a re-run still pinned at the old
hash, expecting the `skill.drift` event to carry `pinned <old> found <new>` with no `coder.spawned`. Nothing in it
has been run: the deploy restarts lnx's daemon, and the operator switch in `_tmp/autonomy/control` remains
`PAUSED`.

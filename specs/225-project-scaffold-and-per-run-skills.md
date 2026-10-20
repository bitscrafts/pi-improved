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

# Spec: the coder node gets its skills from the daemon's own store

## Overview
Spec 225 built the shape — a `SkillSource` trait, a per-run skills directory, `skill.drift` — and then left it
unwired. Measured on `89eacc4`:

    $ grep -rn "with_skills\|with_runs_dir" --include=*.rs crates/
    crates/pii-server/tests/skills_are_materialised_read_only.rs:123:  .with_skills(Arc::new(skills))
    crates/pii-server/tests/skills_are_materialised_read_only.rs:124:  .with_runs_dir(runs.path().to_path_buf()),
    crates/pii-server/tests/skill_drift_refuses_before_spawn.rs:104:  .with_skills(Arc::new(skills))
    crates/pii-server/tests/skill_drift_refuses_before_spawn.rs:105: .with_runs_dir(runs.path().to_path_buf()),
    crates/pii-server/src/node_kinds/nodes/coder.rs:123:  pub fn with_skills(…)
    crates/pii-server/src/node_kinds/nodes/coder.rs:130:  pub fn with_runs_dir(…)

Every call site is a test or the definition. Both registrations that build the node kind in production —
`crates/pii-server/src/engine/registry.rs:64` and `:184` — call `CoderFactory::new(Arc::clone(&log))`, whose
defaults are `skills = NoSkills` and `runs_dir = None`. So on a deployed daemon a coder node with a pinned skill
refuses it as MISSING, and `coder.spawned` can never carry `skill=<name> sha256=<hex>`. Four tests are green
because they hand the factory a source production never hands it; the value is asserted in a test and never read
by the live path (DO-NOT-REPEAT 1.50).

The live registry already receives the store: `registry(…, memory: Arc<pii_store::MemStore>, …)`. The reader
exists too: `MemStore::doc(&self, id: &str) -> Result<Option<Vec<u8>>>` and `MemStore::doc_ids()`, the same pair
`pii skill show` walks to find `DocKind::Skill` documents (`crates/pii-cli/src/cmd_skill.rs:230,247`). Nothing
new has to be invented — one adapter and two call sites.

## Requirements
1. `StoreSkills` implements `SkillSource` over `Arc<pii_store::MemStore>`, resolving a skill **name** by exactly
   the rule `pii skill show <name>` uses — walk `doc_ids()`, decode, keep `DocKind::Skill`, match the name — so
   there is ONE naming rule in the tree, not two. A name the store does not hold returns `None`, and the node
   then refuses it as missing (spec 225 R3), never as empty text.
2. The live `registry()` wires it: `CoderFactory::new(log).with_skills(Arc::new(StoreSkills::new(memory)))
   .with_runs_dir(<store_dir>/runs)`, where `<store_dir>` is `cfg.store_path.parent()`. `lint_registry*` may keep
   `NoSkills` — a lint must not read the store — and that choice is stated in a comment beside it.
3. The runs dir is created on demand and the materialised skill text is written there, outside the project, and
   bound read-only at `.pi/skills` and `.claude/skills` (spec 225 R3, unchanged).
4. `coder.spawned` carries `skill=<name> sha256=<hex>` for each materialised skill, read from the LIVE path: the
   test builds the registry the daemon builds and reads the journalled event, never the factory in isolation.
5. A stored skill whose text no longer hashes to its pin refuses BEFORE any container argv is built: the node
   fails with `skill.drift`, the run's terminal error names the skill, and the fake podman journal holds no `run`
   line.
6. The per-spawn env file stops using `std::env::temp_dir()` (`coder.rs:269`, red under `lint-scratch`): it is
   created under the same runs dir, `create_new` as today, and the existing "second spawn never reuses a path"
   test still passes unchanged.

## TDD Contract
- `crates/pii-server/tests/live_registry_supplies_the_skill_source.rs` builds the node kind through
  `pii_server::engine::registry::registry(...)` — the function the daemon calls — with a `MemStore` holding one
  pinned `DocKind::Skill` document, a fake podman that journals its argv, and a fake agent binary. It asserts:
  (a) the materialised file exists under `<store_dir>/runs/<run>/skills` and NOT under the project;
  (b) its bytes equal the stored document;
  (c) `coder.spawned` names `skill=<name> sha256=<hex>` with the pin's hash;
  (d) the podman argv carries the read-only bind for both `.pi/skills` and `.claude/skills`.
  Control in the same file: a store WITHOUT the skill makes the node refuse as missing, and the journal holds no
  `run` — so a green (a)-(d) cannot come from `NoSkills` being ignored.
- `crates/pii-server/tests/live_registry_refuses_a_drifted_skill.rs` stores the document, then edits its bytes
  under the same id, and asserts the node fails with `skill.drift` naming the skill, the run's terminal error is
  that string, and no container `run` was journalled. Control: the untouched store spawns (one `run` line).
- RED first: on `89eacc4` both files must fail, and the way they fail is the finding — the first on
  `coder.spawned` carrying no `skill=`, the second by spawning a drifted document instead of refusing.

## Exit Criteria
- [ ] `cargo test -p pii-server --test live_registry_supplies_the_skill_source`
- [ ] `cargo test -p pii-server --test live_registry_refuses_a_drifted_skill`
- [ ] `grep -q 'with_skills' crates/pii-server/src/engine/registry.rs`
- [ ] `! grep -q 'env::temp_dir()' crates/pii-server/src/node_kinds/nodes/coder.rs`
- [ ] `CARGO_TARGET_DIR=_tmp/tools-gate cargo run -q -p xtask -- lint-scratch`
- [ ] `CARGO_TARGET_DIR=_tmp/tools-gate cargo run -q -p xtask -- lint-observability`
- [ ] on lnx, after a deploy: `pii run start` on a coder graph with a pinned skill journals `coder.spawned`
      with `skill=<name> sha256=<hex>`, and the same graph with an edited document refuses with `skill.drift`

## Guardrails
- Do NOT make a production `SkillSource` that reads `$HOME`, a `.pi/skills` file, or anything outside the store:
  the store is the one source, which is what R1's single naming rule buys.
- Do NOT weaken the drift check to "hashes differ but proceed", and do NOT let the missing-skill path fall through
  to a spawn with no skills — a refusal is a result.
- Do NOT leave a second naming rule behind: if `pii skill show` and `StoreSkills` disagree about what a skill is
  named, fix the shared resolver, not both copies.
- Do NOT close this on the seven unit criteria alone. Criterion 7 is live and needs a deploy; until it is
  measured the item is `needs-review`, and if M0 (spec 226) still refuses the container start, that is the
  reason, stated as such.

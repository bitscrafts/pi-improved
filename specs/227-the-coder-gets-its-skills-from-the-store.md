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
7. `lint-observability` is green on the WHOLE tree, and that is in scope here. Measured by the orchestrator on
   HEAD `90cd395` (a clean worktree, none of this item's WIP): the lint was already RED with 11 findings that
   earlier items (217, 219, 225 and the CLI commands) left behind, so criterion 6 could not pass on any work
   this spec asked for. The first attempt (session `32d50491`) met criteria 1-5 and failed only here, with the
   same 11 findings. Clear each one by its cause, never by loosening the lint:
   - `crates/pii-observe/src/names.rs`: `CODER_SPAWNED`, `CODER_EXITED`, `CODER_ENV_NAMES`, `CODER_ENV_REFUSED`,
     `SKILL_DRIFT` are missing from `ALL` - add them (the unit test that checks `ALL` for duplicates must stay).
   - raw print macros over budget in `crates/pii-cli/src/{cmd_ctl.rs, cmd_graph_installed.rs, cmd_project.rs,
     cmd_skill.rs, main.rs}` and `crates/pii-server/src/engine/registry.rs`: a CLI command's stdout IS its
     product, so a budget line in `xtask/observability-allowlist.txt` (`path  # reason  # N`) with a real reason
     is the expected fix there; a print in the daemon (`registry.rs`) that reports an event goes through the
     pii-observe seam instead. Do NOT change `xtask/src/lint_observability.rs`.

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
- Do NOT edit the lint, its exemption registry, or a budget without a `# reason` that a reader can check: R7 is
  cleared by fixing the findings, not by moving the bar.
- Do NOT close this on the seven unit criteria alone. Criterion 7 is live and needs a deploy; until it is
  measured the item is `needs-review`, and if M0 (spec 226) still refuses the container start, that is the
  reason, stated as such.

## Closure
Supervisor, 2026-10-20. Criteria 1-6 are green on the committed tree, per the orchestrator's row note. M0 now
passes on lnx (spec 230, run r258), so the container start no longer blocks criterion 7. Criterion 7 is blocked
by something else: production has no way to put a skill into the daemon's store. These readings were taken
against the lnx daemon, pid 3539364, binary installed 2026-10-20T12:38:35:
- a copy of `/run/user/1000/pii/memory.redb` holds zero skills (`pii skill list --all --dir <copy>` prints nothing)
- outside tests, only `pii skill edit` constructs `DocKind::Skill`, and it edits an id that already exists
- `pii skill` against the live dir fails with `Database already open. Cannot acquire lock.`; with no `--dir` it
  looks in `/run/user/1000/pii/default/memory.redb`, which is the wrong store
The criterion stays as written, not weakened. Spec 233 (row A101) adds `pii skill add` through the daemon, and its
live criterion runs this one. A95 stays `needs-review` until 233 is deployed and this criterion is measured.

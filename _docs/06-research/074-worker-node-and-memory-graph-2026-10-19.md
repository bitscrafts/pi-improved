# 074 — The worker node and the memory graph (2026-10-19)

Scope: two questions only — pii driving coding agents (spec 185) and pii's memory graph (specs 203/204).
Measured state taken as given from the brief (deployed 420fe8a7e946); claims below are checked against the working tree where marked **[read]**.

## 1. Is the five-gap list right?

**Mostly right, but two of the five gaps are worded so their fixes would fail, one is not blocking, and seven are missing.**
Everything below was read in the working tree at HEAD 439f175 (30 commits ahead of the deployed 420fe8a).

### The five, judged

| # | Gap as listed | Verdict |
|---|---|---|
| 1 | spawn missing | **Right, and bigger than stated.** It isn't just a missing call; see M2–M4. `agent_worker.rs` documents a launcher (`Dispatch.launcher`) that nothing runs. |
| 2 | kind not wired | **Right, with a trap: the name is taken.** `worker` is already registered (`engine/registry.rs` → `nodes::worker::WorkerFactory`) as the *supervised-worker pool* node, which uses `sup.invoke(…, 120_000)`. A graph that writes `kind = "worker"` gets the pool node and fails with "worker 'x' is not configured". The agent node needs its own kind name. See M1. |
| 3 | sid not persisted | **Right for claude only, and it can't be fixed the way the list implies.** pi needs no persisted sid: `plan()` already derives a deterministic session file (`<workdir>/.pii-sessions/<stem>.jsonl`), and using the same file again *is* resuming. For claude, the argv has no `--output-format json` and no `--session-id`, so **no sid is ever produced** that could be persisted. The fix is to create the sid before the spawn instead of capturing it afterwards (see §2 step 4). |
| 4 | criteria not fed back | **Half right.** The judging is already written: `decide()` makes a clean exit with a failing or absent criterion FAILED. What's missing is the *runner*: nothing executes criteria, and there's no node config field to hold them. Running them is easy, because `/bin/sh` is already the one argv0 a graph run may exec (`run_scope.rs:113`). |
| 5 | learn still shell | **Real, but not blocking.** `learn_fact()` is a pure function that returns `PutRows`, so wiring it later is one `MemStore` write. A graph can hand a spec to pi and get a verdict without learning anything. Order it after the verdict, not before. |

### Missed gaps

- **M1. Kind name collision** (above). Proposed kind: `coder`.
- **M2. The child gets no credentials and can't be given PATH.** `pii-core/src/childproc.rs`: `run_child` does `.env_clear().envs(scrubbed_env(&live_env(), &[]))`. Only PATH/HOME/LANG/LC_ALL/TERM/PWD/TMPDIR and `PII_*` survive, and every `*_API_KEY` and OAuth token is dropped. `ProcCap::run` (which has the timeout and cancel) **takes no env at all**. `spawn_child` takes env but has no timeout. Worse, `scrubbed_env`'s `push` keeps the *first* value it sees: allowlisted parent vars go in first, so a caller's `PATH` in `extra` is silently ignored. The doc comment says "last write wins"; the code does the opposite. So `plan()` checks credentials in an env map that the spawned child would never receive.
- **M3. The agents as tested are shell aliases, not the binaries.** In the operator shell, `pi` = `pi --append-system-prompt "$(cat ~/.pi/agent/system.md)"` and `claude` = `claude --dangerously-skip-permissions`. `plan()`'s argv has neither. A spawned `claude -p` without `--permission-mode bypassPermissions` can't write files headless. As root it also needs `IS_SANDBOX=1` (`scripts/autonomy/env.sh:17`). A spawned `pi` runs without the system prompt every manual run had. Any "it worked by hand" evidence was gathered with those flags; the planned argv drops them.
- **M4. Graph-run capability scope forbids the spawn.** `run_scope.rs:113` pins `proc_argv0` to `["/bin/sh"]`. `ProcCap::run`/`spawn_child` deny any argv0 outside that scope, so `pi` or `~/.local/bin/claude` is refused before it starts. Either the scope gets the agent's absolute path for this node kind, or the spawn goes through `/bin/sh -c`, which is a script by another name (see §3).
- **M5. Failure must be a port, not an `Err`.** The scheduler retries a retryable `Err` (`worker.rs` relies on this deliberately: `Unavailable` ⇒ retry). If an agent timeout or a failing criterion surfaces as `Err`, the RetryPolicy re-runs a 20–60 min paid dispatch with no new information. `plan()` already returns `Unavailable` for missing credentials, which is a refusal that will be retried. Verdicts and refusals must be `Outcome::Next{port:"failed"|"refused"}`.
- **M6. Re-attach would dispatch twice.** Spec 143 (A70, done) re-runs an *incomplete* node after a daemon restart. A coder node killed mid-flight would restart from zero on a dirty tree, unless the session handle exists **before** the spawn so that the re-run resumes. This is the same issue the shell already hit (`cycle.sh` "session recovered from the worker's log", measured 2026-10-13).
- **M7. The node's verdict is weaker than the loop's.** The shell loop's "done" means criteria pass **and** fresh-gate is green (`state.py` "satisfied by …: the criterion passed and fresh-gate is green"). `decide()` sees criteria only. That's fine for the first slice, but a coder-node PASS must not be read as loop-equivalent until the gate is one of its criteria (see §6).

Not gaps, but true: the brief builder (attempt-2 corrective brief carrying the failing criterion) and the escalation ladder (`escalate()`) are pure and ready. Both are *after* the first verdict, not before.

## 2. Minimum Rust change set: one graph hands a spec to pi and gets a verdict

Six steps in dependency order. Each step is one commit and one test file (or unit-test block), and each has a reading that proves it. The steps are cumulative, so stop after any one and the tree is still coherent. The proposed kind name is `coder` (M1).

### Step 1. `pii-core`: a child can be given env, and the caller's env wins

- `crates/pii-core/src/childproc.rs`
  - `scrubbed_env`: an `extra` pair **replaces** an allowlisted one instead of being dropped (or fix the doc comment and refuse a duplicate key, but don't leave it silent).
  - Add `ProcCap::run_with_env(argv, cwd, env, timeout)`. Make `run` call it with `&[]` so there's still one `Command` site and the F3.8 count is unchanged. Thread `env` into `run_child`.
- Tests (in-file, beside `run_child_scrubs_environment`):
  - `scrubbed_env_extra_overrides_an_allowlisted_path`: parent `PATH=/a`, extra `PATH=/b` → `/b`.
  - `run_with_env_child_sees_the_given_secret_and_no_other`: `/bin/sh -c 'printf "%s|%s" "$ANTHROPIC_API_KEY" "$OLLAMA_API_KEY"'`, with only the first in `env` and both in the parent → stdout `k1|`.
- Reading: `cargo test -p pii-core childproc` lists both tests; the existing scrub test still passes.

### Step 2. `agent_worker.rs`: the argv carries the headless flags; no launcher

- `crates/pii-server/src/node_kinds/nodes/agent_worker.rs`
  - `WorkerSpec` gains:
    - `bin` (absolute path, required);
    - `env_pass: Vec<String>` (variable names copied from the daemon's env);
    - `env_set: BTreeMap<String,String>` (literal pairs, for example `IS_SANDBOX=1`);
    - `system_prompt: Option<String>` (path; pi gets `--append-system-prompt <contents>`);
    - `timeout_s` (default 3600);
    - `criteria: Vec<String>` (required and non-empty; `decide` already fails an empty set, so refuse it at build instead).
  - `Dispatch { launcher, argv }` becomes `Dispatch { argv, env }`, with `argv[0] = bin`.
  - claude argv: `bin --permission-mode bypassPermissions --output-format json --model M` + (`--session-id S` | `--resume S`) + `-p BRIEF`.
  - The credential check reads `env_pass`. A named variable that's absent is a refusal: `WORKER_START_REFUSED` naming **which** variable is missing.
- Tests (in-file `#[cfg(test)]`):
  - `claude_first_attempt_mints_a_session_id_and_is_headless`
  - `claude_second_attempt_resumes_the_same_sid`
  - `pi_argv_carries_the_system_prompt_and_its_session_file`
  - `a_missing_named_credential_refuses_with_the_variable_named`
  - `the_dispatch_env_holds_exactly_env_pass_plus_env_set`
- Reading: `cargo test -p pii-server agent_worker` passes. `grep -n launcher agent_worker.rs` → nothing.

### Step 3. The session handle exists before the spawn (sid persistence)

- **Rule: the handle lives in the workdir, beside the spec's stem, and is written before the child starts. If it exists, the dispatch resumes.**
  - pi: `<workdir>/.pii-sessions/<stem>.jsonl`. This is already the rule. pi creates the file; the same path on attempt 2 resumes.
  - claude: `<workdir>/.pii-sessions/<stem>.claude-sid`, holding one UUID.
    - Absent → mint one (UUIDv4-shaped from 16 bytes of `/dev/urandom`; there's no `uuid` crate in the workspace and claude only validates the shape), write it atomically, pass `--session-id`.
    - Present → `--resume <it>`.
- Why here and not in the journal or memory: it survives a daemon crash *mid-spawn* (M6). It's readable by the shell loop and by a human. It needs no new store. And re-attach (spec 143) then resumes instead of restarting.
- A pure `fn session_handle(existing: Option<String>, mint: impl FnOnce() -> String) -> (String, bool /*resume*/)` in `agent_worker.rs`; the file I/O lives in the node (step 5).
- Test: `sid_is_written_before_spawn_and_reused_on_attempt_two` in `crates/pii-server/tests/coder_node.rs` (step 5's file). The fake agent (a test-tempdir script, which is fine in development) records its argv and checks that the sid file **already exists when it starts**.
- Reading: after one dispatch, `cat <workdir>/.pii-sessions/<stem>.claude-sid` prints a UUID equal to the `--session-id` in the fake's argv log. After the second dispatch, the argv log shows `--resume <same>`.

### Step 4. Criteria run, and `decide` gets them

- In `agent_worker.rs` (the pure part), plus the node:
  - After the worker exits, each `criteria[i]` runs as `ctx.proc()?.run(["/bin/sh","-c",cmd], workdir, criteria_timeout)`. `/bin/sh` is already in scope, so no capability change is needed.
  - Each run maps to `CriterionRun { command, rc }`; a timeout is `rc = 124`, which `ProcOutput` already reports.
  - **Every** criterion runs, not just up to the first failure, so the evidence is complete. `decide()` still names the first failing one.
- This is how criteria failures reach the Outcome: `decide(worker_rc, report, &runs)` → `Verdict` → step 5 maps it to a port and value. The worker's stdout is kept only as `report` (last 4 KiB). It **never** decides anything.
- Unit test: `a_clean_exit_with_a_failing_criterion_is_failed_and_names_it`. `decide` already has this shape; add the runner-level version with a scripted runner trait (same pattern as `heal.rs`'s `GateRunner`).

### Step 5. The `coder` kind, registered; failures are ports, not errors

- `crates/pii-server/src/node_kinds/nodes/coder.rs` (new, ~150 lines; keeps `agent_worker.rs` pure):
  - `CoderFactory`:
    - `kind() = "coder"`, `effects = MUTATES_EXTERNAL`, `required_caps = SPAWN | PROC`;
    - ports `passed | failed | refused`.
  - `run()`:
    - `WorkerSpec::from_cfg` → session handle (step 3) → `plan` → `ctx.proc()?.run_with_env(...)` → criteria (step 4) → `decide`;
    - returns `Outcome::Next{port, value}` with value `{verdict, failing, worker_rc, sid|session, criteria:[{command,rc}], report_tail}`;
    - **never `Err` after build** (M5): a refusal is port `refused`, and a timeout or failure is port `failed`.
- `nodes/mod.rs`: `pub mod coder;`. `engine/registry.rs`: register `CoderFactory` in `registry()` **and** in the lint registry.
- `engine/run_scope.rs`: `proc_argv0` = `/bin/sh` + `EngineConfig.coder_bins` (absolute paths from daemon config; empty by default, so the default is still refuse).
- Tests, `crates/pii-server/tests/coder_node.rs`, using a fake agent script plus a temp workdir:
  1. `coder_passes_only_when_its_criteria_pass`: the fake writes `DONE`; criterion `test -f DONE` → port `passed`.
  2. `coder_clean_exit_with_failing_criterion_is_failed`: the fake exits 0 and writes nothing → port `failed`, `value.failing == "test -f DONE"`. **This is the test that fails if criteria aren't fed back.** A node that trusted the worker's exit would report `passed`.
  3. `coder_failure_is_not_retried`: graph `retry.max = 3`; the fake increments a counter file → counter == 1.
  4. `coder_bin_outside_scope_is_refused_not_spawned`: `bin` not in `coder_bins` → port `refused`, and the fake's counter file is absent.
  5. step 3's sid test.
- Reading: `cargo test -p pii-server --test coder_node` shows 5 passed. `pii graph lint` accepts a graph with `kind="coder"`.

### Step 6. Live: one graph, real pi, a verdict both ways

- `graphs/coder-smoke.toml`: one `coder` node.
  - `agent=pi`, `bin=<abs path of pi>`, `model=<cheapest working>`, `env_pass=[the provider key]`.
  - `workdir` = a scratch git worktree under `_tmp/`.
  - `spec` = a three-line spec: "create HELLO containing `hi`".
  - `criteria = ["grep -qx hi HELLO"]`.
  - The daemon config lists pi's path in `coder_bins`.
- Readings, on the **installed** binary:
  - (a) `pii graph run coder-smoke` → the journal's `NodeFinished` has port `passed` and `value.criteria[0].rc == 0`, and `<wt>/.pii-sessions/<stem>.jsonl` exists.
  - (b) The same run with `criteria=["grep -qx bye HELLO"]` → port `failed`, `value.failing` names it, and the pi session file is reused, not recreated: same inode, more lines.
  - (c) `ps` during the run shows `pi --session …` as a child of `pii-server`, with no `bash` launcher in between.
- Learning (`learn_fact` → `MemStore`) is step 7, after this. It's one write in `coder.rs` plus `coder_verdict_writes_one_outcome_fact`. It isn't needed for the verdict.

## 3. 185 vs production: script or in-process env?

**Spawn directly, with the environment set in-process and stated in full. Do not use a script.**

The argument:

1. **What 0d is for.** `scripts/autonomy/env.sh` gives the reason: "a dispatch that worked only because the calling shell happened to have the right PATH is not reproducible." The invariant is *the child's environment is explicit and not inherited*. A script was just how a shell dispatcher could meet it. A Rust spawn that does `env_clear()` and then sets declared pairs meets it more strictly than any script, because step 1's test can **prove** what the child sees. A script's environment is only as explicit as whoever last edited it.
2. **A script can't ship.** The installer exports three binaries from a clean tree; nothing under `scripts/` is deployed. A launcher path in a graph is a dependency the installer doesn't satisfy. That's the same defect commit 439f175 just removed from the index graph.
3. **A script would break the capability model.** Spawning through a script means `/bin/sh` (or the script path) as argv0. `/bin/sh -c` is a universal exec, so `proc_argv0` would no longer say anything about which agent can run. Direct spawn keeps argv0 = the agent's absolute path, scoped by `coder_bins`.
4. **A script wouldn't even work.** `run_child` scrubs every `*_API_KEY` before the script starts (M2). It would get no credentials unless pii passed them explicitly, and at that point pii is already setting the env in-process, so the script adds nothing but a file that isn't shipped.
5. **What the script did that still has to happen** (PATH for the agent's own children, such as cargo; `IS_SANDBOX=1`; `CARGO_BUILD_JOBS=8`; `FRESH_GATE_TARGET`) becomes `env_set` in graph data. It's visible, diffable and linted, instead of being tribal knowledge in `env.sh`. This depends on step 1's override fix: today a declared `PATH` is silently ignored.

**Secrets:** graph files name variables (`env_pass = ["ANTHROPIC_API_KEY"]`); they never hold values. Values come from the daemon's own environment, set by the service unit's `EnvironmentFile`, which the installer owns and the operator fills. A named variable that's missing is a `refused` port with the name, never a spawn.

**What 185's guardrail should say instead** (it replaces line 35 and line 60's "Do NOT spawn an agent without a script…"):

> **0d (production form).** A coder dispatch's environment is declared, never inherited: the child starts from `env_clear()`, receives the scrubbed allowlist, then exactly the variables the node names (`env_pass`, values from the daemon's environment) and the literal pairs it sets (`env_set`), with a declared pair overriding an allowlisted one. `argv[0]` is an absolute path listed in the daemon's `coder_bins`; anything else is refused before spawn. A named variable that is absent refuses the dispatch with `WORKER_START_REFUSED` naming the variable. No dispatch depends on a file the installer does not ship. Development-only launch scripts are allowed under `scripts/`, but no graph under `graphs/` may reference one.

The last sentence can be enforced by the gate, using the same check as 439f175: `grep -n 'scripts/' graphs/*.toml` must be empty.

## 4. Entity-only indexing, and the smallest link design

### Two corrections to the measured state, both measured just now

1. **The link verb has a parser. It just doesn't accept `--rel`.** On the deployed `~/.local/bin/pii`, in a `mktemp -d` scratch dir:
   - `pii memory link A B --kind contains --source file:a.rs:1 --dir D` → rc 0, returns `r-a3f6…`, and `neighbours A` lists it.
   - `pii memory link A B --rel contains --dir D` → the entity usage, because `--rel` was an unknown flag and `--source` was mandatory.

   `cmd_ctl.rs:191` grouping `link` with `entity` is just routing; it isn't the defect. The uncommitted diff in `cmd_entity.rs` (`"--kind" | "--rel"`, plus a default `actor:cli` source) is the entire parser fix. It's three lines, not a new parser. Spec 204 / A72 is sized for a gap that doesn't exist.
2. **Relationships don't feed the search graph lane, and the lane already fires.** There are two unrelated graphs:
   - The **entity graph**: `<dir>/memory/graph.json`, `put_relationship`, read by `neighbours`. relationships=0 lives here.
   - The **concept graph**: `memory.redb` table `mem_edge`, written by `write_fact` / `memory.put` with `concepts`, read by `graph_lane::graph_search`. No line of `search.rs` reads `graph.json`.

   Live reading, `pii memory search "worker dispatch session"`:
   - top-level `lanes = [bm25, vector, graph]`. This list is always present; it means "ran", not "hit" (`graph_lane.rs:11`);
   - and a hit, `5d38c2ef… module pii-server::engine::conn_heartbeat_tests contains …`, whose **per-hit** lanes are `['graph']` only.

   The module documents that `cmd_index.rs::doc()` writes with path and symbol concepts already activate the lane. "Lanes are always [bm25,vector]" was a reading of per-hit lanes on queries with no concept overlap. It isn't a property of the store.

### Is entity-only indexing worth anything today?

**Little on its own.** 1415 `name → kind, file:line` rows are a ctags-equivalent: an exact-name lookup in one call. grep and graphify already do this, and the rows go stale on the first commit (there's no re-index-on-change and no deletion of removed symbols). Their value is as **anchors**. They're the stable ids that a link, a provenance line, or a learned outcome ("spec 185 touched `agent_worker::plan`") can point at. The value of the work so far is in the module *documents*, which are searchable and lane-active. The entities only matter if something links to them.

### Smallest link design that makes links matter for search

- **Verb:** `pii memory link <from-id> <to-id> --rel <kind> [--source S]... [--attr k=v]... [--dir D]`. `--kind` stays as an alias, and the source defaults to `actor:cli`. This is exactly the uncommitted diff; there's no new surface.
- **Edge kinds: two, and refuse the rest until something consumes them.**
  - `contains`: crate→module and module→symbol; source = the `file:line` of the symbol.
  - `depends_on`: crate→crate; source = `file:crates/<c>/Cargo.toml`.

  No `calls` and no `uses`: pii has no parser that can produce them honestly, and an edge kind with no producer only invites invented edges.
- **The bridge (the actual activation):** when `pii index code` writes a `depends_on` x→y, it adds y's crate token to the **concepts** of every module document of x. A link becomes a concept co-occurrence, which is the only thing the lane reads. This needs no search-code change and no second traversal. The alternative, teaching `graph_search` to walk `graph.json`, is a new lane with new ranking, so it's bigger than "smallest".
- **The one test** (`crates/pii-cli/tests/memory_link.rs`, beside the existing two):
  `a_depends_on_link_lets_the_graph_lane_recall_a_module_bm25_cannot`.
  1. Index a fixture with crate `alpha` (module `alpha::m`, text "module alpha::m contains run") and crate `beta`, where alpha depends_on beta.
  2. Search for `beta`.
  3. Assert that a hit for `alpha::m` exists **and its per-hit `lanes == ["graph"]`**. `beta` isn't in its text, so bm25 can't produce it; the fixture has no embedder, so vector can't either.
  4. Then remove the dependency, re-index, and assert the hit is gone.

  Assert on the hit's lanes, never on the top-level `lanes`: that list includes `graph` even when the lane hit nothing, so a test on it passes with the feature broken.

## 5. needs= column for plan.tsv
_pending_

## 6. What must not be claimed production-ready tomorrow
_pending_

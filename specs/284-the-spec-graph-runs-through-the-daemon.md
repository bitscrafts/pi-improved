# Spec 284: the shipped spec graph runs through the daemon's engine, end to end

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composability, level 1 of 4. The user's verdict: *"if 'has never actually run through the daemon' our composability is useless."* Measured 2026-10-26: `Engine::start_run` has run a `call` only with a one-node toy sub-graph (`daemon_journals_calls.rs`); the real `graphs/spec.toml` — `code → verify → docs (call) → reverify → done`, two coder nodes, two judges, a `[limits]` budget — has run only through the **library** runner (`spec_graph_documents_after_verify.rs`), which builds its own `Ctx`, its own `Budget::unlimited()` and its own registry. Everything the daemon adds — the engine's budget from `[limits]` (`runs.rs:59`), its registry, its capability grant, its journal, the `woven`/`graph.call` events written by `run::run` — has never been exercised on the shipped composed graph.

This spec drives that graph through **`Engine::start_run("spec", input)`**, the function the daemon's `run start` reaches, with stand-in agents (nothing spends), and asserts what the journal says.

## Verified Premises
- **CONFIRMED** (read 2026-10-26) `Engine::start_run(name, &input)` at `crates/pii-server/src/lib.rs:443`; `daemon_journals_calls.rs` shows the harness: `Engine::new(EngineConfig { graph_dir, store_path, model: "scripted:<fixture>", sandbox: Off, … })`, then `settle()` polling `engine.store.get_run` until `Succeeded`/`Failed`, then `store.read_journal`.
- **CONFIRMED** `graphs/spec.toml` carries `[limits] max_micro_usd = 1000000` and a `docs` call pinned by sha256 to `graphs/docs.toml`; the coder charges the run's budget after each run (spec 276) and caps a claude run by what is left.
- **CONFIRMED** the workspace forbids `unsafe_code` (`Cargo.toml:60`), and `std::env::set_var` is `unsafe` in edition 2024: **a test cannot put a fake agent on `PATH` in its own process.**
- **MEASURE FIRST** how the engine's `coder` resolves the agent binary directory (the project rule: from `PATH`) and whether `env_pass = ["PATH", …]` copies the **daemon process's** `PATH`. If it does, the test must run the engine in a **child process** whose environment has the fake bin dir first (re-exec `std::env::current_exe()` with `--exact <test> --nocapture` and `PATH=<fake>:…`, a marker env var telling the child to run the real body). Read the way existing tests already do this before inventing one (`grep -rn "current_exe" crates/*/tests`).
- **MEASURE FIRST** the exact `RunEvent` variants the journal uses for node start/finish and the field names on `coder.spawned` / `coder.exited` (`session`, `claude_cost_usd`, the argv); assert on what is there, do not invent names.
- **MEASURE FIRST** whether the engine's capability grant admits the judge's `git` and `/bin/sh` and the agent binaries the way the library test's `root_ctx` did; if it does not, that is a **finding** (a daemon-only failure) — report it, do not widen the grant to make the test pass.

## Requirements
### Functional
- **R1. One new test file** `crates/pii-server/tests/spec_graph_runs_through_the_daemon.rs`. It builds an `Engine` whose `graph_dir` is the REAL tracked `graphs/` directory (`CARGO_MANIFEST_DIR/../../graphs`), a throwaway git repo holding a spec with one criterion (the helper shape of `spec_graph_documents_after_verify.rs`: copy what it needs; do not edit that file), a scratch bin directory holding a fake `pi` and a fake `claude`, and starts `spec` with `agent = claude`, `max_budget_usd = 3.00`, `FAKE_COST = 0.40` per coder run.
- **R2. The happy run, asserted from the engine's own journal** (not from the fake agent's log): (a) the run reaches `Succeeded`; (b) the nodes ran in the order `code`, `verify`, the call's head, the docs coder, `reverify`, `done` (read the call's inlined node ids from the journal, do not hard-code a guess); (c) exactly **one** `graph.call` observation, naming graph `docs` and the **full digest of `graphs/docs.toml` as read from disk now**; (d) two `coder.spawned` events carrying **two different sessions**; (e) the docs coder's argv names the docs brief and not the implementation brief; (f) **the budget is shared through the call**: the first coder's `--max-budget-usd` is `1` (the `[limits]` ceiling beats the input's 3.00), the second — the docs coder INSIDE the called graph — is **`0.6`** after the first charged 0.40. The last is the proof that an inlined call shares the host's budget through the daemon, and it is read from the argv the fake agent recorded.
- **R3. Controls, each through the same engine:** (1) a docs agent that deletes the file the criterion checks → the run ends `Failed`, the journal shows `reverify` failing and `done` never reached; (2) `docs_brief` absent from the input → the run refuses **before any agent spawns** (`graph.call.need`, zero `coder.spawned`), spec 267 through the daemon; (3) a scratch copy of `graphs/` whose `docs.toml` has one byte changed → `start_run` refuses with the pin mismatch naming both digests and starts nothing; (4) the same run with the `[limits]` table of the **scratch copy** set to `100000` micro-USD → after the first coder charges 0.40 the run fails on the exhausted budget and **no docs coder spawns** (the engine's between-node check, spec 276).
- **R4. A defect is a result, not something to fix here.** If the test shows the daemon path differs from the library path (a missing capability, a missing event, a budget not shared), the test stays and fails with the evidence; the worker reports it and changes no source.

### Non-Functional
- Test-only change: no file under `crates/*/src`, `graphs/` or `scripts/` is edited. Both files stay under the 450-line gate: the tests live in `crates/pii-server/tests/spec_graph_runs_through_the_daemon.rs` and the helpers they use (the scratch bin dir holding the fake `pi` and `claude`, the child-process re-exec, the throwaway git repo, the `Engine` builder) live in `crates/pii-server/tests/spec_graph_daemon/mod.rs`, included with `mod spec_graph_daemon;` — a directory module is not a test target of its own (the same shape `crates/pii-graph/tests/common/mod.rs` already uses), and it must carry that harness, not be an empty placeholder. Create no third file: the row that commits this work names exactly these two paths, and the cycle refuses a change its row does not list. Nothing spends: no real agent, no network.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test spec_graph_runs_through_the_daemon 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test "$(cargo test -q -p pii-server --test spec_graph_runs_through_the_daemon -- --list 2>/dev/null | grep -c ': test$')" -ge 5`
- [ ] `grep -q 'Engine' crates/pii-server/tests/spec_graph_runs_through_the_daemon.rs && grep -q 'start_run' crates/pii-server/tests/spec_graph_runs_through_the_daemon.rs`
- [ ] `grep -q '"0.6"\|0\.6' crates/pii-server/tests/spec_graph_runs_through_the_daemon.rs`
- [ ] `cargo test -q -p pii-server --test spec_graph_documents_after_verify 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test daemon_journals_calls 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `git diff --quiet -- crates/pii-server/src crates/pii-graph/src graphs`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `test -f crates/pii-server/tests/spec_graph_daemon/mod.rs`

## Files
- `crates/pii-server/tests/spec_graph_runs_through_the_daemon.rs`
- `crates/pii-server/tests/spec_graph_daemon/mod.rs`

## Guardrails
- Do NOT edit any source, any graph, or any existing test. Do NOT widen a capability, loosen an assertion, or `#[ignore]` a test to make it pass. Do NOT call `set_var`. Do NOT commit.
- Assert on the journal the engine wrote, never on a string the test itself produced.

## Follow-ups
Spec 285 (failure matrix), spec 286 (mutation sweep), spec 287 (the same graph through the **real** devbox daemon process with a stand-in agent).

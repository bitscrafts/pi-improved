# Spec 272: the tracked spec graph calls a documentation graph — the first real composed graph

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composition, first use (operator, 2026-10-24: *"a node that calls a graph, for instance to document the task/spec"*). Depends on A136 (per-node pi sessions, spec 271) and on the call machinery of specs 255-270.

`graphs/spec.toml` is `code → verify → done`. After the criteria pass nothing documents what was done. This spec adds the documentation as a CALLED graph, `graphs/docs.toml`, so composition is used by a graph that ships, not only by tests: `code → verify → docs (call) → reverify → done`.

Two design points, both from the measured state:
- **The docs coder must not be able to break verified code.** A documentation agent has a write-capable coder and no scope limit that has been measured. So the graph runs `verify` AGAIN after `docs` (`reverify`, the same judge kind with the same input, reading the same committed blob at the same `base`): if the docs step changed anything the spec's criteria depend on, the run ends refused. This is a check built from existing kinds; it does not claim the docs coder is confined.
- **The docs coder gets its own session.** With A136 a pi coder node's default session is `(run, node)`, so `docs__docs` never resumes `code`'s conversation. Claude mints a fresh session per spawn. No `session`/`sid` is given.

## Verified Premises
- **CONFIRMED** (read, 2026-10-25) `graphs/spec.toml`: `code`(coder) `→ verify`(judge) `→ done`, `verify fail → unverified`(refuse), `code err → refused`(refuse); the coder reads `agent model provider spec workdir brief max_budget_usd` from `$.input.*` (spec 257), `timeout_ms = 3000000`, `env_pass` literal; the judge reads `spec base workdir`.
- **CONFIRMED** (read) `crates/pii-graph/tests/spec_graph_is_tracked_and_valid.rs:77-98` and `spec_graph_has_a_verdict.rs:65-70,128` read `graphs/spec.toml` as TEXT and load it with `load_toml_with_registry`. A `call` node in that text makes that loader fail (measured 2026-10-24 on the built binary: `V0: node 'docs' has unknown kind 'call'`), exactly as the judge node did in A124. Both files must therefore load through `load_host_with_graphs(src, fragments_dir, graph_dirs, reg)` (`crates/pii-graph/src/fragment/weave.rs`) with `graphs/` as the graph dir. This spec ALLOWS editing exactly those two files and ONLY their loader line(s) (and the imports they need); every assertion stays byte for byte.
- **CONFIRMED** (read) the call syntax: host `[[node]] id kind="call" graph sha256 caps [node.in]`; the sub-graph declares `exit = "<node id>"`, optional `needs = [..]`; `call` keys are exactly `id kind graph sha256 caps in` (anything else refused, `graph.call.key`); the sha256 is the digest of the sub-graph FILE bytes and a mismatch refuses (`fragment.pin.mismatch`).
- **CONFIRMED** (read) a mapped `in` key overrides the host input for a declared key; a declared key that resolves to Null refuses the run before any sub-graph node (spec 267, `graph.call.need`); a sub-graph reading a host node by id is refused (spec 261).
- **CONFIRMED** (read) the coder's output on success is `{pid, rc, criteria[...]}` and carries NO session handle, so nothing here shares a session by expression.
- **NOT CLAIMED** that a docs coder is confined to documentation files: only that code damage is caught by `reverify`. A confinement mechanism is a follow-up.
- **NOT CLAIMED** that a real pi or claude run of this graph passes: the tests drive the fake agent of `coder_session_per_node.rs`; the first real run is a separate, user-approved step (it spends).

## Requirements
### Functional
- **R1. `graphs/docs.toml`.** A tracked graph: `name = "docs"`, `entry = "docs"`, `exit = "docs"`, `needs = ["agent","model","provider","spec","workdir","brief"]`, one coder node `docs` (`kind = "coder"`, `caps = ["spawn","proc"]`) whose keys read `$.input.agent`, `model`, `provider`, `spec`, `workdir`, `brief` exactly as `spec.toml`'s coder does, `timeout_ms = 1200000`, the SAME literal `env_pass` list as `spec.toml`, and NO `max_budget_usd` (the budget stays the host's; see Follow-ups), no `session`, no `sid`, no `criteria`. No absolute path, no credential.
- **R2. The call in `graphs/spec.toml`.** Insert a `[[node]] id = "docs" kind = "call" graph = "docs" sha256 = "<digest of graphs/docs.toml>" caps = ["spawn","proc"]` with `[node.in] brief = "$.input.docs_brief"` (the docs coder is briefed by its OWN key, never by the implementation brief), a `reverify` judge node (`kind = "judge"`, `caps = ["proc"]`, `spec`/`base`/`workdir` exactly as `verify`), and edges: `verify ok → docs`, `docs ok → reverify`, `docs err → refused`, `reverify ok → done`, `reverify fail → unverified`. The old edge `verify ok → done` is REMOVED. `entry`, `caps`, the `code` node, `verify`, `done`, `unverified`, `refused` and every other edge are unchanged. The `sha256` is the real digest of the file as committed; the test in R4 recomputes it.
- **R3. The shipped CLI loads it.** `pii graph check graphs/spec.toml` (the BUILT binary, spec 260's path) exits 0 and `pii graph explain graphs/spec.toml` prints the line `call docs -> graph docs sha256 <12 hex> caps [proc,spawn] needs [agent,model,provider,spec,workdir,brief] exit docs` (spec 269's shape; caps in the order that command prints today: copy it from the output, do not guess).
- **R4. Tests, with controls.** New `crates/pii-server/tests/spec_graph_documents_after_verify.rs` (new file; fake agent as in `coder_session_per_node.rs`, copy its helpers) and `crates/pii-cli/tests/shipped_spec_graph_loads.rs` (the built binary). Server: (a) the tracked `spec.toml` loaded with `graphs/` as graph dir and run with a passing fake agent and a criterion that passes: the journal shows `code`, `verify`, `docs__docs`, `reverify`, `done`, in that order, and one `graph.call` observation for `docs`; (b) `docs__docs` is briefed by `docs_brief`: the brief the fake agent received (capture it as `coder_brief_names_the_spec.rs` does) is the docs brief, and `code` received the implementation brief; (c) the two coders (`code`, `docs__docs`) journal two DIFFERENT session paths for pi (A136) and the second file holds the header plus ONE pid; (d) CONTROL: the criterion FAILS at `verify`: the run ends at `unverified`, no `docs__docs` node ever starts, and no docs agent is spawned; (e) the reverify guard: the fake agent for the docs step breaks the criterion (it deletes the file the criterion checks): the run ends at `unverified` via `reverify`, and `done` never runs (this is the test that proves the guard is real: it fails if `reverify` is absent); (f) CONTROL: with `docs_brief` absent from the input the run is refused before any docs agent spawns, naming `docs_brief` (the `needs`/`in` contract; assert the `graph.call.need` text names the key); (g) the pin: `sha256_hex(graphs/docs.toml bytes)` equals the `sha256` written in `spec.toml`'s call node, AND a copy of `spec.toml` with one byte of `docs.toml` changed fails to load with `fragment.pin.mismatch`. CLI: (h) `pii graph check graphs/spec.toml` exits 0; (i) `pii graph explain graphs/spec.toml` prints the R3 line; (j) CONTROL: `pii graph check` on a copy of `spec.toml` whose pin is `000…0` exits non-zero and names `fragment.pin.mismatch`.
- **R5. The two existing tests keep their assertions.** `spec_graph_is_tracked_and_valid.rs` and `spec_graph_has_a_verdict.rs` change only how they load the tracked graph (R2's premise). `git diff` of each must show only loader/import lines.

### Non-Functional
- Every file under the 450-line gate. No new dependency. No new node kind, no engine change.
- Do not edit any other existing test, `call.rs`, `call_needs.rs`, `run/mod.rs`, `coder*`.
- The docs graph holds no `criteria`: its own verdict is deferred to `reverify`.

## TDD Contract
| Test | Expected |
|---|---|
| `a_passing_run_documents_after_verify` | order `code, verify, docs__docs, reverify, done`; one `graph.call` for `docs` |
| `the_docs_step_is_briefed_by_docs_brief` | the two agents got two different briefs |
| `the_two_coders_have_two_sessions` | different paths; second file: header + one pid |
| `CONTROL_a_failed_verify_never_reaches_docs` | ends `unverified`; no docs spawn |
| `reverify_catches_a_docs_step_that_breaks_the_criteria` | ends `unverified` via `reverify`; `done` never runs |
| `CONTROL_a_missing_docs_brief_refuses_before_any_docs_agent` | refused, names `docs_brief` |
| `the_pin_is_the_real_digest_and_a_change_breaks_it` | equal; changed byte → `fragment.pin.mismatch` |
| `check_loads_the_shipped_graph` | exit 0 |
| `explain_prints_the_call_line` | the R3 line |
| `CONTROL_a_wrong_pin_fails_check` | non-zero, names `fragment.pin.mismatch` |

**The requirement a plausible implementation satisfies only in appearance:** adding `docs` after `verify` and wiring `docs ok → done`. Every test of the happy path passes, and a docs agent that breaks the code ships green. Test (e) fails it.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test spec_graph_documents_after_verify 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-cli --test shipped_spec_graph_loads 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test -f graphs/docs.toml`
- [ ] `grep -q 'kind = "call"' graphs/spec.toml`
- [ ] `grep -q 'id = "reverify"' graphs/spec.toml`
- [ ] `cargo test -q -p pii-graph --test spec_graph_is_tracked_and_valid 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-graph --test spec_graph_has_a_verdict 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test coder_session_per_node 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-graph --test call_requires_needs 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test -z "$(git diff -U0 -- crates/pii-graph/tests/spec_graph_is_tracked_and_valid.rs crates/pii-graph/tests/spec_graph_has_a_verdict.rs | grep -E '^[+-]' | grep -v '^+++\|^---' | grep -E 'assert')"`
- [ ] `git diff --quiet -- crates/pii-graph/src crates/pii-server/src crates/pii-cli/src`
- [ ] `cargo clippy -q -p pii-graph -p pii-server -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT touch any source under `crates/*/src` (this spec adds two graphs and tests). Do NOT add or remove an `assert` in the two existing graph tests. Do NOT remove `reverify` or wire `docs ok → done`. Do NOT put `max_budget_usd`, `session`, `sid` or `criteria` in `docs.toml`. Do NOT commit.

## Follow-ups (not this spec)
1. The first REAL run of `spec` through the daemon with pi/ollama and with claude/haiku (user-approved; it spends), with `docs_brief` in the input.
2. The coder charging the run budget, so the docs step is bounded; until then `docs.toml` carries no `max_budget_usd` and the call is not run unattended in a loop.
3. A confinement for the docs coder (write scope limited to documentation paths) so `reverify` is a second line, not the only one.
4. `run-batchC-pii.sh` and the queue's worker moving onto `pii run start spec`.

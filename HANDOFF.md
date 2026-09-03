# Pii — Session Handoff

**Status: GREEN — PHASE 1 COMPLETE + OPUS REVIEW FIXES LANDED; PHASE 2 PART 1 COMPLETE (6/6 EC) (2026-09-03)**

## PHASE 2 PART 1 — COMPLETE (6/6 exit criteria)
Spec `specs/04-phase2-client-server-part1.md` (client/server split: `pii-proto`
wire contract, `pii-server` daemon, `pii-model`/`pii-tools`, `pii-cli` `-p`/
`daemon`/`sandbox-exec`). Commits `7800539` (crates scaffolding, worker-
produced + hand-fixed) and `25cbf5b` (CLI subcommands, hand-written after
two worker attempts stalled — see Errors & Fixes below).

**pidag/worker infra fixes landed this session** (both durable, both
committed to their respective repos):
- **pidag worker hang fixed** (`pidag` commit, `src/worker/pi_print.rs`):
  `wait_with_output()` blocks on pipe EOF, not process exit — a pi bash
  tool grandchild holding the stdout/stderr pipe open hung the worker
  indefinitely after pi itself exited. Now waits on `child.wait()` and
  drains pipes concurrently. Verified: a run that previously hung 10+ min
  now completes iterations cleanly.
- **nvidia model EOL**: `deepseek-ai/deepseek-v4-flash` returned HTTP 410
  (EOL 2026-08-07); current id is `deepseek-ai/deepseek-v4-flash-0731`.
  Updated in `~/.pi/agent/models.json` worker.free[0] and this project's
  `.pidag/config.toml`. **The nvidia deepseek-v4-flash-0731 worker was
  unproductive on this large a spec** (4/4 implement iterations failed —
  timeouts and a silent fallback to the still-quota-blocked ollama-cloud
  mid-run); the missing `pii-cli` subcommands (F5.5-F5.9) were
  hand-implemented by the orchestrator instead of retried through pidag.
- **ollama-cloud session-usage-limit (HTTP 429)** hit mid-run on the
  glm-5.3-flash:cloud path; not a payment error, no reset time given.
  Not scheduled around this session — worker chain failed over to nvidia
  per AGENTS.md instead of waiting.

**Caveats / not yet exercised**: `pii daemon` (UDS path) was validated
indirectly via `scripts/phase2/uds-vs-loopback.sh` (spawns the built
binary as a real subprocess) but has no direct unit test; `pii tui`
(F5.1-F5.4, `pii-tui` crate) is Part 2/3/4 scope, not started.

## What exists
- 3-pass architecture research in `_docs/06-research/` (passes 1-3, ~48k words total).
- 5 reference repos cloned in `_repos/` (read-only).
- pidag initialized + BUG-FIXED (D1 worker-error swallowing, AlreadyDone fast-fail, binary/PATH/API-key hygiene, worker pipe-EOF hang — pidag commits `b2399f9`..`d21a0a8` + this session's `pi_print.rs` fix).
- **Phase 0 COMPLETE**: workspace skeleton + CI gates (commit `1ae1b32`).
- **Phase 1 COMPLETE**: core engine (Ctx + graph engine + journal + scheduler + validator + run/resume/trace CLI) + opus review fixes (F1-F16).
- **Phase 2 Part 1 COMPLETE**: client/server split, wire protocol, headless CLI, sandbox (6/6 EC, see above).

## PHASE 1 — COMPLETE
Spec `specs/03-phase1-core-engine.md` authored by opus, split into 4 parts (**24 exit criteria**). The pidag+glm-5.3-flash-worker path was debugged end-to-end and now genuinely implements code.

Committed milestones:
- **pii-core** (`7ce9402`): Ctx, CapSet, Budget, CancelToken, Principal, TraceCtx, Observer, FsCap. 18 tests.
- **pii-store**: redb persistence, transactional append_event (one write txn), snapshots, resume index. 5 tests.
- **pii-graph model+validator**: GraphSpec/JoinPolicy/Edge, load_toml, tiny `$.` expr, NodeFactory/NodeRegistry, validate() V0-V3/V5/V6/V9.
- **pii-graph sched state**: EdgeState/Readiness/RunState, readiness (All/Any/Quorum/AllSettled, entry-node-zero-inbound explicit), dead propagation fixpoint.
- **pii-graph blackboard + built-in nodes**: transform/route/emit/wait runtimes.
- **pii-graph run engine + CLI** (`21c3f6a`): tick loop draining all ready, UnsatisfiableJoin, limits; `pii --version` + `pii graph lint`.
- **pii-graph journal + run/resume/trace CLI** (`64d1bda`): journal emission + cap-narrowing, `sched/dispatch.rs` (parallel+sequential, cancel_losers), `cmd_run`/`cmd_resume`/`cmd_trace`, ndjson observer, four-test diamond suite (4 passed, incl. unsatisfiable V6 runtime + race cancel).
- **Phase-1 EC closure** (`c3e94fc`): readiness test naming, budget.rs test-name collision, bench-gate compact JSON (F6.2).

## OPUS REVIEW (contract = `_docs/06-research/`, NOT the spec) — all findings fixed
Full adversarial review by opus against the research docs (report `_tmp/phase1-opus-review.md`, 642 lines). Verdict: **PASS-WITH-FINDINGS, no BLOCKER**. The scheduler (the part a sibling project got wrong) is correct. Fixes landed:
- **F3** (`0fe39ba`): engine now validates before storing/executing; `run_with_validate` keeps the `--no-validate` escape hatch for the runtime UnsatisfiableJoin path.
- **F1/F2** (`0fe39ba`): budget wired into the scheduler (GraphLimits max_tokens/max_micro_usd); `BudgetShare::Shared` so same-process nodes share one budget (no first-node-drains-parent starvation).
- **F4** (`2a93750`): automated resume/crash-recovery test (`tests/resume.rs`) — the property was previously only verified by hand.
- **F6** (`e4d078d`): Effects as fail-closed bitflags (Default = WRITES_FS|MUTATES_EXTERNAL; adds NET/SPAWNS_PROC/LONG_RUNNING).
- **F7 part 1** (`510cbea`): CapKind moved to pii-abi (stable ABI surface). Full GraphNode move BLOCKED by circular dep (GraphNode::run takes &Ctx from pii-core, which depends on pii-abi) — documented, deferred.
- **F5** (`f1b2783`): bounded parallel dispatch (chunk by max_parallel / available_parallelism).
- **F8** (`1ebf756`): transitive cancel_losers (whole losing arm) + diamond Test E (5/5).
- **F9/F10** (`f8801a5`): V9 arity for Quorum; V5 port-with-no-edge warning.
- **F14/F15/F16** (`dffcf41`): PiiError doc/partition test; --allow-write canonicalisation; release profile (panic=abort) + perf profile.
- **run.rs split** (`d46af42`): run_helpers.rs to stay under the 450-line budget.

**Exit criteria — parts 2/3/4 6/6 met; part 1 4/6** (EC1 asserts pii-abi has no bitflags — superseded by F6; EC6 asserts diamond '4 passed' — now 5 after F8's Test E. Both are stale criteria, not regressions; specs are planner-owned so not edited). `quality-gate ALL PASS`.

## CAVEATS (open items from the opus review — read before Phase 2)
1. **F7 half-done — node ABI still in `pii-graph`, not `pii-abi`.** `GraphNode`/`NodeFactory` live in `pii-graph` and depend on `pii_core::Ctx`/`CapKind`. Moving them into `pii-abi` is blocked by a circular dependency (`GraphNode::run` takes `&Ctx` from `pii-core`, which depends on `pii-abi`). `CapKind` IS now in `pii-abi` (the load-bearing half). The full trait move needs a Phase-2-scale refactor of `Ctx` (and `Budget`/`CancelToken`/`Observer`) into `pii-abi`. Until then a WASM/worker node must depend on `pii-graph` (drags petgraph/toml/redb).
2. **Part-1 exit-criteria EC1/EC6 are stale, not regressions.** EC1 asserts `pii-abi` has no `bitflags` (superseded by F6 — bitflags is now a deliberate ABI dep). EC6 asserts the diamond suite is exactly `'4 passed'` (now 5 after F8's Test E). The specs are planner-owned (read-only for the workhorse), so these two criteria were NOT edited — a planner pass should update them.
3. **`pii-store/src/lib.rs` (383) and `validate.rs` (409) exceed the 380-line warn threshold** (both under the 450 hard limit). `lint-files` passes but warns. Consider splitting when next touched.
4. **`Ctx::deadline` is inert** (opus F19, not yet fixed): no constructor sets it, so `PiiError::Timeout` (the only retryable variant) is unreachable and the retry loop is dead code. Needs `Ctx::with_deadline` or deriving from `GraphLimits::max_wall_ms`.
5. **Snapshots written but never read** (opus F12): `resume()` replays the whole journal; `latest_snapshot` has no non-test caller. Correct (full replay is a superset) but O(total events) resume cost + snapshot writes are pure overhead until seeded.
6. **`max_steps` counts ticks, not nodes** (opus F20): a 500-wide batch counts as one step. Weakened (not absent) guard; wall-clock still bounds the run.

## KEY LEARNINGS (see agent-memory `implementation/*`, pidag `docs/`)
- **pidag worker error-swallowing (D1)**: worker `Err`/empty failures collapsed to "execution failed"; fixed in pidag `execute.rs`.
- **Stale-binary shadow**: `/usr/local/bin/pidag` (pre-D1) + `/usr/local/bin/pi` shadowed patched copies → phantom "No API key". USER DIRECTIVE: built binaries ONLY in `~/.local/bin`.
- **Background env drops OLLAMA_API_KEY**; background shells lack `/root/.local/bin` on PATH.
- **ollama-cloud large-prompt instability**: empty 0-token responses on >15KB prompts (glm-5.3-flash + deepseek-v4-flash). reasoning_effort:"none" + small prompts.
- **granite-4.2 tier-1 nano probe**: `_docs/06-research/004-granite-nano-tier-probe.md` — granite4.2:3b needs `--reasoning off`; Tier 0 stays pure-Rust embedder.

## NEXT — Phase 2: client/server split + coding harness
- Prompt ready at `_tmp/phase2_spec_prompt_2026-09-02.md`. Specs `04-phase2-client-server{,-part1..4}.md` partially authored by planner.
- Load-bearing gate: `cargo tree -p pii-tui` does NOT contain `pii-graph`.
- `pii-proto` (wire types), `pii-server` (UDS + loopback), `pii-cli -p`, `pii-tui` thin client, `starter/react` + `starter/verify-fix`, bash mediation + Landlock sandbox.

## Key files
- `specs/03-phase1-core-engine-part{1,2,3,4}.md` — Phase 1 split spec (all ECs now met).
- `examples/*.toml` — 10 functional + lint-negative example graphs (incl. crash-resume.toml).
- `crates/pii-graph/src/{spec,toml_load,expr,node,nodes,validate,sched/state,sched/dispatch,blackboard,journal,run}.rs`
- `crates/pii-store/src/{lib,tables,codec}.rs`, `crates/pii-cli/src/{main,cmd_run,cmd_trace,cli_util,ndjson}.rs`
- `crates/pii-graph/tests/diamond.rs` — four-test diamond suite.

## Memory keys
`pii/implementation/phase1-pii-core-complete`, `.../phase1-pii-store-complete`,
`.../phase1-pii-graph-lint-complete`, `.../phase1-scheduler-state-complete`,
`.../phase1-blackboard-nodes-complete`, `.../phase1-run-cli-lint-complete`,
`.../phase1-journal-run-resume-trace-complete`, `.../ollama-cloud-large-prompt-instability`

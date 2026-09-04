# Pii — Session Handoff

**Status: YELLOW (FROZEN) — PHASE 1 COMPLETE; PHASE 2 COMPLETE (22/22 EC); PHASE 3 PARTS 1-3 COMPLETE (28/28 EC); PHASE 3 PART 4 IN PROGRESS — 5/13 EC; USER DIRECTIVE 2026-09-04 22:54 UTC: "NO MORE CLAUDE CODE ON THIS! KEEP LESSONS LEARNED! NO MORE TOKENS TO TRY!" — part4 RESUME IS PAUSED INDEFINITELY pending user go-ahead. DO NOT spend Anthropic or ollama-cloud tokens on part4 without explicit user direction. Work is SAFE at checkpoint `db1de5f` + handoff `5ac882f`.**

## ⛔ STOP — USER ORDER: NO MORE TOKEN SPEND ON PART4 (2026-09-04 22:54 UTC)
User directive, verbatim intent: no more Claude Code on part4, keep the
lessons learned, no more tokens to try. All resume machinery has been
removed: the 03:15 UTC fresh-session cron (job c50f625a0644) was deleted.
Nothing is scheduled. No live claude/pidag/pi processes. The next agent
must NOT launch claude -p, pidag, or any paid/cloud work on part4 without
the user saying so. If part4 is later resumed, use FRESH `claude -p`
sessions only (see the token-bleed rule below) or whatever the user
directs at that time. The 8 remaining EC are documented below so a human
or a future free/approved run can pick up exactly where it stopped.

## LESSONS LEARNED (recorded for good — do not un-learn these)
1. **NEVER `claude --resume` for continuation** — it re-reads the whole
   accumulated conversation every turn (26-64M cache-read tokens per run)
   and exhausts the 5h Anthropic window in ~20 min of real work. Five
   resumed runs burned ~$35, ~90% on history replay. Fresh `claude -p`
   sessions start near 0% utilization and pay only for real work.
2. **Fresh session per run + compact brief + state in a file** is the
   correct delegation pattern for long implementation tasks.
3. Anthropic 5h session caps reset at fixed UTC times; ollama-cloud
   session caps give no reset time; ollama weekly quota resets Mon 07:00
   local (10:00 UTC); glm-5.3-flash:cloud may 404 in the live catalog.
4. Specs are planner-only; never let a workhorse edit them.
5. Checkpoint-commit clean-building work at every natural stop — the
   part4 work was checkpointed (`db1de5f`) and nothing was lost.

## PHASE 3 PART 4 — FROZEN STATE (5/13 EC; work SAFE)
All work from sonnet runs 1-6 is committed at checkpoint `db1de5f`
("chore(phase3 part4): checkpoint daemon, channels, node kinds, CLI,
fixtures mid-implementation (builds clean)") + handoff `5ac882f`. Tree
builds clean; `./quality-gate.sh` ALL PASS (fmt/clippy/test).

**EXIT CRITERIA 5/13 PASS** (validate: `/root/.pi/agent/skills/validate-exit-criteria/run.sh specs/05-phase3-assistant-gateway-part4.md /projects/pi-improved`):
- PASS: build, quality-gate, lint-files, shutdown.sh, graph-lint.
- FAIL (8, remaining when/if resumed): `scripts/phase3/{discord-e2e,cron,
  crash,secret-scan}.sh` (crash.sh can use graphs/starter/crash.toml;
  cron.sh soak_cron.toml), `cargo test -p pii-server --test {channels,
  approvals,scheduler}` (test files not yet written or not passing —
  check `crates/pii-server/tests/`), `cargo xtask rss-gate --secs 300
  --interval 10 --max-mb 50 --runs 3000` (finishes compiling then fails/
  hangs — fix the gate, do NOT weaken thresholds).

**DELIVERED (committed `db1de5f`)**: `crates/pii-channels/` (discord.rs
DiscordChannel + cli.rs CliChannel, tests pass), `pii-server/src/nodes/
{gate,memory,crash}.rs` in registry, `pii-server/src/daemon/` full module
(approvals/config/methods/mod/router/scheduler/status + attach_daemon_ctx),
RunSuspended/RunResumed bus mapping, FakeHttpServer stateful routes,
StatusResult extended, dispatch.rs delegating arms, CLI subcommands
(cmd_ctl/cmd_soak/cmd_sys + main.rs wiring + cmd_daemon), daemon SIGTERM
shutdown (shutdown.sh PASSES), fixtures (discord routes.json, model
assistant.json/atoms.json, soak.toml, workers.toml, ollama chat.json),
graphs/starter/{crash,slow,soak,soak_cron}.toml.

**REMOTE CHANNEL CHANGED: Telegram → Discord (user directive, 2026-09-04).**
Both `specs/05-phase3-assistant-gateway.md` and `-part4.md` are fully
Discord-edited (planner work, commits `562eb93`+`5e212ed`):
`PII_DISCORD_TOKEN`/`PII_DISCORD_CHANNEL_ID`/`PII_DISCORD_API_BASE`,
`DiscordChannel` in src/discord.rs, poll `GET /channels/{id}/messages?
after={cursor}`, send `POST /channels/{id}/messages` `{"content":…}`,
auth `Authorization: Bot <token>` header (never logged), script
`discord-e2e.sh`, fixtures under `fixtures/phase3/discord/`. **Never
revert to Telegram; specs are authoritative and Discord-only.**

**Next after part4 13/13 + committed** — user-requested, queued:
1. Opus deep analysis of the full implementation against
   `_docs/06-research/`, with **sonnet-5 actively fixing found faults**
   (not just reporting them — differs from the Phase 1/2 opus reviews).
2. Ask Opus for a **path to an MVP** as the first delivery.

## PHASE 3 PARTS 1-3 — COMPLETE (28/28 EC across 3 parts)
- **Part 1** (11/11 EC, commit `9b181b2`): `pii-net` (authorize/dial/
  listen/resolve/service/testing/clock/limits), `xtask net-check` gate,
  `pii-model::OllamaProvider`.
- **Part 2** (6/6 EC, commit `835efbb`): `pii-worker` crate (frame/spec/
  supervisor/control), `pii worker-exec`, 5-test supervisor integration
  suite (kill-9→WorkerDied, restart<2s, crash-loop cap, hot-reload drops
  nothing, stderr-flood no-deadlock), `scripts/phase3/{worker-kill,
  hot-reload}.sh`.
- **Part 3** (10/10 EC, commit `a87e637`): `pii-memory` crate (BM25 +
  i8-quantised vectors + graph co-occurrence lane, RRF k=60 fusion,
  temporal decay, consolidation with hard candidate cap, honest
  degradation), `xtask memory-eval` (anti-rigging fixture gate),
  `fixtures/phase3/memory/{corpus.jsonl,queries.jsonl}`.

Each part's pidag workhorse run hit its 50-tool-call iteration budget
1-2 times per part (normal, not a bug) — relaunching `launch-partN.sh`
resumes from committed state and made real incremental progress each
time; this is the expected pattern, not a stall.

## PHASE-3 SPEC — AUTHORED (master + 4 parts, committed `b555549`; part4 Discord-edited `562eb93`)
Opus authored `specs/05-phase3-assistant-gateway.md` + `-part1..4.md`
(1354 lines) against `_docs/06-research/`. Load-bearing requirement:
`cargo xtask net-check` (pii-net is the only socket/resolve call site).
Explicit scope cuts: gateway/TLS/ACP → `specs/06-phase3b-gateway-mcp.md`,
hickory-resolver cut, agent-memory-core cut (pii-memory first-party),
L2/L3 generative cuts, consolidation LLM cut, 72h soak → `rss-gate`,
reactive triggers → Phase 4. Protocol bumps to `pii.v2` (one required test
change: `hello_skew_is_hard_refusal` offers `pii.v99`). 31 runnable exit
criteria with script contracts. specs/ read-only for the workhorse.

## OLLAMA-CLOUD QUOTA — near weekly exhaustion (2026-09-04)
Repeated HTTP 429 "session usage limit" errors on `glm-5.3-flash:cloud`
during part4 (2 distinct quota walls same session, ~11:00 and ~11:59
UTC). User confirmed the weekly ollama quota is close to exhausted.
**Do NOT launch more pidag/ollama-cloud work this week without explicit
user go-ahead** — prefer Claude Code (Anthropic subscription, account
tier) for remaining Phase 3 part4 work and the Opus-review/MVP-path
follow-ups. Also note: `glm-5.3-flash:cloud` (the id in
`.pidag/config.toml`'s `[worker] default_model`) no longer resolves in
the ollama-cloud catalog as of this session — only `glm-5.2:cloud`,
`kimi-k2.7-code:cloud`, `deepseek-v4-flash:cloud` are listed. Probing
with `glm-5.3-flash:cloud` returns "Model ... not found", NOT a quota
error — use `glm-5.2:cloud` for any future ollama-cloud PONG probes.

## OPUS PHASE-2 REVIEW FIX SET — LANDED (all 5 BLOCKERs + major MAJORs)

The opus review (report `_tmp/phase2-opus-review.md`) found 5 BLOCKERs and
16 MAJORs. All 5 BLOCKERs and the major MAJORs are now fixed and committed:
- **F1** (BLOCKER): `proc_argv0=/bin/sh` granted in runs.rs + cmd_run.rs so
  the `bash` tool can execute; `starter/verify-fix`'s `test` node is live.
- **F2** (BLOCKER): `xtask deps-check` enforces all 4 of §12.2's invariants
  (pii-abi purity, pii-proto purity, pii-tui no pii-graph, single versions).
- **F3** (BLOCKER): `event.subscribe` filters live events by run_id + kinds.
- **F4** (BLOCKER): `Event::Lagged` added; bounded per-subscription queue.
- **F5** (BLOCKER): `selfchange.sh` copies Pii's own repo, drives react, and
  runs `cargo test --workspace` in the copy. Added `--input` to `pii -p`.
- **F6** (MAJOR): implemented run.start/run.get/run.list/graph.lint/
  event.unsubscribe.
- **F7** (MAJOR): `xtask proto-gen` + checked-in `pii-proto/schema/` +
  `proto-check` wired into `ci`.
- **F8** (MAJOR): `cargo xtask ci` now runs fmt/clippy/tests.
- **F9** (MAJOR): UDS peercred fails CLOSED + writes a 101 frame.
- **F10** (MAJOR): control dir 0700, socket 0600 after bind.
- **F11** (MAJOR): `pii tui` subcommand wired; `run_tui` prompts before
  subscribing.
- **F12** (MAJOR): agent node emits live ToolCall/ToolResult events.
- **F13** (MAJOR): `graph run`/`resume` use the server registry (lint/run agree).
- **F14** (MAJOR): `clippy.toml` has the full disallowed-macros + method bans.
- **F15** (MAJOR): `lint-files` excludes `#[cfg(test)]` bodies + tests/ dirs.
- **F16** (MAJOR): bash output cap now covers stdout/stderr fields.
- **F17** (MAJOR): sandboxed daemon grants write to store/runtime dirs.
- **F18** (MAJOR): `pii -p --sandbox` applies Landlock before the runtime.
- **F19** (MAJOR): `RunCancelled` uses the run's real journal seq.
- **F20** (MAJOR): `session.state` records the assistant reply.
- **F21** (MAJOR): real peercred (101 frame) + reattach (real bus) tests.
- **F28/F31/F32** (MINOR/NIT): overflow-checks, TTY-aware --json, real p50.

All 4 part ECs still pass (22/22). quality-gate: PASS.

## PHASE-2 CAVEATS / DEFERRED (from the opus review — read before Phase 3)
The opus review's "Contract points NOT implemented" table (report lines
957-983) lists requirements that are absent. The BLOCKER/MAJOR ones are now
fixed (F1-F21 above). The following remain genuinely deferred or cut, and
are NOT advertised as done:
1. **`pii acp` (Phase-2 EC3) is absent** — no `pii-acp` crate, no `acp`
   subcommand. The Zed-connect exit criterion was never implemented and is
   NOT flagged as deferred in the completion claim. Needs a planner decision:
   cut it, or carry it into Phase 3.
2. **Remote transport (`wss://host:7420/v1/ws`, TLS, `pii pair` bearer
   tokens)** — absent. Reasonably deferred (§14 puts networking in Phase 3)
   but not stated as deferred anywhere. Phase 3 owns this.
3. **`operator`/`viewer` roles** — absent. `dispatch.rs` discards
   `PeerIdentity`; no role exists in `HelloResult`. Phase 3 (gateway/auth).
4. **`run.await` sugar** over subscribe-until-terminal — absent from
   `Method`; `cmd_oneshot` hand-rolls the wait loop. Low value.
5. **`thinking` status line** (empty string clears) — absent. No
   `LlmThinking`/`thinking` variant anywhere.
6. **`NodeProgress`/`ToolProgress`** (bus-only progress) — absent; the
   bus-only stream has no producers (consistent with F12's live events).
7. **`ApprovalRequested`/`approve.resolve` suspend-resume approvals** —
   absent. Phase-3 by §14, but §8.8 is "the shape that pays for all of this".
8. **`RunRegistry`/`RunHandle` with `progress` + `done` watch channels;
   `run.attach`** — partial: `RunHandle` holds only a `CancelToken`; no
   `progress`, no `done`. `run.attach` unimplemented.
9. **Compaction gate (`compact_at_tokens`)** — declared in
   `graphs/starter/react.toml:12`, consumed nowhere. `AgentFactory::build`
   reads `system`/`tools`/`max_turns`/`model` only. `compact.json` fixture
   absent.
10. **`xtask size-gate` / `xtask rss-gate`** (the `contracts` job) — absent;
    only `bench-gate` exists. `cargo deny`/`deny.toml`, `cargo machete`,
    `typos`, `nextest` — absent.
11. **Postcard as a negotiated encoding over UDS** — cut to a refusal
    (`conn.rs:118-127`). Deliberate and documented in-code; acceptable.
12. **`Ctx::deadline` inert** (Phase-1 opus F19, still open): no constructor
    sets it, so `PiiError::Timeout` (the only retryable variant) is
    unreachable and the retry loop is dead code. Needs `Ctx::with_deadline`
    or deriving from `GraphLimits::max_wall_ms`.
13. **Snapshots written but never read** (Phase-1 opus F12): `resume()`
    replays the whole journal; `latest_snapshot` has no non-test caller.
    Correct (full replay is a superset) but O(total events) resume cost +
    snapshot writes are pure overhead until seeded.
14. **`max_steps` counts ticks, not nodes** (Phase-1 opus F20): a 500-wide
    batch counts as one step. Weakened (not absent) guard; wall-clock still
    bounds the run.
15. **Part-1 EC1/EC6 stale, not regressions** (Phase-1 caveat 2): EC1 asserts
    `pii-abi` has no `bitflags` (superseded by F6); EC6 asserts the diamond
    suite is exactly `'4 passed'` (now 5 after F8's Test E). Specs are
    planner-owned; a planner pass should update them.
16. **`pii-store/src/lib.rs` (383) and `validate.rs` (409) exceed the 380-line
    warn threshold** (both under the 450 hard limit). `lint-files` passes but
    warns. Consider splitting when next touched.

## OPUS PHASE-2 REVIEW (report `_tmp/phase2-opus-review.md`, 1118 lines)
Full adversarial review by opus against `_docs/06-research/` (NOT the specs).
Verdict: **FAIL — 5 BLOCKERs, 16 MAJORs, 8 MINORs, 4 NITs.** The architecture
*shape* is right (pii-proto pure, pii-tui tree clean, one transport path,
journal is the only redb writer, Ctx/ProcCap/Landlock careful) but the
enforcement layer and last mile fail. The 5 BLOCKERs:
- **F1**: `proc_argv0` is never granted anywhere → `ProcCap::run` denies
  unconditionally → the `bash` tool (advertised in every default session and
  both starter graphs) can never execute → `starter/verify-fix`'s `test` node
  is dead and Phase-2 exit criterion 1 is unreachable by construction.
- **F2**: `xtask deps-check` implements only 1 of §12.2's 4 invariants; the
  `pii-tui → pii-graph` gate does not exist.
- **F3**: `event.subscribe` ignores its own filter — every subscriber receives
  every run's events (cross-session leak on a multi-session daemon), and the
  single seq cursor drops events across runs.
- **F4**: backpressure inverted — silent drops (`dispatch.rs:214` discards the
  count), no `Lagged` variant on the wire, unbounded per-connection queue.
- **F5**: `selfchange.sh` uses `starter/react` not `verify-fix`, a toy two-file
  repo not Pii's own, a fixture that hard-codes the edits the assertions grep
  for, and never runs `cargo test`.

Notable MAJORs: F6 (10 of 22 wire methods unimplemented incl. `run.start`),
F7 (no `pii-proto/schema/` + `xtask proto-gen`), F8 (`xtask ci` runs neither
fmt/clippy/tests), F9 (UDS peercred fails OPEN when `SO_PEERCRED` unreadable +
no 101 frame), F10 (socket dir never 0700, 0600 applied after bind = TOCTOU),
F11 (`pii-tui` unreachable from the binary + `run_tui` always fails with
"session has no runs"), F12 (3 of 10 Event variants never emitted — no live
bus), F13 (`pii graph run` is a second execution path bypassing the protocol),
F14 (`clippy.toml` missing `disallowed-macros` + 6 of 11 method bans), F15
(`lint-files` counts `#[cfg(test)]` bodies), F16 (64 KiB bash cap bypassed by
uncapped stdout/stderr fields), F17 (workspace sandbox doesn't grant write to
store/runtime dir → `pii daemon --sandbox workspace` can't open its DB), F18
(Landlock applied per-thread; `pii -p --sandbox` applies it inside a running
multi-threaded runtime), F19 (`RunCancelled` broadcast with `seq: u64::MAX`
permanently poisons every subscriber's dedup cursor), F20 (`session.state` not
a full snapshot — assistant turns never recorded, tokens hardcoded 0), F21
(`peercred`/`reattach` tests are test theatre).

**Recommended order**: F1 (grant proc) unblocks the harness + F5; F2+F8 restore
the gates; F3/F4/F19 fix the event stream; F9/F10 close the local-auth hole;
F21 replaces the two certifying-nothing tests; F6/F7/F11/F12 are the remaining
contract surface.

## PRE-REVIEW BUG FIXES (commit `851a6f5`) — fixed before the opus review
Five bugs found while preparing the Phase-2 opus review, verified against
the spec and fixed first:
- **A (F2.5)**: `event.subscribe` required an explicit `run` and rejected
  the TUI's session-scoped subscribe (`run:None, session:Some`) with
  "needs run" — the TUI could never attach. Engine now tracks
  session→runs; a session-scoped subscribe resolves to that session's most
  recent run. Neither run nor session → `Invalid`.
- **B (F1.4)**: `sys.status` hardcoded `runs_active:0`/`sessions:0`. Now
  reports the live run-registry and session-map lengths.
- **C (F1.4/part-1 scope)**: hello encoding negotiation was not enforced —
  a client offering only non-json encodings was accepted. Server now
  refuses (code `-32602`) any hello whose `encodings` do not include
  `json`. New test `hello_non_json_encoding_refused`.
- **D (F2.10)**: agent node did not retry-once a retryable provider error.
  Now retries once, then routes to `err`. A non-retryable error still
  propagates as a node failure (run failure) — preserves the `fail.json`
  EC (exit 1).
- **E (F2.10)**: agent node always reported `files_changed: []`. Now
  records the resolved path of every successful `edit`/`multi_edit`/
  `fs_write` call.

## PHASE 2 — COMPLETE (all 4 parts, 22/22 exit criteria)
Spec `specs/04-phase2-client-server.md` split into 4 parts. All four are
done and committed:
- **Part 1** (6/6): `pii-proto` wire contract, `pii-server` daemon, `pii-model`/`pii-tools`, `pii-cli` `-p`/`daemon`/`sandbox-exec`. Commits `7800539`, `25cbf5b`.
- **Part 2** (6/6): starter graphs `react.toml`/`verify-fix.toml`, `tool` node kind, F4.5 loop re-entry, `peercred` test, daemon sandbox-before-model. Commit `1f441d4`.
- **Part 3** (5/5): `pii-tui` crate (pure protocol client), `handshake` tests, `selfchange.sh` E13 (real multi-file change). Commit `09809b8`.
- **Part 4** (5/5): `latency` test (N4 < 50 ms), scripted provider sleep. Commit `22535eb`.

**Key fixes landed across parts 2-4** (all root-caused, not papered over):
- **F4.5 loop re-entry**: engine now executes bounded cycles (was rejecting
  them at run time). `apply_completion` returns Result; re-entry resets the
  target + downstream reachable set; budget enforced at dispatch time
  (exactly `budget` executions). Split into `sched/loop_helpers.rs`.
- **fs tools relative-path bug**: `edit a.txt` resolved against process CWD,
  not the session workspace → "path parent not found". Each fs tool now
  carries a workspace and joins relative paths to it.
- **agent node caps bug**: TOML loader defaults node caps to
  `required_caps_of(kind)`, empty for `agent`/`tool` (builtin registry
  doesn't know them) → every edit denied `Ctx::fs()`. Starter graphs now
  declare explicit `caps = ["fs_read","fs_write","proc"]`.
- **Landlock PartiallyEnforced** (part 1): device nodes need file-only
  access rights, not the full directory-rights set.

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

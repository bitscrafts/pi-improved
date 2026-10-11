<!-- Filed 2026-10-11T15:21:50Z from an Opus plan-mode analysis run.
     Produced by: Claude Code + claude-opus-5-5, plan mode, READ-ONLY (no project writes).
     Brief: /projects/_tmp/pii-review-20261011/brief-does-the-tree-deliver-the-vision.md
     Inventory the report was measured against: /projects/_tmp/pii-review-20261011/codebase-inventory.txt
     Repo HEAD at analysis time: d2d9ec0 (tree: 0 changed paths)
     Run: session ef17e6d6-1741-42d6-87c6-16ffc775d231, stop_reason end_turn, turns 97, cost $39.76905

     STATUS: NOT a spec and NOT a plan of record. Nothing in this document is authorised work.
     Every proposal needs its own pre-registered gate before implementation, and any idea previously
     closed by a measurement stays closed until a NEW measurement reopens it.

     Anything marked "asserted-from-memory" is NOT verified — treat it as a lead, not a fact. -->

I have the evidence I need. HEAD is unchanged (`d2d9ec0`), the tree is clean, and no scratch daemon is running. Writing the report.

# pi-improved: does the tree deliver the vision? (Opus judgement, 2026-10-11)

**Provenance.** This is the reviewer of 043 and 045, in a resumed session, in test mode. HEAD is `d2d9ec0` and the working tree is clean before and after (0 changed paths, C-run). Nothing in the repo was edited and nothing was committed. All scratch work is under `/projects/_tmp/pii-review-20261011/`:

| path | what it holds |
|---|---|
| `run/` | gate and coverage logs, the coverage-table generator |
| `q7-btlF/` | an export of HEAD used for the race reproduction, the pillar demonstrations and the concurrency harness |
| `mcpprobe-*/`, `p1-*/` | the deployed-binary probes |

**Evidence labels:**

| label | meaning |
|---|---|
| **C-run** | I ran it this session and read the output |
| **C-read** | I read the cited lines myself |
| **I** | inferred |
| **asserted** | stated in the brief or a document and not re-verified by me |

## Contents

- 0. Verdict
- 0.1 Premise corrections
- Part 1: Dossier
  - 1.1 What I ran
  - 1.2 The system, explained
  - 1.3 The four pillars: decided, built, last measured
  - 1.4 Coverage summary (full table in Appendix A)
  - 1.5 NOT COVERED
- Part 2: Findings and proposals
  - 2.1 Ranked findings
  - 2.2 The ten questions
  - 2.3 Measurement plan with pre-registered thresholds
  - 2.4 Ordering traps
  - 2.5 What not to do, and the number that closed it
  - 2.6 Decisions that are only the user's
- Part 3: The value proof
  - 3.1 Per-pillar evidence
  - 3.2 The fastest demonstrable win
  - 3.3 What "delivered" looks like
  - 3.4 The most expensive thing that does not serve the vision
- 4. Ranked proposals, the single next action, its cost and its gate
- 5. NOT VERIFIED
- Appendix A: per-file coverage table (758 tracked files)

## 0. Verdict

1. **The tree delivers working substrate for all four pillars, but the machine delivers almost none of it.** The `pii` binary installed on this host is dated 2026-09-17 and `pii-mcp` is dated 2026-09-09. 86 commits to `crates/` since then (specs 61-132) are not deployed. The installed `pii -p` fails against the deployed config with rc 5, "no model configured" (C-run).
2. **One pillar has a real user.** Hermes calls the September `pii-mcp` as an MCP server: 184 pii tool calls since 2026-09-11, 161 of them in the last 30 days. That is 0.32% of Hermes's 50,978 tool messages in that window (C-run, read-only query of `~/.hermes/state.db`). Pillars 2, 3 and 4 have no user on this machine. No daemon runs, no unit is installed, and no channel, cron or trajectory is configured (C-run).
3. **Built from HEAD, every pillar works when I drive it by hand** (C-run):

   | pillar | demonstration | result |
   |---|---|---|
   | 1 | `pii -p` asked for a file edit through `fs_write` | file created in 11.5 s |
   | 2 | message dropped in the daemon's `cli` inbox | reply in the outbox about 4 s later |
   | 3 | `remember` then `recall` through the daemon | 2 entities and 1 relationship stored and read back |
   | 4 | the learning loop on scripted fixtures | the orchestrator's L1-L5 sequence reproduced exactly |

   Pillars 3 and 4 have no path from real use into them. Nothing writes entities from a conversation, trajectories are off by default, and every learning step needs a person to type it.
4. **The gate is green and mostly honest** (C-run): `fresh-gate` PASS in 5 min 45 s (1,107 passed, 0 failed, 10 ignored), and line coverage is 81.30% against a floor of 74.0. Three things the gate does not see are now measured:
   - the quarantine flake's mechanism, reproduced 3 of 3 (Q7);
   - a fixed-path redb test of the exact DO-NOT-REPEAT 1.48 class, failing 2 of 6 concurrent rounds;
   - a node panic that leaves its run reported as "in flight, poll again" forever, with no crash record.
5. **052's verdict ("a real engine, not yet a product") still holds and is now sharper.** The engine is broader (about 64k lines of source, up from about 35k at 043), but nothing since 2026-09-17 has reached the machine. The single next most valuable change needs no new code: deploy HEAD and wire one real Discord loop (section 4).

### 0.1 Premise corrections (verified contradictions to the brief)

| brief says | measured | label |
|---|---|---|
| "Only the OpenAI-compatible + Anthropic provider kinds exist" | Five model kinds exist: `llamacpp-server` (reusing the OpenAI-compatible provider), `openai`, `anthropic`, `gemini` (`crates/pii-model/src/gemini.rs`, 271 lines) and in-process `llamacpp` behind a feature. That is three wire protocols, not two. | C-read |
| `001` promises "30+ channels / ~20 providers" | `001-pii-vision.md:23` already says "measured: 5 kinds ... '30+' was the zeroclaw figure, not ours". The live contradiction is between 052's P4 addendum ("those claims are now COMMITMENTS") and `001`, not between `001` and the code. | C-read |
| tests create scratch under `_tmp` and "a lint step enforces it" | `lint-scratch` only checks that a `tempdir_in(` call aimed at `_tmp` creates its directory first (`xtask/src/lint_scratch.rs:1-8`). It does not forbid `std::env::temp_dir()`. The pii-web test at `crates/pii-web/src/main.rs:505` writes to `/tmp`, and 457 `/tmp/pii-web-mem-*` directories have accumulated. | C-run |
| Q7 "cannot be closed without a reproduced panic" | Reproduced deterministically (section 2.2, Q7). | C-run |
| open-items #3: the fixed-path class was fixed in passing | `crates/pii-memory/tests/lanes.rs:30` still returns a fixed path, `db()`, used by 3 tests. Two concurrent runs failed with "Database already open" in 2 of 6 rounds. My own 045 review also missed this site. | C-run |
| "Gates today: 14 fail-closed steps", with `fresh-gate` as the CI model | The export runs 13 steps. Coverage is SKIPPED by default ("set FRESH_GATE_COVERAGE=1"), and the log says so. The coverage floor is enforced only by the in-repo gate. | C-run |
| "Live on this machine right now": the local model server is llama.cpp | True for the model servers (:8082 answers in 2 ms, :8083 answers). pii itself is not live at HEAD on this machine (verdict 1). | C-run |

## Part 1: Dossier

### 1.1 What I ran

| what | command | result | label |
|---|---|---|---|
| CI-model gate | `cargo run -q -p xtask -- fresh-gate` | `quality-gate: ALL PASS · fresh-gate: PASS`, 13 steps plus coverage SKIPPED, 14:57:30 to 15:03:15 UTC; 142 `test result:` lines, 1,107 passed, 0 failed, 10 ignored | C-run |
| coverage | `cargo llvm-cov --workspace --lib --bins --tests --json --summary-only` | 81.30% (32,298 of 39,726 lines, 262 files); floor 74.0 | C-run |
| per-crate tests named in the brief | from the export log | `pii-server --lib` 117 (1 ignored) · `pii-store --lib` 49 · `pii-channels --lib` 29 plus 13 (adapters) + 12 (mqtt) + 2 (observe) integration · `pii-learn` tests 18 + 10 + 17 | C-run |
| deployed binaries | `ls -la ~/.local/bin/pii*` | `pii` 2026-09-17 01:30; `pii-mcp` 2026-09-09 05:29 | C-run |
| deployed `pii -p` | `PII_RUNTIME_DIR=<scratch> ~/.local/bin/pii -p "..."` | rc 5, `no model configured (set PII_MODEL or --model)` | C-run |
| what the live `pii-mcp` exposes | MCP `initialize` + `tools/list` over stdio | 7 tools: `bash edit fs_read fs_write glob grep multi_edit` | C-run |
| real use of pii | read-only SQLite query of Hermes `state.db` | `mcp__pii__bash` 171, `grep` 7, `glob` 4, `fs_read` 2 (2026-09-11 to 2026-10-10) | C-run |
| pillars 1-4 on HEAD | scratch daemon and one-shots, model `llamacpp-server:granite-4.2-3b` at `10.10.10.217:8082` | section 3.1 | C-run |
| Q7 race | 300 ms pause inserted between the commit and the event in a scratch copy | red 3 of 3 at `daemon/mod.rs:716:14`; control green 3 of 3 | C-run |
| fixed-path class | two concurrent runs of the `lanes` test binary, 6 rounds | 2 of 6 rounds fail `Database already open`; solo run 8 of 8 | C-run |
| learn loop | the orchestrator's `/projects/_tmp/e2e125/run.sh`, retargeted at my HEAD build | L1 refused, L2 applied, L3 reverted, L4 applied, L5 `auto_reverted` (`actor=daemon before=Some(0) after=Some(0)`) | C-run |

### 1.2 The system, explained

**Processes and binaries.** There are four binaries, not "a single binary" as `001:3` says (C-read).

```
                         ┌─────────────────────────── host (lnx) ───────────────────────────┐
  person / Hermes  ───►  │ pii  (CLI: -p one-shot, daemon, graph, sop, memory, learn, store, │
                         │      backup, metrics, health, tui, sandbox-exec, worker-exec)     │
  Hermes (MCP stdio) ──► │ pii-mcp  (ToolSet over MCP; the only binary in daily use today)   │
  browser ───────────►   │ pii-web  (read-only HTTP viewer; proxies the daemon UDS)          │
  cron/person ───────►   │ pii-evolve (boss-watch launcher; dials the daemon UDS)            │
                         │                                                                   │
                         │ pii daemon ── <runtime>/control.sock (JSON-RPC, closed vocabulary │
                         │               of 34 methods, pii-proto/src/method.rs)             │
                         └───────────────────────────────────────────────────────────────────┘
  models:  llama.cpp :8082 (granite/laguna)  · :8083 (laya)   ← llamacpp-server:<model>
           openai / anthropic / gemini wire kinds (base URL + key from env)
```

**Who writes what.** The daemon is the single writer of the redb stores. The JSON stores are written by both the daemon and the CLI (C-read, C-run).

```
<runtime_dir>/
  store.redb     runs, run_journal, run_snapshot, run_by_status, resume_index, graph_body, sessions
                 writer: the engine inside `pii daemon` (or inside a one-shot `pii -p` with its own dir)
  memory.redb    BM25 postings, vectors, doc memory, MEM_LAYER  (pii-store/src/mem)       writer: daemon
  approval.redb  human approvals (ApprovalStore)                                            writer: daemon
  heal.redb      heal ledger + spec 131 deny decisions (attempts ring, cap 20)              writer: daemon
  channel.redb   channel cursors                                                            writer: daemon
  timer.redb     timer wheel (scheduler)                                                    writer: daemon
  memory/        graph.json, access.json, counters.json  (entities/relationships/tiers, 126-127)
                 writers: daemon `remember` nodes AND CLI `pii memory entity ...`, serialised by a
                 create_new lockfile with stale-pid detection (pii-memory/src/entity/store.rs:303)
  learn/         candidates/, verdicts, promotions/<id>.json, ledger.jsonl (123-125)
                 writers: CLI `pii learn scan|verify|promote|revert` AND the daemon's 5 s sweep;
                 NO lock (pii-learn has none; grep finds no lock or create_new)
  trajectories/  <run_id>.jsonl, only if [core].trajectory_dir / $PII_TRAJECTORY_DIR is set (default OFF)
  audit.log, daemon log, crashes/
```

`pii store compact` and backup/restore refuse while a daemon holds the redb stores (`pii_store::compact::ensure_stopped`, C-read). Nothing equivalent protects `learn/`.

**The graph run path** (C-read, `engine/runs.rs`, `pii-graph/src/run`):

```
client ──run.start──► conn.rs (reader awaits each request; heavy methods on spawn_blocking, spec 80)
        ──► dispatch.rs ──► Engine::start_run ──► runs::spawn_run
               register_run · build Ctx (caps, Budget from [limits], trajectory observer, cancel)
               └─ spawn_blocking ──► start_or_refuse
                     ├─ quarantine_of(graph): reads learn/promotions/*.json   (start only; resume never asks)
                     │     └─ refused → record_quarantine_refusal (heal.redb deny) → refuse_start:
                     │          1) append_event(Failed) commits   2) bus.broadcast   3) log run.start.refused
                     └─ pii_graph::run::run → validate → scheduler ticks → node dispatch
                           → journal write-behind (pending buffer → store.append_events)
                           → terminal → bus → reactive loop (timer arm, approval request, cron re-arm,
                             channel reply; lag → rescan Suspended runs, spec 79/88)
```

**The four pillars as they exist today:**

| pillar | code that exists | entry points a person or graph can use | in use on this machine |
|---|---|---|---|
| 1. Coding agent core | `pii-core` (Ctx, fscap, childproc, budget), `pii-tools` (bash, fs, edit, grep, glob, worktree, memory tools, Landlock sandbox), `agent` node, sessions | `pii -p`, `pii tui`, `pii-mcp` | yes, through Hermes and the September `pii-mcp` |
| 2. Assistant/gateway | daemon, UDS, approvals, cron, 5 channel kinds (cli, discord with forum, telegram, matrix, webhook), `post` node, SOP compiler, MQTT, WASM host, HAL (simulated), model tiers, 5 model kinds | `pii daemon` with `[[channel]]`, `[[cron]]`, `[[mqtt]]`, `[hal]`, `[wasm]`; `pii sop` | no |
| 3. Memory + tiering | BM25/vector/graph-lane document memory; entities/relationships with tiers in JSON; `[[model_tier]]` | `remember`/`recall` nodes, `pii memory ...`, agent `memory = {enabled}` (document memory only) | no |
| 4. Self-improvement | trajectories (opt-in), `pii-learn` extract → verify (shadow run) → promote (`quarantine_graph`) → outcome window → auto-revert | `pii learn scan|verify|promote|revert|ledger` (manual), the daemon's sweep (automatic revert only) | no |

**The observability seam and its blind spots.** `pii-observe` defines events, measures, an audit trail, a file log, latency histograms and the trajectory sink. `lint-observability` makes the seam mandatory, with exemptions pinned at 2 entries (C-read). `pii health` and `pii metrics` render daemon state (C-run).

What the seam cannot see, all measured this session:
1. **A node panic.** The run's buffered journal events never reach the store and no crash file is written. `run get` answers "accepted and in flight; its journal has not materialised yet, poll again" indefinitely (C-run, F2).
2. **The order of refusal facts.** The store says `Failed` before `run.start.refused` exists, so an observer that polls status can miss the event (C-run, Q7).
3. **Kernel-level denials inside child processes** under Landlock. This was already stated in 052's correction (asserted).
4. **Races between the two writers of `learn/`.** No event exists for "two actors ended the same promotion" (C-read, F5).
5. **Real use.** The only real usage signal, Hermes's calls into `pii-mcp`, lives in Hermes's database and not in pii's.

### 1.3 The four pillars: decided, built, last measured

| pillar | what is decided | what is built | last measured number |
|---|---|---|---|
| 1. Coding agent | Local models only through llama.cpp (`llamacpp-server:`); `ollama:` refused by name (129). | One-shot, TUI, MCP server, sandboxed tools, sessions. | HEAD `pii -p` edit through `fs_write` 11.51 s, plain answer 0.54 s; deployed `pii -p` rc 5; 161 Hermes calls in 30 days (C-run). |
| 2. Gateway | Channels are written against the trait. User directive: "2 kinds openai and anthropic" for models. Discord forum is the surface; systemd, not Docker. | Daemon, 5 channel kinds, `post` node with idempotence, forum posting, SOP, MQTT, WASM, HAL (simulated). | HEAD inbox-to-outbox reply about 4 s through granite (C-run); one live forum thread by the orchestrator (asserted); deployed: none. |
| 3. Memory + tiers | Tiers are derived, never set; nothing is deleted by time; archive is the only removal path (127). | Entities/relationships with provenance, caps, tiers, archive/restore; model-tier table. | Through a HEAD daemon: 2 entities, 1 relationship, tiers hot 3, warm 0, cold 0 (C-run); deployed: none. |
| 4. Self-improvement | Exactly one action (`quarantine_graph`); propose → verify → promote → measure → revert. | All five stages, plus the 131 refusal ledger. | L1-L5 reproduced on HEAD with scripted fixtures (C-run); trajectories off by default; zero real-data runs. |

### 1.4 Coverage summary (full per-file table in Appendix A)

**State rule.** H means coverage of at least 70% from the llvm-cov run. S means below 70% (S<40 is called out). U-cov means the file is not in the llvm-cov report: declaration-only, feature-gated or build script. T means test code that ran green in `fresh-gate`. U-doc means a document whose accuracy was not re-checked file by file. R means a graph, example or script that some other tracked file references; D? means none does. CL or NC means a spec with or without a closure section. C means config or metadata.

| crate | lines covered | % |
|---|---|---|
| pii-abi | 473 / 481 | 98.3 |
| pii-observe | 1,523 / 1,604 | 95.0 |
| pii-http-fetch | 203 / 220 | 92.3 |
| pii-hal | 496 / 538 | 92.2 |
| pii-channels | 1,664 / 1,811 | 91.9 |
| pii-learn | 1,483 / 1,617 | 91.7 |
| pii-model | 1,007 / 1,113 | 90.5 |
| pii-proto | 479 / 529 | 90.5 |
| pii-wasm | 376 / 419 | 89.7 |
| pii-tools | 1,323 / 1,476 | 89.6 |
| pii-graph | 3,047 / 3,403 | 89.5 |
| pii-core | 1,321 / 1,487 | 88.8 |
| pii-store | 2,658 / 3,009 | 88.3 |
| pii-memory | 1,757 / 2,001 | 87.8 |
| pii-worker | 745 / 850 | 87.6 |
| pii-net | 1,287 / 1,516 | 84.9 |
| pii-server | 6,756 / 8,090 | 83.5 |
| pii-uds | 75 / 94 | 79.8 |
| pii-evolve | 164 / 206 | 79.6 |
| pii-mcp | 198 / 299 | 66.2 |
| pii-web | 259 / 400 | 64.8 |
| xtask | 2,562 / 4,276 | 59.9 |
| pii-cli | 2,205 / 3,837 | 57.5 |
| pii-tui | 237 / 450 | 52.7 |

**What matters in the low-coverage files** (C-run):
- `crates/pii-cli/src/cmd_learn.rs` has **0%** (0 of 256 executable lines). The CLI that drives all of pillar 4 is never executed by the gate. Its behaviour is proven only by the orchestrator's scratch harnesses, which live in `/projects/_tmp`.
- `cmd_sop.rs` 0%, `cmd_backup.rs` 0%, `node_kinds/nodes/worker.rs` 5%, `tools/src/sandbox.rs` 43%, `engine/registry.rs` 48%.
- `pii-model/src/llamacpp.rs` and `pii-hal/src/gpiod.rs` are compiled by clippy steps and never executed.

**Totals.** 758 tracked files: 321 H, 44 S, 139 U (110 docs, 27 `.rs` declaration, feature or build files, and 2 data files), 44 C, 60 R, 2 D?, 148 specs (28 CL, 120 NC). The NC specs are mostly 01-104, which predate the closure convention. 110 is the exception among the recent ones.

**Source size.** About 64.2k lines in `crates/*/src` plus about 19.9k in `crates/*/tests`; xtask is 6,596 lines (C-run, `git ls-files | xargs cat | wc -l`).

### 1.5 NOT COVERED

| item | reason |
|---|---|
| `_repos/` (5 reference clones, 5,720 `.rs` files in the inventory) | read-only reference, not product |
| `_tmp/`, `target/`, `graphify-out/`, `.pidag/`, `.pii/` | scratch, build output or derived artefacts, all gitignored |
| `crates/*` (a literal directory named `*`, untracked, holding `_tmp/`, created 2026-09-18 01:21) | not tracked; reported as F11 |
| repo-root `lint-approval.redb`, `lint-heal.redb`, `lint-memory.redb` | gitignored; created by `engine/registry.rs:174-204` opening lint stores in `runtime_dir` (F11) |
| content accuracy of every U-doc row | only the documents named in the findings were re-checked against code |
| the 2 PDFs and `.ipynb` | binary |
| `/projects/docs/...` (DO-NOT-REPEAT, RESUME-REGISTER) | outside the repo; rows 1.44-1.50 read only |

## Part 2: Findings and proposals

### 2.1 Ranked findings

| # | sev | finding | evidence | effort | risk if left |
|---|---|---|---|---|---|
| F1 | **HIGH (delivery)** | Nothing since 2026-09-17 is deployed, and the deployment is internally broken. The installed `pii` predates the `llamacpp-server:` kind that the installed config selects, so `pii -p` exits 5. `pii-mcp` dates from 2026-09-09. No unit, daemon, channel, cron or trajectory runs. 86 commits (specs 61-132) are not in use. | C-run: `ls -la ~/.local/bin/pii*`; the deployed one-shot; `git log --since=2026-09-17T01:30 --oneline -- crates \| wc -l` gives 86; `cat ~/.pii/pii.toml` | hours, no code | Every claim of "delivered" is untestable by a user; the only real user runs month-old code |
| F2 | **HIGH (reliability)** | A node that panics leaves its run "accepted and in flight; poll again" forever. The journal's buffered events (seq 0-2 are in the trajectory) never reach the store, no crash file is written, and the run is absent from `run ls --status running`. | C-run: three `pii run start starter/crash` against a HEAD daemon, then `pii run get r000000000004`; `crates/pii-server/src/node_kinds/nodes/crash.rs:55` | 1 spec (catch the unwind in the `spawn_blocking` closure in `engine/runs.rs`, write a `Failed` terminal like `refuse_start`, emit an event) | A client or channel polling a crashed run waits forever; pillar 4 never sees the failure |
| F3 | **MED (gate)** | The quarantine "flake" is a deterministic ordering race in the test's assumption (Q7). | C-run, reproduced 3 of 3 | about 10 lines | Red gates under load; time lost to re-runs |
| F4 | **MED (gate hygiene)** | `crates/pii-memory/tests/lanes.rs:30` `db()` is a fixed redb path used by 3 tests, the DO-NOT-REPEAT 1.48 class. `rrf_equals_bm25_when_lanes_disabled` also starts with `let _ = remove_file(db())`, which can delete a concurrent run's store. | C-run: 2 of 6 concurrent rounds fail `Database already open. Cannot acquire lock.`; solo 8 of 8 | minutes | Gate failures that read as flakes whenever a developer and a gate run overlap |
| F5 | **MED (audit integrity)** | `learn/` has two writers (CLI and daemon sweep) and no lock. `append_ledger` assigns `seq` by counting lines (`pii-learn/src/promote/storage.rs:75-93`). The sweep discards a save after appending to the ledger (`promote/mod.rs:362`). `revert` (`:307`) and `sweep` (`:328`) can both end one promotion. Resumed runs of a quarantined graph are not refused (`engine/runs.rs:128-150` is start-only). | C-read; race not executed | 1 spec (Q6 option B) | Duplicate ledger seqs, contradictory end states, a "reverted" ledger with a still-active file |
| F6 | **LOW-MED (hygiene)** | A pii-web test writes to `std::env::temp_dir()` (`crates/pii-web/src/main.rs:505`), and 457 `/tmp/pii-web-mem-<pid>` directories have leaked. `lint-scratch` cannot see it. 7 `temp_dir()` sites remain across crates. | C-run | small | Unbounded `/tmp` growth; a rule the brief believes is enforced is not |
| F7 | **LOW-MED (egress authority)** | Since 132, a channel's egress grant is whatever the channel declares (`daemon/config.rs:645`, `allow_declared`). Discord's declared host comes from `PII_DISCORD_API_BASE` (`pii-channels/src/discord.rs:67`, `:113-115`). An environment variable can therefore redirect the bot token to any host, and `[net] grants` no longer describes all egress. `channel_ctx` also still sets `Ctx` caps that the dial check does not read, which is two sources of truth. | C-read | small (validate declared hosts against an allowlist in `[net]`, or refuse a non-default base unless granted) | Silent widening of egress by configuration |
| F8 | **MED (pillar 4 honesty)** | The outcome metric is decided when the promotion is made: `improved = after < before` (`storage.rs:209-231`). Under quarantine, refused starts produce no failure trajectory, so `after` is about 0 and the verdict reduces to "did it fail in the window before promotion". The verifier compares failure class with `--input null` and empty caps (124's own limit). `cmd_learn.rs` has 0% coverage. Every step except revert is manual, and trajectories are off by default. | C-read; C-run for coverage and the L5 numbers | design | The loop can only confirm what an operator already knew |
| F9 | **MED (pillar 3 honesty)** | No path goes from conversation to entity memory. The `agent` node's `memory = {enabled}` uses the BM25 document memory. `remember`/`recall` appear only in `examples/memory-*.toml`, with literal names, and in `graphs/discord-digest.toml`, which is unwired. | C-read; C-run (daemon demo) | design | A library with tests, not a memory a user experiences |
| F10 | **LOW (docs truth)** | Stale or contradictory lines:<br>• `001:34-36` says "~14 crates" and "2,000-line gate"; there are 23 crates and a 450-effective-line gate.<br>• `001:82` has the wrong link label.<br>• `001:3` says "single Rust binary"; there are 4.<br>• `examples/pii.production.toml:108` says channel kind is "cli" or "discord"; 5 kinds exist.<br>• `~/.pii/pii.toml` comments still describe `ollama:` and say pii-net lacks TLS.<br>• 052's P4 addendum calls "30+/~20" commitments, contradicting `001:23`. | C-read | docs only | Readers trust the wrong numbers |
| F11 | **LOW (footguns)** | A literal directory named `*` sits in `crates/` (untracked). `registry.rs:174-204` drops `lint-*.redb` files into `runtime_dir`, which at the repo root means three stray files. Two scripts have no references (`scripts/run-batchA-implement.sh`, `scripts/run-rereview-resume.sh`). | C-run | minutes (move aside; human action under the no-delete rule) | A `*` directory is hazardous next to any unquoted glob, given this workspace's `rm` incidents |
| F12 | **INFO** | `gemini` is a third wire protocol outside the user's "openai and anthropic" directive. | C-read | small | Surface without a user |

### 2.2 The ten questions

**Q1. Does each pillar have a real, reachable user behaviour?**

| pillar | real user behaviour on this machine today | reachable at HEAD (command) | used by anything other than its tests |
|---|---|---|---|
| 1 | **Yes, partly.** Hermes calls `mcp__pii__bash` about 5 times a day through `pii-mcp` from 2026-09-09. The CLI path is broken (rc 5). | `pii -p "<task>"` with `PII_CONFIG` pointing at a config whose `model = "llamacpp-server:granite-4.2-3b"` and `[llama] host = "http://10.10.10.217:8082/v1"`: an `fs_write` edit lands in 11.5 s (C-run). Code: `crates/pii-cli/src/cmd_oneshot.rs`, `crates/pii-server/src/node_kinds/agent/turn.rs`. | Hermes (C-run) |
| 2 | **No.** | `pii daemon` with a `[[channel]] kind="cli"` bound to `starter/assistant`; append `{"chat":"c1","user":"u","text":"...","cursor":1}` to the inbox; the reply appears in the outbox in about 4 s (C-run). Code: `daemon/router.rs`, `pii-channels/src/cli.rs`. The Discord forum path was proven live once by the orchestrator (asserted). | none |
| 3 | **No.** | `pii run start memory-remember --input '{"note":"..."}'`, then `pii memory tiers --dir <rt>` (C-run). Code: `node_kinds/nodes/entity.rs`, `pii-memory/src/entity/*`. | examples and one unwired graph only |
| 4 | **No.** | The scripted harness (`/projects/_tmp/e2e125/run.sh` pattern) (C-run). Code: `pii-learn/src/*`, `daemon/mod.rs:49-57`. | none; default config produces no trajectories |

Pillars 3 and 4 are substrate with no user. Pillar 2 is substrate on this machine. Pillar 1 is the only one with a user, through a stale binary.

**Q2. Is the one action the pillar, or a token gesture?**

As shipped, it is a well-engineered gesture.

What is real (C-run): a candidate needs cited evidence, must survive a shadow run, must name a measurable metric (L1 refuses), is enforced by the engine with a named reason, and is reverted automatically.

What is not real:
- The only measured outcome is tautological under the only action (F8).
- The loop has never run on real data. 123's own limit says its thresholds are untuned on production data, and 124 found real graphs come back `inconclusive (graph_unavailable)`.
- It needs a person at three of its five stages.

**Smallest honest next step:** not a second action. Make the existing loop run unattended on real data, and measure whether it ever proposes anything worth doing:
1. Turn trajectories on in the deployment.
2. Add a daily `[[cron]]` graph, or a daemon timer, that runs extraction and verification. Promotion stays human-gated.
3. Replace the outcome metric with one quarantine cannot satisfy by construction, for example "the graph's definition changed, a new graph hash appeared, within the TTL" or "refused starts per day fell", which measures whether callers adapted.

**Decision rule (pre-registered).** Over 14 days of real deployment:
- if extraction yields **≥ 3 confirmed candidates** and the user judges **≥ 2 of 3 worth acting on**, design action #2;
- if it yields **0 confirmed candidates**, the pillar's input does not exist in practice, and building more actions is waste.

**Q3. Is the memory pillar used?**

No. Its callers are (C-read):
- `node_kinds/nodes/entity.rs` (`remember`/`recall`), used by `examples/memory-remember.toml` and `examples/memory-recall.toml`, which use the literal names `esp32-board`, `office` and `located_in`, and by `graphs/discord-digest.toml`, which is unwired;
- the CLI (`pii memory entity|link|neighbours|tiers|archive|restore`, `crates/pii-cli/src/cmd_entity.rs`);
- the tests.

The `agent` node, the only component that sees conversation, reads and writes the BM25 document memory (`memory = {enabled = true, top_k = 6}` in `graphs/starter/assistant.toml`), not entities. The data in the entity store on any machine is exactly what a hand-written graph or command put there. It is a library with tests and a working daemon path (C-run).

**Q4. Rewrite or defend the "30+ channels / ~20 providers" line?**

Rewrite, and most of it is already done. `001:23` already says "measured: 5 kinds" for channels and providers and attributes "30+/~20" to zeroclaw (C-read). What remains dishonest:
- (a) 052's P4 addendum calling those figures "COMMITMENTS on a plan" (C-read);
- (b) the gap between the user's directive ("2 openai and anthropic kinds; all other fits on this 2") and a code base with a third wire protocol (`gemini.rs`, 271 lines) plus the in-process loader.

**Cheapest honest change, docs only, under an hour.** Restate `001:23`'s provider sentence as: "two wire protocols, OpenAI-compatible and Anthropic, reach every vendor. `llamacpp-server:` is the OpenAI-compatible kind aimed at the local llama.cpp. `gemini` is a third protocol pending the user's keep-or-retire decision. In-process `llamacpp:` is feature-gated." Then mark 052's "COMMITMENTS" sentence as superseded by that directive. Do not build adapters.

**Q5. Which claims fail my own test?**

| claim | my test | verdict |
|---|---|---|
| 125: the loop closes end to end (L1-L5) | reran the orchestrator's harness against my HEAD build | **holds** (C-run) |
| open-items #2: "the quarantine flake's cause is NOT established" | widened the commit-to-event window | **superseded**: cause established (C-run) |
| open-items #3 / DO-NOT-REPEAT 1.48: the fixed-path class fixed in passing | concurrency harness on `lanes` | **does not hold**: one more site, 2 of 6 (C-run) |
| brief: "a lint step enforces" `_tmp` scratch | read `lint_scratch.rs`; counted `/tmp` | **does not hold** (C-run) |
| brief: "14 fail-closed steps", with `fresh-gate` as the CI model | ran `fresh-gate` | **partly**: 13 measured, coverage skipped in the export (C-run) |
| 129: "`llamacpp-server:` is the local kind" | ran the deployed binary with the deployed config | **true in code, false on this machine** (C-run) |
| 132: the channel grant is "fail-closed for an undeclared host" | read `allow_declared` and Discord's `egress_hosts` | **holds as stated**, but "declared" includes an env-var-chosen host (C-read, F7) |
| 131: the ledger accumulates refusals with `attempt_count == attempts.len() + dropped` | read `engine/refusal.rs`; not re-run live | **plausible**; not re-run (section 5) |
| 128/132 live Discord proofs | **not re-run**: they post into the user's forum and need the token, an outward action I was not authorised to take | section 5 |
| open-items: "re-verify any of this without me" | the commands point at `/projects/_tmp/verify13{1,2}-live.sh` and `conc-persist.sh` | **fragile**: the re-verification lives in scratch, which the repo's own rules allow to be cleared (C-read) |

**Q6. One audit trail or three?**

**Option B: the promotion becomes an RPC the daemon applies and records.** The discriminating fact is already measured. `learn/` has two writers and no lock (C-read), and its `seq` allocation reads and counts before appending (`storage.rs:75-93`). The entity store in the same project already solved the identical problem with a lockfile (`entity/store.rs:303`). So:
- option C leaves a live race;
- option A, mirroring, keeps both writers and adds a third copy;
- option B matches the pattern the daemon already uses for approvals and for 131's denials.

**Cost:** one spec. Add `learn.promote` and `learn.revert` to the closed method vocabulary. The CLI routes through the daemon when one is running, and otherwise refuses with the same `ensure_stopped` pattern `store compact` uses. The daemon writes the allow decision into the heal ledger next to 131's deny, and the JSONL becomes a derived export or is retired.

**Discriminating measurement, if anyone doubts the race:** 50 rounds of `pii learn revert <id>` launched at the instant a 1-second outcome window closes under a live daemon. Count rounds with two end-state ledger lines for one promotion, or with duplicate `seq`. **Any count > 0 rules out C.**

**Q7. The intermittent quarantine test.**

Mechanism established (C-run):
1. In `refuse_start` (`crates/pii-server/src/engine/runs.rs:184-216`), the `Failed` header commits first (`append_event`), then the bus broadcast, then `engine.log().event(RUN_START_REFUSED)`.
2. The test polls `get_run` until the status leaves `Running`, then immediately does `rec.events()...find(RUN_START_REFUSED).expect("run.start.refused")` (`daemon/mod.rs:712-716`).
3. A blocking thread descheduled between the commit and the log call loses that race. That explains why it fails only on loaded machines, and why raising 2 s to 60 s changed nothing: the panic is the `expect`, not the timeout.

Reproduction: with a 300 ms sleep inserted after the broadcast, the test fails 3 of 3 at `daemon/mod.rs:716:14` with `run.start.refused`. The unmodified control passes 3 of 3 (0.31-0.33 s).

**Fix.** Either wait on the event rather than the status (a condition poll on `rec.events()` with the same stated bound), or emit `RUN_START_REFUSED` before the commit. I recommend fixing the test, since an event after a durable write is the right product order.

**Pre-registered check.** With the 300 ms probe in place, the fixed test passes 10 of 10. The next natural gate failure, if any, must not show `mod.rs:716`.

**Q8. What to retire, ranked by risk removed over effort.**

| rank | item | risk removed | effort | whose decision |
|---|---|---|---|---|
| 1 | The second writer of `learn/` (`ledger.jsonl` and promotion files written by the CLI) via Q6 option B | audit contradictions (F5) | 1 spec | user picks B |
| 2 | Stray `crates/*` directory, repo-root `lint-*.redb`, 457 `/tmp/pii-web-mem-*` dirs | glob footgun; `/tmp` growth | minutes (move aside) | human, under the no-delete rule |
| 3 | The September binaries on the host (`~/.local/bin/pii*`), replaced, with the old ones moved aside | F1 | hours | user (host action) |
| 4 | `ollama.service` on the host | a dead dependency path | minutes | user (directive already given) |
| 5 | `gemini` provider (271 lines plus tests) | surface outside the directive | small | user |
| 6 | telegram, matrix and webhook channels (663 lines plus 13 + 4 tests): list them in `dormant-capabilities.md` rather than delete | none until used; avoids pretending | docs | user |
| 7 | `pii-hal` `gpiod` backend and its clippy gate step | one gate step for code never run on hardware | small | user |
| 8 | unreferenced scripts `run-batchA-implement.sh`, `run-rereview-resume.sh` | clutter | minutes | orchestrator |
| 9 | `deploy/Dockerfile` | already documented as a spare | none | leave |

**Q9. Does 052's verdict still hold?**

Yes. What changed since 052 (C-read, C-run):
- P0/P1 closed (musl static build, unit file, backup/restore, metrics, health).
- Every pillar has substrate.
- Coverage is measured (81.30%), and 1,107 tests pass in a fresh tree.

What did not change: nothing is deployed, and no person uses pillars 2-4.

The shortest path to "usable product" on this machine, using only what exists:
1. Release-build HEAD (`cargo build --release -p pii-cli -p pii-mcp`). Install it with the old binaries moved aside.
2. Write `/etc/pii/pii.toml` from `examples/pii.production.toml` with `llamacpp-server:granite-4.2-3b`, `trajectory_dir` on, one `[[channel]] kind="discord"` for the forum with the token in a 0600 env file, and one daily `[[cron]]` running `discord-digest`.
3. Enable `deploy/pii.service` on the lnx host. That is a user action, since a container cannot start host units.
4. Observe for 7 days with the measurements in 2.3.

**Q10. The single next most valuable change** is in section 4. What I would not do next is in 2.5.

### 2.3 Measurement plan (pre-registered thresholds)

| question | measurement | threshold, decided now | evidence path |
|---|---|---|---|
| Is HEAD deployed? | `pii health --json` → build identity; `ls -la --time-style=full-iso ~/.local/bin/pii*` | git rev equals the deployed commit; binary mtime after the deploy | daemon health |
| Is pillar 2 used? | `pii metrics` → `pii_channel_posts_total`; forum threads | **≥ 7 posts in 7 days**; 0 refusals not explained by config | metrics, Discord |
| Did Hermes regress after a `pii-mcp` swap? | `state.db` rate of `mcp__pii__*` calls and their error results | call rate within ±50% of the previous 161 per 30 days; error share not above the previous week's | Hermes `state.db` |
| Does pillar 4 have input? | `ls <traj>/trajectories \| wc -l`; `pii learn scan` counts | trajectories accumulate daily; candidate count recorded, **any value**, since this is the measurement | learn area |
| Is pillar 4 worth a second action? | confirmed candidates over 14 days, with the user's judgement | **≥ 3 confirmed and ≥ 2 of 3 accepted**, else stop building actions | 2.2 Q2 |
| Is the F2 fix real? | `debug_crash` run, then `pii run get` | terminal `failed` within 5 s, crash event logged; red before the fix | new test |
| Is the Q7 fix real? | the probe-widened test | 10 of 10 green with the 300 ms probe | scratch harness |
| Is the F4 fix real? | the concurrent `lanes` harness | 0 of 10 rounds collide | scratch harness |
| Is the Q6 race real? | 50-round revert-versus-sweep harness | any duplicate seq or double end means C is rejected | scratch harness |

### 2.4 Ordering traps

1. **Swapping `pii-mcp` under Hermes is not the same change as deploying the daemon.** HEAD's `pii-mcp` exposes a different tool set from the September binary's 7 tools (I). Deploy the daemon first; swap `pii-mcp` separately, behind a `tools/list` probe and a day of watching Hermes's call and error rate.
2. **Do not enable trajectories before F2 is understood.** A panicking node writes partial trajectories (C-run: the run 4 file holds seq 0-2 only), and the learn scan would read runs with no terminal. Either fix F2 first or treat terminal-less trajectories as excluded, and check how `scan` handles them before trusting counts.
3. **Option B before any second promotion action.** A second action multiplies the two-writer race.
4. **Fix F4 and Q7 before the next batch of specs.** Otherwise the next gate red will again be read as a flake.
5. **The Discord channel's base URL must stay at the default in deployment** until F7 is addressed, because the env var both selects the host and grants it.

### 2.5 What NOT to do next, and the number that closed it

| do not | the number |
|---|---|
| Build a second promotion action | 0 runs of the loop on real data; the outcome metric is decided by the window (F8) |
| Build more channel or provider kinds | 4 of 5 channel kinds and 4 of 5 model kinds have 0 users on this machine; the user's directive is two protocols |
| Reopen the code-knowledge-graph service | 043 §8 decision 6: the graph lacked both caller edges and would have made `reachability-check` weaker; no new measurement exists |
| Build a Docker path | user directive (systemd on lnx); `deploy/Dockerfile` is untested here |
| Raise test timeouts to fix flakes | Q7: the timeout was raised from 2 s to 60 s and the failure was never a timeout |
| Add more gate steps for substrate no one runs | the HAL `gpiod` clippy step guards 117 lines that have never run on hardware |

### 2.6 Decisions that are only the user's

1. Deploy HEAD on the lnx host as a systemd service, and allow it to post into the forum unattended (daily digest).
2. Swap the `pii-mcp` binary Hermes uses (separately from 1).
3. The audit trail: option B (recommended), A or C.
4. Keep or retire `gemini`, the telegram/matrix/webhook channels, and the HAL `gpiod` backend.
5. Disable `ollama.service` on the host (directive given; the action is the user's).
6. Forum as home channel or not, and the default cron tag (open-items #7).

## Part 3: The value proof

### 3.1 Per-pillar evidence

**Pillar 1: coding agent.**
- *Watch today, at HEAD* (C-run):
  ```
  PII_CONFIG=<cfg with model = "llamacpp-server:granite-4.2-3b", [llama] host = "http://10.10.10.217:8082/v1">
  pii -p "Use the fs_write tool to create the file note.txt ... pii was here"
  ```
  rc 0 in 11.51 s, with `files_changed: [".../note.txt"]` and the file contents exact.
- *Gap:* the installed binary returns rc 5 with the installed config (F1).
- *Used by others:* yes. Hermes made 184 calls, mostly `bash`.

**Pillar 2: assistant/gateway.**
- *Watch today, at HEAD* (C-run): `pii daemon` with one `cli` channel; one JSON line appended to the inbox produced `{"text":"2+2 equals 4.","to":"c1"}` in the outbox after about 4 s.
- *Gap:* no deployment; Discord is not configured; `discord-triage` and `discord-digest` are unwired (open-items #7c).
- *Used by others:* no.

**Pillar 3: memory and tiering.**
- *Watch today, at HEAD* (C-run): `pii run start memory-remember` and `memory-recall` through the daemon, then `pii memory tiers` returns `{"entities":2,"relationships":1,"tiers":{"hot":3,...}}`.
- *Gap:* nothing writes entities from real activity (F9).
- *Used by others:* no.

**Pillar 4: self-improvement.**
- *Watch today, at HEAD* (C-run): the e2e harness. Ledger L1 refused (no metric), L2 applied, L3 reverted, L4 applied, L5 `auto_reverted` by the daemon at `before=0 after=0`. A quarantined start was refused with `learn: graph denied-emit is quarantined by promotion p-ec9c08252011092f`.
- *Gap:* off by default; manual at three of five stages; the metric is tautological (F8); `cmd_learn.rs` has 0% gate coverage.
- *Used by others:* no.

### 3.2 The fastest demonstrable win

Candidates, ranked by visibility per day of effort:

1. **Deploy HEAD and run `discord-digest` daily into the user's forum.** No new code is needed; this is configuration plus a host action. Visible on the user's phone in a day. It exercises pillar 2 (post node, forum path, idempotence), pillar 3 (recall feeds the digest) and pillar 4's input (trajectories). **Pick this one.**
2. Fix the deployed one-shot by installing HEAD's `pii`. That is visible to the user, but only on demand.
3. Wire `discord-triage` to the forum. That is more surface, and it needs inbound polling of a forum, which the closures do not prove.

### 3.3 What "delivered" looks like, observably

A pillar is delivered when, over a 7-day window, a metric the daemon already exports moves because of real use. Each row names the artefact that already exists to show it.

| pillar | observable | existing artefact |
|---|---|---|
| 1 | Hermes's `mcp__pii__*` calls continue against the HEAD binary, and `pii -p` on the host returns rc 0 | Hermes `state.db`; `pii health` identity |
| 2 | `pii_channel_posts_total` ≥ 7 and a thread a day in the forum | `pii metrics` (`cmd_metrics.rs`), the forum |
| 3 | `pii memory tiers` shows entities that some graph wrote in the window, with tier changes | `pii memory tiers`, `pii_memory_*` counters |
| 4 | a non-zero trajectory count, and `pii learn scan` output on real runs (any number) | `pii learn list`, `pii_learn_*` metrics, health `learn` block |

### 3.4 The most expensive thing that does not serve the vision

It is building breadth that nothing deploys, while the deployment stands still:
- 86 commits to `crates/` since the installed binary;
- about 5,600 lines of surface no deployment uses (HAL plus WASM 2,593 with tests, SOP 1,497 with tests, telegram/matrix/webhook 663, MQTT 588, gemini 271);
- an extra gate step (`clippy hal-gpiod`) for code that has never run on hardware.

All of that against **0 deployed specs since 2026-09-17**. The number that says so is the binary mtime: `pii` at 2026-09-17 01:30:57 and `pii-mcp` at 2026-09-09 05:29:07, against HEAD `d2d9ec0` (C-run).

## 4. Ranked proposals, the single next action, its cost and its gate

| rank | proposal | finding | cost | pre-registered gate |
|---|---|---|---|---|
| 1 | **Deploy HEAD on the lnx host and run one real loop** (section 3.2) | F1 | about 2-3 hours of orchestration plus user host steps; no code | `pii health` identity equals the deployed rev; 24 h up under systemd; `pii_channel_posts_total` ≥ 7 over 7 days; Hermes call and error rate unchanged |
| 2 | Make a panicking node produce a terminal | F2 | 1 spec | `debug_crash` run, then `pii run get` returns `failed` within 5 s; red before the fix |
| 3 | Fix the Q7 test to wait on the event | F3 | about 10 lines | 10 of 10 green with the 300 ms probe |
| 4 | Make `lanes.rs:30` hermetic and sweep the class | F4 | minutes | 0 of 10 concurrent rounds collide; `grep -rn 'fn db() ' crates/*/tests` returns nothing |
| 5 | Option B: one writer for promotions | F5, Q6 | 1 spec | the 50-round race harness shows 0 doubles; `grep -c 'append(true)' crates/pii-learn/src` is 0 or reached only via the daemon |
| 6 | Run extraction and verification unattended; replace the tautological metric | F8, Q2 | 1 spec | after 14 days, the Q2 decision rule is applied |
| 7 | Bound channel egress by `[net]` | F7 | small | a non-default `PII_DISCORD_API_BASE` without a `[net]` grant is refused at startup |
| 8 | Docs truth pass | F10, Q4 | under 1 hour | `grep -n '~14 crates\|2,000-line' _docs/01-overview/001-pii-vision.md` returns nothing; 052 P4 annotated |
| 9 | `/tmp` hygiene and widen `lint-scratch` to `temp_dir()` | F6 | small | `grep -rn 'env::temp_dir()' crates --include=*.rs` shows 0 test sites; lint red on a probe |
| 10 | Retire per 2.2 Q8, rows 4-8 | Q8 | small each | user decisions |

**The single next action.** Deploy HEAD as the running pii on the lnx host:
1. Release build, then install with the old binaries moved aside.
2. Write the config: `llamacpp-server:granite-4.2-3b`, trajectories on, one Discord forum channel, one daily `discord-digest` cron.
3. Run it under `deploy/pii.service`.
4. Swap `pii-mcp` for Hermes only afterwards, as a separate step behind a `tools/list` probe.

**Why this one.** It is the only change that turns "the tree passes" into "a person sees pii work", and it makes every later measurement possible.

**Cost.** No code, a few hours, and two user host actions: enabling the unit and placing the token.

**Gate.** The rank-1 row above. Its failure modes are themselves the next findings.

## 5. NOT VERIFIED

| item | why |
|---|---|
| 128/132 live Discord posts, 131 live three-refusal harness | outward-facing (the user's forum) and token-bearing; not authorised in this run |
| Every orchestrator mutation in the 110-132 closures, except the 125 L1-L5 reproduction | time; reproduced only the end-to-end loop |
| `fresh-gate` on each spec's own commit | ran on HEAD only |
| HEAD `pii-mcp` tool list and Hermes compatibility | not built or probed |
| Behaviour of `pii learn scan` on terminal-less trajectories (F2 interaction) | not run |
| Q6 race in execution | read, not executed |
| HAL on hardware, WASM fuzzing | out of scope here, as the closures state |
| `ollama.service` on the host | unreachable from the container (measured `000` by the orchestrator, asserted) |
| Accuracy of every U-doc row in Appendix A | only the named documents were re-checked |
| The token-economy fact (laya `ask` used zero times) | orchestrator tooling outside this repo; asserted |
| Whether Hermes's 184 pii calls succeeded | counted calls, did not classify results |

## Appendix A: per-file coverage table (all 758 tracked files)

States: H ≥70% covered · S <70% covered · U-cov not in the llvm-cov report · T test code, ran green · U-doc document, accuracy not re-checked per file · R referenced by another tracked file · D? no reference found · CL/NC spec with or without a closure section · C config/meta. Sizes: `NL` lines; for `.rs` files `NL P% Nt` is lines, coverage and test count; `refN` is the number of other tracked files referencing it.

**.cargo/** (1 file)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| config.toml | Force clang for the vendored llama.cpp C++ build (spec 35) | 10L | C |  |

**.claude/** (12 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| CLAUDE.md | graphify | 3L | U-doc |  |
| settings.json | (binary/data) | 25L | C |  |
| skills/graphify/.graphify_version | (binary/data) | 0L | C |  |
| skills/graphify/SKILL.md | graphify skill definition | 713L | U-doc |  |
| skills/graphify/references/add-watch.md | graphify reference: add a URL and watch a folder | 56L | U-doc |  |
| skills/graphify/references/exports.md | graphify reference: extra exports and benchmark | 87L | U-doc |  |
| skills/graphify/references/extraction-spec.md | graphify reference: extraction subagent prompt | 70L | U-doc |  |
| skills/graphify/references/github-and-merge.md | graphify reference: GitHub clone and cross-repo merge | 46L | U-doc |  |
| skills/graphify/references/hooks.md | graphify reference: commit hook and native CLAUDE.md integ | 33L | U-doc |  |
| skills/graphify/references/query.md | graphify reference: query, path, explain | 311L | U-doc |  |
| skills/graphify/references/transcribe.md | graphify reference: transcribe video and audio | 52L | U-doc |  |
| skills/graphify/references/update.md | graphify reference: incremental update and cluster-only | 210L | U-doc |  |

**(root)** (11 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| .gitignore | ignore rules | 14L | C |  |
| AGENTS.md | Pii — Project Agents Guide | 95L | U-doc |  |
| CLAUDE.md | graphify rules | 9L | U-doc |  |
| Cargo.lock | lockfile | 4184L | C |  |
| Cargo.toml | workspace manifest | 116L | C |  |
| HANDOFF.md | Pii — Session Handoff | 941L | U-doc |  |
| clippy.toml | clippy config | 30L | C |  |
| coverage-floor.txt | Spec 108 R2: workspace line-coverage floor (74.0) | 5L | C |  |
| quality-gate.sh | Quality gate: fmt, clippy, test, coverage floor and the xtask checks | 198L | C | the export skips its coverage step by default |
| rust-toolchain.toml | toolchain pin | 3L | C |  |
| rustfmt.toml | rustfmt config | 2L | C |  |

**_docs/01-overview/** (2 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| 001-pii-vision.md | Pii — Vision & Architecture Overview | 83L | U-doc | stale: :34-36 '~14 crates', '2,000-line gate' (23 crates, 450 effective); :82 link label; :3 'single binary' (4 binaries) |
| README.md | 01-overview | 3L | U-doc |  |

**_docs/02-guides/** (18 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| 023-llamacpp-granite-on-pi-process.md | how-to (moved from 06-research) | 181L | U-doc |  |
| README.md | 02-guides | 14L | U-doc |  |
| discord-graphs.md | Discord graphs: the post node and the first two graphs | 128L | U-doc |  |
| discord-surface.md | The Discord surface (deployment note) | 53L | U-doc |  |
| feature-gating.md | Pii — Feature-Gating Subsystems | 92L | U-doc |  |
| hardware-hal.md | Hardware HAL: GPIO, I2C and SPI behind a claim and a scope | 108L | U-doc |  |
| learning-loop.md | The learning loop: extraction, verification, promotion | 276L | U-doc |  |
| memory-entities.md | Memory entities, relationships and tiers (126, 127) | 206L | U-doc |  |
| model-tiers.md | Model tiers (spec 114) | 57L | U-doc |  |
| mqtt.md | MQTT: the reactive front door (spec 117) | 93L | U-doc |  |
| observability-and-deploy-conventions.md | Observability and deploy conventions | 76L | U-doc |  |
| pi-local-llamacpp.md | Calling pi with local llama.cpp models | 73L | U-doc |  |
| pidag-way-of-work.md | Pidag — How We Implement (the SDD pipeline) | 123L | U-doc |  |
| pii-mcp-bridge.md | pii-mcp: MCP server + client | 83L | U-doc |  |
| quality-gate-and-fresh-tree.md | The quality gate: fourteen steps, two trees | 145L | U-doc | "fourteen": the export runs 13 plus coverage SKIPPED |
| wasm-plugins.md | WASM plugins (spec 122) | 154L | U-doc |  |
| writing-a-channel.md | Writing a channel | 121L | U-doc |  |
| writing-an-sop.md | Writing an SOP | 113L | U-doc |  |

**_docs/05-operations/** (5 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| 033-spec65-guardrail-violation-doc-format.md | incident record (moved) | 104L | U-doc |  |
| 105-runbook.md | Pii operator runbook | 398L | U-doc |  |
| README.md | 05-operations | 3L | U-doc |  |
| dormant-capabilities.md | Dormant capabilities: what is deliberately NOT wired | 50L | U-doc | candidate home for the telegram/matrix/webhook listing (Q8) |
| open-items.md | Open items: done and still missing | 99L | U-doc | #2's cause is now established (Q7); #3 incomplete (lanes.rs:30); re-verify scripts live in `/projects/_tmp`, which is not durable |

**_docs/06-research/** (51 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| 000-CANDIDATES-INDEX.md | Researched, not scheduled: candidate index | 35L | U-doc |  |
| 000-TOPIC-MAP.md | research corpus topic map | 109L | U-doc |  |
| 002-pii-architecture-plan-pass2.md | canonical architecture plan (merged corpus) | 4485L | U-doc |  |
| 005-dsh-vs-pii-harness-analysis.md | dsh vs Pii comparative analysis | 258L | U-doc |  |
| 006-pii-graph-engine.md | Graph engine architecture & design | 271L | U-doc |  |
| 007-pii-phase4-plan.md | Phase 4 "Daily Driver" plan | 164L | U-doc |  |
| 012-p47-remaining-opus-plan.md | merged canonical (P4.7 plan) | 534L | U-doc |  |
| 013-dsh-context-slice-c-reference.md | dsh-context reference design | 113L | U-doc |  |
| 014-local-model-spec-implementation.md | merged canonical (local models) | 656L | U-doc |  |
| 015-llm-in-process-llama-cpp.md | In-process LLM via llama-cpp-2 | 377L | U-doc |  |
| 016-ml-applications.md | Classical ML applications | 315L | U-doc |  |
| 018-manuals-self-contained.md | Manuals self-contained | 156L | U-doc |  |
| 019-generic-provider-endpoints.md | Generic OpenAI + Anthropic endpoints | 447L | U-doc |  |
| 020-git-graph-web-ui.md | git-graph web UI feasibility | 262L | U-doc |  |
| 021-mqtt-broker.md | MQTT: broker, bridge, or nothing | 460L | U-doc |  |
| 022-android-events-app.md | Android events app | 313L | U-doc |  |
| 024-pii-memory-mcp-status-and-next.md | memory + MCP status | 160L | U-doc |  |
| 025-model-routing-and-failover-plan.md | model routing + failover plan | 379L | U-doc |  |
| 026-codebase-analysis-and-phase5-plan.md | merged canonical (phase 5 plan) | 1054L | U-doc |  |
| 027-orchestration-resumption-spec38-verification.md | orchestration resumption | 109L | U-doc |  |
| 028-sdd-orchestration-review-checkpoint.md | SDD orchestration checkpoint | 183L | U-doc |  |
| 029-coding-harness-design-questions-for-opus.md | coding-harness design questions | 219L | U-doc |  |
| 030-OPUS-START-HERE.md | Opus review entry point | 500L | U-doc |  |
| 031-fable-review-ground-truth-2026-09-18.md | ground truth for fable review | 154L | U-doc |  |
| 032-fable-pii-review-2026-09-18.md | fable review findings | 573L | U-doc |  |
| 034-cactus-needle3-automation-research.md | Cactus Needle 3 research | 508L | U-doc |  |
| 037-pii-native-graph-service-design.md | merged canonical (graph service design) | 825L | U-doc | veto from 043 §8 decision 6 still holds |
| 038-pii-chromecast-device-manager-feasibility.md | Chromecast feasibility | 203L | U-doc |  |
| 039-jev-system-one-decision-layer-analysis.md | Jev System-1 analysis | 566L | U-doc |  |
| 040-laya-on-llamacpp-and-graph-driving.md | Laya on llama.cpp | 273L | U-doc |  |
| 041-APPENDIX-codebase-inventory.txt | older inventory snapshot | 18929L | U-doc |  |
| 041-BRIEF-full-review.md | 043's brief | 152L | U-doc |  |
| 042-PII-REVIEW-STAGED-STATE-2026-09-28.md | staged review state | 139L | U-doc |  |
| 043-opus-full-review-2026-10-06.md | Opus full review | 507L | U-doc |  |
| 044-REREVIEW-BRIEF-2026-10-09.md | 045's brief | 75L | U-doc |  |
| 045-opus-rereview-2026-10-09.md | Opus re-review | 295L | U-doc | missed `lanes.rs:30` `db()` (F4) |
| 046-mutation-verification-batch-cd-2026-10-09.md | spec 91 mutation log | 32L | U-doc |  |
| 047-close-green-mutants-log-2026-10-09.md | spec 92 mutation log | 12L | U-doc |  |
| 048-replay-err-divergence-measured-2026-10-09.md | replay divergence measured | 18L | U-doc |  |
| 049-replay-err-fix-remeasure-2026-10-09.md | replay fix re-measure | 11L | U-doc |  |
| 050-inline-methods-measured-2026-10-09.md | inline RPC handlers measured | 25L | U-doc |  |
| 051-scratch-helper-and-sweep-2026-10-09.md | spec 96 scratch report | 23L | U-doc |  |
| 052-production-readiness-2026-10-09.md | production-readiness assessment | 361L | U-doc | its P4 addendum calls '30+ channels / ~20 providers' commitments, contradicting 001:23 |
| 053-observability-contract-and-enforcement-2026-10-09.md | observability contract | 253L | U-doc |  |
| 054-coverage-and-adequacy-2026-10-10.md | coverage measured, ratchet | 74L | U-doc |  |
| 055-exemption-audit-2026-10-10.md | observability exemption audit | 83L | U-doc |  |
| 056-load-and-soak-2026-10-10.md | load and soak | 167L | U-doc |  |
| 057-vision-implementation-plan-2026-10-10.md | vision implementation plan | 101L | U-doc |  |
| 058-cost-findings-2026-10-10.md | cost findings | 194L | U-doc |  |
| README.md | 06-research | 5L | U-doc |  |
| assets/034-needle3-colab-notebook.ipynb | notebook | 0L | U-doc |  |

**_docs/99-archive/** (17 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| 06-research-merged-2026-09-28/001-pii-architecture-plan.md | Architecture Plan (archived) | 1356L | U-doc |  |
| 06-research-merged-2026-09-28/003-pii-architecture-plan-pass3.md | Pass 3 (archived) | 786L | U-doc |  |
| 06-research-merged-2026-09-28/004-granite-nano-tier-probe.md | Granite nano probe (archived) | 56L | U-doc |  |
| 06-research-merged-2026-09-28/008-pii-evolve.md | P4.7 boss design (archived) | 215L | U-doc |  |
| 06-research-merged-2026-09-28/009-pii-evolve-opus-review-pass1.md | boss review (archived) | 102L | U-doc |  |
| 06-research-merged-2026-09-28/010-pii-full-review-pass1.md | full review pass 1 (archived) | 518L | U-doc |  |
| 06-research-merged-2026-09-28/011-pii-architecture-conformance-review-pass1.md | conformance review (archived) | 217L | U-doc |  |
| 06-research-merged-2026-09-28/017-tiny-models-real-usage.md | tiny models (archived) | 232L | U-doc |  |
| 06-research-merged-2026-09-28/035-graphify-code-knowledge-graph-research.md | graphify research (archived) | 267L | U-doc |  |
| 06-research-merged-2026-09-28/036-graphify-integration-design-discussion.md | graphify design (archived) | 322L | U-doc |  |
| 06-research-merged-2026-09-28/README.md | merged documents index | 17L | U-doc |  |
| 06-research-merged-2026-09-28/merge-ledger.json | merge ledger | 57L | U-doc |  |
| README.md | 99-archive | 3L | U-doc |  |
| handoff-2026-09.md | archived session handoff | 2002L | U-doc |  |
| workhorse-spec-violations-2026-09/05-phase1-run-engine-part1-journal-caps-resume-groundwork.md | violation record | 155L | U-doc |  |
| workhorse-spec-violations-2026-09/06-phase1-run-engine-part2-concurrent-dispatch.md | violation record | 150L | U-doc |  |
| workhorse-spec-violations-2026-09/07-phase1-run-engine-part3-crash-resume.md | violation record | 150L | U-doc |  |

**_docs/** (2 files) and **_docs/manuals/** (4 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| INDEX.md | docs index | 85L | U-doc |  |
| TOC.md | docs table of contents | 41L | U-doc |  |
| manuals/technical-manual.pdf | built manual | 0L | U-doc |  |
| manuals/technical-manual.tex | technical manual source | 820L | U-doc |  |
| manuals/user-manual.pdf | built manual | 0L | U-doc |  |
| manuals/user-manual.tex | user manual source | 735L | U-doc |  |

**crates/pii-abi/** (10 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 23L | C |  |
| src/cap.rs | Coarse capability bits | 59L 100% 0t | H |  |
| src/effects.rs | Effect classification for parallel scheduling | 83L 100% 2t | H |  |
| src/error.rs | Kernel error type | 267L 100% 9t | H |  |
| src/failover.rs | Typed failover reasons and classifier | 432L 99% 27t | H |  |
| src/http.rs | HTTP data types, no sockets | 117L 96% 3t | H |  |
| src/ids.rs | Id newtypes | 76L 85% 1t | H |  |
| src/journal.rs | Event-sourced journal types | 161L n/a 0t | U-cov |  |
| src/lib.rs | Kernel ABI root | 26L n/a 0t | U-cov |  |
| src/node.rs | Node input/output contract | 169L 100% 4t | H |  |

**crates/pii-channels/** (17 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 32L | C |  |
| src/cli.rs | CliChannel: NDJSON on two files | 138L 95% 1t | H |  |
| src/discord.rs | DiscordChannel over HTTP(S) | 403L 88% 9t | H | F7: egress host comes from `PII_DISCORD_API_BASE` (:67, :113-115) and is granted without `[net] grants` |
| src/discord/forum.rs | Posting into a Discord forum (132) | 382L 97% 8t | H |  |
| src/lib.rs | channel trait: inbound poll + outbound reply | 168L 96% 1t | H |  |
| src/matrix.rs | MatrixChannel (112) | 225L 83% 0t | H | no deployment uses it (Q8) |
| src/mqtt/config.rs | MQTT settings and safety rules | 178L 82% 0t | H |  |
| src/mqtt/mod.rs | MqttChannel (117) | 410L 94% 0t | H | no deployment uses it |
| src/observe.rs | channels' observability seam | 102L 100% 0t | H |  |
| src/post.rs | egress seam of the post node (128) | 432L 93% 8t | H | idempotence key kept on timeout (mutation-proven by the orchestrator, asserted) |
| src/secret.rs | channel credential source | 82L 92% 2t | H |  |
| src/telegram.rs | TelegramChannel (112) | 200L 93% 0t | H | no deployment uses it (Q8) |
| src/webhook.rs | WebhookChannel (112) | 238L 93% 0t | H | no deployment uses it (Q8) |
| tests/adapters.rs | telegram/matrix/webhook adapters, offline | 395L n/a 13t | T |  |
| tests/mqtt.rs | MQTT channel vs loopback broker | 260L n/a 12t | T |  |
| tests/mqtt_broker/mod.rs | minimal in-process MQTT broker | 273L n/a 0t | T |  |
| tests/observe_contract.rs | poll/send failures reach the observer | 82L n/a 2t | T |  |

**crates/pii-cli/** (34 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 40L | C |  |
| src/cli_util.rs | helpers for `pii graph` subcommands | 186L 93% 1t | H |  |
| src/cmd_backup.rs | `pii backup` / `pii restore` | 88L 0% 0t | S |  |
| src/cmd_ctl.rs | thin protocol CLI: net, worker, memory, approve | 239L 37% 0t | S |  |
| src/cmd_daemon.rs | `pii daemon`: socket, daemon start | 767L 53% 9t | S |  |
| src/cmd_entity.rs | `pii memory entity/link/neighbours/compact` | 344L 88% 0t | H |  |
| src/cmd_health.rs | `pii health` | 135L 77% 1t | H |  |
| src/cmd_learn.rs | `pii learn scan/list/show/verify/promote/revert/ledger` | 394L 0% 0t | S | 0% coverage: the pillar-4 CLI is never executed by the gate (F8) |
| src/cmd_metrics.rs | `pii metrics` | 636L 97% 10t | H |  |
| src/cmd_oneshot.rs | `pii -p` headless entry point | 430L 76% 5t | H | pillar 1 demonstrated on HEAD (C-run) |
| src/cmd_run.rs | `pii graph run` / `resume` | 420L 66% 5t | S |  |
| src/cmd_sandbox_exec.rs | `pii sandbox-exec` | 126L 75% 2t | H |  |
| src/cmd_soak.rs | `pii soak` | 208L 10% 1t | S |  |
| src/cmd_sop.rs | `pii sop compile/lint/show/run/resume` | 160L 0% 0t | S |  |
| src/cmd_store.rs | `pii store compact` | 49L 83% 0t | H |  |
| src/cmd_sys.rs | `pii sys`, `pii run ls/start/get` | 199L 42% 2t | S |  |
| src/cmd_trace.rs | `pii graph trace` | 336L 27% 4t | S |  |
| src/cmd_tui.rs | `pii tui` | 147L 64% 0t | S |  |
| src/cmd_worker_exec.rs | `pii worker-exec` reference worker | 136L 0% 0t | S |  |
| src/daemon_log.rs | daemon log and audit plumbing | 278L 88% 4t | H |  |
| src/main.rs | `pii` entry point | 266L 53% 2t | S |  |
| src/ndjson.rs | NDJSON stdout observer | 145L 79% 8t | H |  |
| tests/canary_secrets.rs | secrets never reach run output | 91L n/a 3t | T |  |
| tests/daemon_default_sandbox.rs | daemon confined by default | 74L n/a 4t | T |  |
| tests/daemon_log_wiring.rs | daemon log attached to subsystems | 57L n/a 1t | T |  |
| tests/failure_output.rs | operator failures go through cli.failure | 19L n/a 1t | T |  |
| tests/memory_entity.rs | `pii memory entity` against the real binary | 120L n/a 3t | T |  |
| tests/memory_tiering.rs | `pii memory tiers/archive/restore` | 166L n/a 2t | T |  |
| tests/oneshot_default_sandbox.rs | `pii -p` confined by default | 79L n/a 3t | T |  |
| tests/sandbox_grant.rs | workspace profile grant | 108L n/a 4t | T |  |
| tests/store_compact.rs | compact refuses while a daemon holds | 74L n/a 2t | T |  |
| tests/transport_failures_visible.rs | a silent daemon does not hang the CLI | 62L n/a 2t | T |  |
| tests/tui_default_sandbox.rs | `pii tui` confined by default | 49L n/a 2t | T |  |
| tests/tui_failure_logged.rs | TUI attach failure reported | 47L n/a 1t | T |  |

**crates/pii-core/** (13 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 28L | C |  |
| src/budget.rs | carveable accounting | 212L 93% 2t | H |  |
| src/cancel.rs | cancellation token | 85L 93% 1t | H |  |
| src/cap.rs | capability scopes and set | 301L 91% 5t | H |  |
| src/childproc.rs | child-process machinery | 445L 92% 3t | H |  |
| src/clock.rs | injectable clock | 74L 100% 2t | H |  |
| src/config.rs | environment configuration access | 40L 100% 2t | H |  |
| src/ctx.rs | the enforcing capability context | 585L 85% 5t | H |  |
| src/fscap.rs | filesystem capability gate | 222L 87% 2t | H |  |
| src/lib.rs | crate root | 47L n/a 0t | U-cov |  |
| src/net.rs | HTTP transport seam | 82L 100% 1t | H |  |
| src/observe.rs | events, metrics, observer multiplexing | 669L 86% 10t | H |  |
| src/principal.rs | identity and tracing context | 41L 86% 0t | H |  |

**crates/pii-evolve/** (8 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 29L | C |  |
| src/driver.rs | boss outer loop as a UDS launcher | 174L 95% 0t | H |  |
| src/lib.rs | crate root | 23L n/a 0t | U-cov |  |
| src/main.rs | `pii-evolve` binary | 54L 0% 0t | S | not running on this host |
| src/state.rs | `state.md` management | 141L 88% 0t | H |  |
| tests/launcher.rs | thin UDS launcher | 279L n/a 5t | T |  |
| tests/observe_contract.rs | launch failures reach the observer | 60L n/a 2t | T |  |
| tests/state_tests.rs | `state.md` round trip | 59L n/a 2t | T |  |

**crates/pii-graph/** (36 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 31L | C |  |
| src/blackboard.rs | run blackboard | 75L 97% 2t | H |  |
| src/cron.rs | 5-field UTC cron parser | 223L 88% 4t | H |  |
| src/expr.rs | tiny expression language | 88L 98% 4t | H |  |
| src/journal.rs | journal appending for a run | 292L 94% 0t | H | buffered events are lost when a node panics (F2) |
| src/lib.rs | crate root | 32L n/a 0t | U-cov |  |
| src/node.rs | harness ABI and node registry | 355L 89% 8t | H |  |
| src/nodes.rs | built-in node kinds | 326L 83% 0t | H |  |
| src/render.rs | ASCII graph rendering | 516L 99% 7t | H |  |
| src/run/engine.rs | scheduler tick engine | 377L 86% 0t | H |  |
| src/run/mod.rs | store-backed run entry points | 119L 98% 0t | H |  |
| src/run/resume.rs | journal-replay resume | 133L 95% 0t | H |  |
| src/run_helpers.rs | run-engine helpers | 243L 74% 0t | H |  |
| src/sched/dispatch.rs | node dispatch, retry, cancellation | 460L 85% 0t | H |  |
| src/sched/loop_helpers.rs | loop re-entry helpers | 60L 98% 0t | H |  |
| src/sched/mod.rs | scheduler module root | 14L n/a 0t | U-cov |  |
| src/sched/state.rs | scheduler run-state machine | 532L 93% 5t | H |  |
| src/sop/compile.rs | SOP → graph compiler | 201L 99% 0t | H |  |
| src/sop/lint.rs | SOP lint | 182L 97% 0t | H |  |
| src/sop/mod.rs | SOP engine (113) | 138L 97% 0t | H | no deployment uses it |
| src/spec.rs | graph specification model | 300L 99% 2t | H |  |
| src/toml_load.rs | TOML graph loader | 385L 75% 3t | H |  |
| src/validate.rs | static graph validation | 482L 91% 3t | H |  |
| tests/brake.rs | `max_tool_calls` on limits | 37L n/a 1t | T |  |
| tests/diamond.rs | diamond suite | 358L n/a 5t | T |  |
| tests/examples.rs | checked-in example graphs | 98L n/a 4t | T |  |
| tests/journal.rs | journal write-behind | 269L n/a 7t | T |  |
| tests/journal_is_the_metric_surface.rs | graph metrics from the journal | 135L n/a 2t | T |  |
| tests/loops.rs | loop re-entry | 186L n/a 2t | T |  |
| tests/observe_contract.rs | graph metrics reach the observer | 68L n/a 1t | T |  |
| tests/replay_err_divergence.rs | replay error measurement | 154L n/a 2t | T |  |
| tests/resume.rs | resume / crash recovery | 357L n/a 3t | T |  |
| tests/rss_probe.rs | store-only RSS probe | 79L n/a 1t | T |  |
| tests/sop.rs | SOP format, lint, compiler | 212L n/a 9t | T |  |
| tests/trajectory.rs | a real run through the trajectory bridge | 112L n/a 2t | T |  |
| tests/write_amp.rs | where a run's bytes go | 102L n/a 1t | T |  |

**crates/pii-hal/** (10 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 28L | C |  |
| src/backend.rs | backend trait | 52L n/a 0t | U-cov |  |
| src/gpiod.rs | real GPIO backend (Linux character device) | 117L n/a 0t | U-cov | compiled by the gate (clippy hal-gpiod), never run on hardware |
| src/hal.rs | claim and scope rules | 443L 91% 0t | H |  |
| src/hub.rs | late-bound slot from `[hal]` | 40L 100% 0t | H |  |
| src/lib.rs | crate root | 42L n/a 0t | U-cov |  |
| src/scope.rs | pin/bus capability scope | 122L 97% 0t | H |  |
| src/sim.rs | simulated backend | 217L 93% 0t | H |  |
| src/types.rs | HAL value types | 73L 82% 0t | H |  |
| tests/simulated.rs | HAL contract on the simulated backend | 298L n/a 18t | T |  |

**crates/pii-http-fetch/** (8 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 32L | C |  |
| src/audit.rs | append-only audit log | 105L 96% 1t | H |  |
| src/fetch.rs | fetch argument parsing | 144L 99% 4t | H |  |
| src/lib.rs | GET-only fetch through pii-net | 122L 71% 0t | H | dormant by decision (its own test says so) |
| src/observe.rs | one outcome event per fetch | 76L 100% 1t | H |  |
| tests/dormant.rs | no crate depends on it | 35L n/a 1t | T |  |
| tests/fetch.rs | GET/https/POST blocked/headers blocked | 223L n/a 6t | T |  |
| tests/observe_contract.rs | outcomes reach the observer | 80L n/a 2t | T |  |

**crates/pii-learn/** (16 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 25L | C |  |
| src/candidate.rs | candidate lesson record | 158L 91% 0t | H |  |
| src/config.rs | thresholds and read bounds | 46L 100% 0t | H | untuned on production data (123's own limit) |
| src/extract.rs | four deterministic extractors | 240L 100% 0t | H |  |
| src/learner.rs | one extraction scan and its health seam | 188L 84% 0t | H |  |
| src/lib.rs | crate root | 39L n/a 0t | U-cov |  |
| src/promote/mod.rs | promotion and lifecycle | 404L 91% 0t | H | F5: `let _ = save(...)` at :362 after the ledger append; `revert` (:307) and `sweep` (:328) can both end one promotion |
| src/promote/storage.rs | promotion storage, ledger, measure | 232L 95% 0t | H | F5: `append_ledger` (:75-93) assigns seq by counting lines, no lock, two writers; F8: `measure` (:209-231) is decided by the window choice |
| src/promote/types.rs | promotion vocabulary | 374L 89% 0t | H |  |
| src/scan.rs | bounded, symlink-refusing reader | 332L 92% 0t | H |  |
| src/store.rs | where candidates live | 154L 88% 0t | H |  |
| src/verdict.rs | verifier verdicts | 161L 84% 0t | H |  |
| src/verify.rs | verifier (shadow runs) | 456L 91% 0t | H | a shadow run uses `--input null` and empty caps: it compares failure class, not cause |
| tests/learn.rs | extraction, refusal, injection control | 555L n/a 18t | T |  |
| tests/promote.rs | promotion and lifecycle | 424L n/a 17t | T |  |
| tests/verify.rs | verifier over real-engine fixtures | 385L n/a 10t | T |  |

**crates/pii-mcp/** (3 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 23L | C |  |
| src/main.rs | MCP server + client | 425L 66% 4t | S | the ONLY pii surface in real use: Hermes made 184 tool calls (deployed binary dated 2026-09-09) |
| tests/mcp_eof.rs | bounded lifetime on stdin close | 65L n/a 1t | T |  |

**crates/pii-memory/** (28 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 30L | C |  |
| src/bm25.rs | Okapi BM25 over redb postings | 91L 98% 0t | H |  |
| src/consolidate.rs | decay refresh, merge, archive | 191L 92% 0t | H |  |
| src/decay.rs | temporal decay | 57L 87% 1t | H |  |
| src/doc.rs | documents, layers, content ids | 256L 93% 1t | H |  |
| src/embed.rs | deterministic trigram embedder | 90L 83% 0t | H |  |
| src/entity/archive.rs | audited archive and restore | 436L 78% 0t | H |  |
| src/entity/hub.rs | shared slot and health seam | 102L 84% 0t | H |  |
| src/entity/mod.rs | entities module root | 28L n/a 0t | U-cov |  |
| src/entity/query.rs | bounded query surface | 191L 99% 0t | H |  |
| src/entity/store.rs | entity store: one file, atomic, locked | 575L 83% 0t | H | has the lockfile pii-learn lacks (:303) |
| src/entity/tier.rs | hot/warm/cold tiers | 330L 93% 0t | H |  |
| src/entity/types.rs | record types, content ids | 371L 91% 0t | H |  |
| src/entity/upsert.rs | pure write decision | 120L 97% 0t | H |  |
| src/graph_lane.rs | concept co-occurrence lane | 102L 46% 0t | S |  |
| src/lib.rs | crate root | 24L n/a 0t | U-cov |  |
| src/observe.rs | memory observability seam | 78L 85% 0t | H |  |
| src/quant.rs | i8 quantisation | 82L 100% 1t | H |  |
| src/rrf.rs | reciprocal-rank fusion | 36L 100% 0t | H |  |
| src/search.rs | fusion search | 145L 100% 0t | H |  |
| src/tokenize.rs | tokenisation | 43L 100% 1t | H |  |
| src/vector.rs | vector lane | 55L 93% 0t | H |  |
| tests/consolidate.rs | consolidation rules | 185L n/a 6t | T |  |
| tests/entities.rs | entities and relationships | 479L n/a 16t | T |  |
| tests/lanes.rs | lane semantics | 262L n/a 8t | T | F4: `db()` at :30 is a fixed redb path; 2 of 6 concurrent rounds fail `Database already open` (C-run); DO-NOT-REPEAT 1.48 class |
| tests/observe_contract.rs | search/consolidate reach the observer | 64L n/a 2t | T |  |
| tests/persistence.rs | vectors persisted with the document | 120L n/a 1t | T |  |
| tests/tiering.rs | access, tiers, demotion | 490L n/a 13t | T |  |

**crates/pii-model/** (14 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 44L | C |  |
| examples/ds4_inprocess.rs | in-process llama benchmark | 142L n/a 0t | U-cov |  |
| src/anthropic.rs | Anthropic Messages provider | 246L 85% 0t | H |  |
| src/gemini.rs | Gemini provider | 271L 87% 0t | H | Q4/Q8: third wire protocol, outside the user's 'openai + anthropic' directive |
| src/lib.rs | model provider seam | 517L 94% 7t | H |  |
| src/llamacpp.rs | in-process llama-cpp-2 provider | 437L n/a 6t | U-cov | feature-gated; clippy-only in the gate, never executed |
| src/llamacpp_server.rs | `llamacpp-server:<model>` kind (129) | 43L 95% 0t | H | the kind the deployed config uses and the deployed binary lacks (F1) |
| src/llamacpp_tools.rs | llama tool schema helpers | 201L 93% 4t | H |  |
| src/observed.rs | ObservedProvider wrapper | 160L 100% 1t | H |  |
| src/openai.rs | OpenAI-compatible provider | 369L 84% 0t | H |  |
| src/wire.rs | shared HTTP plumbing | 214L 97% 0t | H |  |
| tests/live_smoke.rs | live smoke per provider (ignored without keys) | 78L n/a 3t | T |  |
| tests/provider_kinds.rs | openai/anthropic/gemini kinds | 671L n/a 21t | T |  |
| tests/real_granite.rs | real-model suite (feature-gated) | 264L n/a 8t | T |  |

**crates/pii-net/** (20 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 38L | C |  |
| src/authorize.rs | pure egress decision | 224L 89% 0t | H |  |
| src/clock.rs | clock re-export | 4L n/a 0t | U-cov |  |
| src/dial.rs | outbound dialling | 79L 88% 0t | H |  |
| src/grant.rs | egress grants | 199L 98% 3t | H |  |
| src/health.rs | egress health | 21L 100% 0t | H |  |
| src/http_codec.rs | HTTP/1.1 codec | 346L 88% 6t | H |  |
| src/lib.rs | crate root | 50L 0% 0t | S |  |
| src/limits.rs | shared limits | 31L 100% 0t | H |  |
| src/listen.rs | listener management | 388L 74% 0t | H |  |
| src/resolve.rs | name resolution | 63L 87% 0t | H |  |
| src/service/conn.rs | grant-charging connection | 207L 85% 0t | H |  |
| src/service/dechunk.rs | incremental chunked decoding | 218L 85% 3t | H |  |
| src/service/http.rs | HttpTransport for NetService | 135L 77% 0t | H |  |
| src/service/mod.rs | the egress chokepoint | 318L 98% 1t | H |  |
| src/service/observe.rs | egress observability seam | 102L 100% 2t | H |  |
| src/testing.rs | test fakes | 250L 75% 0t | H |  |
| tests/accept_pool.rs | bounded accept pool | 223L n/a 3t | T |  |
| tests/egress.rs | egress discipline | 478L n/a 12t | T |  |
| tests/http_codec.rs | codec integration | 102L n/a 5t | T |  |

**crates/pii-observe/** (17 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 21L | C |  |
| src/audit.rs | security-relevant event subset | 142L 93% 4t | H |  |
| src/event.rs | leveled structured events | 283L 95% 6t | H |  |
| src/filelog.rs | daemon log file observer | 284L 93% 5t | H |  |
| src/health.rs | subsystem health reporting | 148L 98% 3t | H |  |
| src/latency.rs | latency histograms | 230L 99% 5t | H |  |
| src/lib.rs | crate root | 50L n/a 0t | U-cov |  |
| src/measure.rs | numeric observations | 48L 100% 1t | H |  |
| src/names.rs | event and measure name registry | 510L 100% 3t | H |  |
| src/observer.rs | observer trait | 292L 95% 5t | H |  |
| src/outcome.rs | run outcome | 81L 96% 1t | H |  |
| src/redaction.rs | redaction at the trajectory sink | 115L 89% 1t | H |  |
| src/retention.rs | trajectory retention | 177L 94% 0t | H |  |
| src/rss_trend.rs | RSS growth per run | 135L 93% 4t | H |  |
| src/secrets.rs | secret scrubbing | 120L 100% 3t | H |  |
| src/trajectory.rs | trajectory sink | 726L 95% 17t | H | writes partial files for a panicked run (F2) |
| tests/rss_probe.rs | trajectory-sink RSS probe | 75L n/a 1t | T |  |

**crates/pii-proto/** (14 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 34L | C |  |
| schema/event.json | event schema | 633L | C |  |
| schema/run.json | run schema | 741L | C |  |
| schema/session.json | session schema | 93L | C |  |
| schema/sys.json | sys schema | 84L | C |  |
| src/client.rs | protocol client | 404L 88% 0t | H |  |
| src/event.rs | server-to-client push vocabulary | 326L 100% 2t | H |  |
| src/lib.rs | wire contract | 318L 86% 5t | H |  |
| src/method.rs | closed method vocabulary (34) | 196L 100% 0t | H |  |
| src/run.rs | run/graph params and results | 120L n/a 0t | U-cov |  |
| src/session.rs | session params and results | 108L n/a 0t | U-cov |  |
| src/sys.rs | sys params and results | 326L n/a 0t | U-cov |  |
| tests/handshake_bound.rs | handshake never hangs | 144L n/a 5t | T |  |
| tests/wire_roundtrip.rs | wire round trip | 395L n/a 12t | T |  |

**crates/pii-server/** (87 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 52L | C |  |
| build.rs | build identity for sys.status | 39L n/a 0t | U-cov |  |
| src/daemon/adapters.rs | config → channel for 112 kinds | 203L 89% 0t | H |  |
| src/daemon/approvals.rs | approval resolution glue | 82L 95% 0t | H | `put_approval_if_absent` (spec 88) holds (C-read) |
| src/daemon/config.rs | daemon config | 911L 92% 14t | H | F7: `allow_declared` (:645) grants whatever a channel declares |
| src/daemon/identity.rs | sys.status identity | 621L 99% 4t | H |  |
| src/daemon/lag.rs | reactive-loop lag recovery | 315L 95% 3t | H |  |
| src/daemon/mod.rs | daemon lifecycle, timers, cron, approvals | 827L 79% 2t | H | Q7: test at :599 asserts the refusal event right after a status poll (:712-716), reproduced 3 of 3; `promotion_sweep` :49-57 is the only autonomous learn action |
| src/daemon/refusal_tests.rs | spec 131 tests | 290L n/a 8t | T |  |
| src/daemon/router.rs | channel router | 358L 94% 3t | H | pillar 2 demonstrated on HEAD (C-run) |
| src/daemon/scheduler.rs | timer scheduler | 164L 97% 1t | H |  |
| src/daemon/status.rs | daemon runtime context | 193L 81% 0t | H |  |
| src/daemon/suspend.rs | reaction to a suspension | 225L 91% 3t | H |  |
| src/engine/bus.rs | journal → bus bridge | 199L 94% 0t | H |  |
| src/engine/conn.rs | connection loop | 303L 84% 0t | H | heavy methods on `spawn_blocking` (80); ordering preserved |
| src/engine/conn_heartbeat_tests.rs | spec 80 tests | 188L n/a 9t | T |  |
| src/engine/dispatch.rs | method dispatch | 433L 53% 0t | S |  |
| src/engine/inline_measure_tests.rs | inline handler measurement | 297L n/a 2t | T |  |
| src/engine/loopback.rs | loopback transport | 16L 100% 0t | H |  |
| src/engine/methods.rs | daemon-only method handlers | 342L 42% 0t | S |  |
| src/engine/mod.rs | engine module root | 41L n/a 0t | U-cov |  |
| src/engine/refusal.rs | spec 131 deny decision | 95L 96% 0t | H | writes the heal ledger; join key = promotion id |
| src/engine/registry.rs | node registries | 276L 48% 1t | S | opens `lint-*.redb` in `runtime_dir` (:174-204): the 3 stray files at the repo root (F11) |
| src/engine/runs.rs | run registry, spawn on spawn_blocking | 862L 96% 15t | H | F3/Q7: `refuse_start` (:184-216) commits `Failed` before emitting `run.start.refused`; quarantine checked only at start (:128-150); F2: no unwind handling around the run thread |
| src/engine/session_durability.rs | session durability | 449L 83% 0t | H |  |
| src/engine/subscribe.rs | event subscribe/unsubscribe | 243L 54% 0t | S |  |
| src/engine/tool.rs | tool node kind | 287L 96% 5t | H |  |
| src/engine/uds.rs | UDS transport and auth | 100L 58% 1t | S |  |
| src/lib.rs | server crate root | 663L 88% 0t | H |  |
| src/local_model_tests.rs | spec 129 tests | 64L n/a 3t | T |  |
| src/node_kinds/agent/memorize.rs | history seeding, MEMORIZE markers | 52L 97% 0t | H |  |
| src/node_kinds/agent/mod.rs | agent node kind | 265L 96% 0t | H | uses document memory, not entities (F9) |
| src/node_kinds/agent/tests.rs | agent node tests | 751L n/a 15t | T |  |
| src/node_kinds/agent/tools.rs | agent tool execution | 154L 85% 0t | H |  |
| src/node_kinds/agent/turn.rs | agent turn loop | 342L 85% 2t | H |  |
| src/node_kinds/compaction.rs | conversation compaction | 217L 98% 4t | H |  |
| src/node_kinds/mod.rs | node-kinds module root | 13L n/a 0t | U-cov |  |
| src/node_kinds/nodes/classify.rs | classify node (128) | 238L 96% 3t | H |  |
| src/node_kinds/nodes/crash.rs | debug_crash node | 61L 11% 0t | S | F2: a node panic leaves the run 'in flight' forever with no crash file (C-run) |
| src/node_kinds/nodes/entity.rs | remember/recall nodes | 334L 86% 0t | H | used only by examples and one unwired graph (F9) |
| src/node_kinds/nodes/gate.rs | approval gate node | 310L 98% 3t | H |  |
| src/node_kinds/nodes/hal.rs | hal node | 251L 78% 0t | H |  |
| src/node_kinds/nodes/heal.rs | heal_detect node | 487L 57% 3t | S |  |
| src/node_kinds/nodes/memory.rs | memory consolidate node | 123L 76% 1t | H |  |
| src/node_kinds/nodes/mod.rs | daemon node kinds root | 19L n/a 0t | U-cov |  |
| src/node_kinds/nodes/mqtt.rs | mqtt publish node | 188L 92% 2t | H |  |
| src/node_kinds/nodes/post.rs | post node (128) | 508L 95% 9t | H |  |
| src/node_kinds/nodes/sop.rs | SOP bookkeeping nodes | 324L 92% 3t | H |  |
| src/node_kinds/nodes/spawn.rs | spawn child run node | 142L 92% 0t | H |  |
| src/node_kinds/nodes/wasm.rs | wasm node | 157L 82% 0t | H |  |
| src/node_kinds/nodes/worker.rs | worker node | 112L 5% 0t | S |  |
| src/observed_provider.rs | provider resolution and wrapping | 76L 95% 0t | H |  |
| src/tiers.rs | model tier table | 156L 98% 0t | H |  |
| tests/agent_line_gate.rs | agent module line gate | 79L n/a 1t | T |  |
| tests/approvals.rs | approvals | 116L n/a 2t | T |  |
| tests/boss_watch.rs | boss-watch graph | 815L n/a 9t | T |  |
| tests/cancellation_brake.rs | cancellation and budget brake | 252L n/a 5t | T |  |
| tests/channel_adapters.rs | webhook channel end to end | 285L n/a 4t | T |  |
| tests/channels.rs | channels | 101L n/a 2t | T |  |
| tests/entities_e2e.rs | entity store through a daemon | 312L n/a 3t | T |  |
| tests/fail_port.rs | provider failure to the fail port | 154L n/a 1t | T |  |
| tests/failover_graph.rs | failover graph | 289L n/a 3t | T |  |
| tests/hal_e2e.rs | HAL through a daemon | 347L n/a 6t | T |  |
| tests/handshake.rs | handshake and version skew | 209L n/a 4t | T |  |
| tests/health_identity.rs | sys.status identity | 266L n/a 5t | T |  |
| tests/latency.rs | session.prompt latency | 102L n/a 1t | T |  |
| tests/learn_e2e.rs | extraction over real-engine trajectories | 83L n/a 1t | T |  |
| tests/log_wiring.rs | observability seams attached | 260L n/a 8t | T |  |
| tests/memory_rpc.rs | memory RPC round trip | 125L n/a 1t | T |  |
| tests/model_routing.rs | model routing | 204L n/a 6t | T |  |
| tests/model_tiers.rs | model tiers | 499L n/a 10t | T |  |
| tests/mqtt_e2e.rs | MQTT through a daemon | 349L n/a 5t | T |  |
| tests/operator_surfaces.rs | operator surfaces must not lie | 194L n/a 3t | T |  |
| tests/peercred.rs | peercred refusal | 77L n/a 2t | T |  |
| tests/post_e2e.rs | post node through a daemon | 511L n/a 8t | T |  |
| tests/reattach.rs | reattach without duplicates | 137L n/a 1t | T |  |
| tests/run_get_states.rs | run.get "not yet" vs "unknown" | 159L n/a 2t | T | does not cover the panic case (F2) |
| tests/scheduler.rs | timer wheel, cron, scheduler | 67L n/a 3t | T |  |
| tests/session.rs | session durability | 233L n/a 7t | T |  |
| tests/slice_e_live.rs | slice E live | 145L n/a 4t | T |  |
| tests/slice_f_promotion.rs | slice F promotion | 175L n/a 4t | T |  |
| tests/sop_engine.rs | SOP end to end | 280L n/a 3t | T |  |
| tests/spawn_node.rs | spawn node and worktree | 244L n/a 2t | T |  |
| tests/status_latency.rs | sys.status latency | 136L n/a 1t | T |  |
| tests/tiering_e2e.rs | tiering through a daemon | 210L n/a 1t | T |  |
| tests/wasm_e2e.rs | plugin host through a daemon | 289L n/a 5t | T |  |
| tests/worktree_join_containment.rs | containment at the caller path | 96L n/a 1t | T |  |

**crates/pii-store/** (20 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 28L | C |  |
| src/approval.rs | approval records | 262L 89% 1t | H |  |
| src/backup.rs | backup and restore of redb files | 322L 87% 4t | H |  |
| src/channel.rs | channel cursor | 180L 87% 1t | H |  |
| src/codec.rs | serde_json codec helpers | 26L 82% 0t | H |  |
| src/compact.rs | offline compaction | 184L 94% 2t | H |  |
| src/heal.rs | heal-attempt ledger | 764L 96% 10t | H |  |
| src/identity.rs | store identity | 380L 88% 4t | H |  |
| src/lib.rs | the only crate that knows redb | 813L 86% 10t | H |  |
| src/mem/mod.rs | memory index store | 179L 80% 0t | H |  |
| src/mem/read.rs | memory read APIs | 289L 85% 0t | H |  |
| src/mem/write.rs | memory write APIs | 168L 85% 1t | H |  |
| src/session.rs | durable session substrate | 593L 87% 10t | H |  |
| src/stamp_tests.rs | format stamp tests | 70L n/a 1t | T |  |
| src/tables.rs | redb table definitions | 109L n/a 0t | U-cov |  |
| src/testing.rs | fault-injection seams | 113L 100% 0t | H |  |
| src/timer.rs | timer wheel store | 177L 86% 1t | H |  |
| tests/mem_store_contract.rs | MemStore contract | 252L n/a 7t | T |  |
| tests/store_contract.rs | journal/index/snapshot contract | 143L n/a 5t | T |  |
| tests/write_amp.rs | write amplification by table | 156L n/a 1t | T |  |

**crates/pii-tools/** (9 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 31L | C |  |
| src/bash.rs | bash mediation | 344L 93% 8t | H | the tool Hermes calls most (171 calls) |
| src/fs.rs | filesystem tools | 360L 98% 5t | H |  |
| src/lib.rs | built-in tool set | 180L 100% 1t | H |  |
| src/memory.rs | memory_search / memory_put tools | 248L 88% 4t | H |  |
| src/observe.rs | tool observability seam | 126L 100% 1t | H |  |
| src/sandbox.rs | Landlock sandbox profiles | 174L 43% 2t | S |  |
| src/search.rs | grep and glob | 355L 89% 5t | H |  |
| src/worktree.rs | worktree tool | 415L 92% 10t | H |  |

**crates/pii-tui/** (9 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 30L | C |  |
| src/driver.rs | TUI driver loop | 211L 54% 5t | S |  |
| src/input.rs | input handling | 190L 12% 0t | S |  |
| src/lib.rs | crate root | 15L n/a 0t | U-cov |  |
| src/observe.rs | TUI observability seam | 61L 100% 1t | H |  |
| src/render.rs | rendering | 206L 65% 0t | S |  |
| src/state.rs | TUI state types | 171L 96% 0t | H |  |
| tests/observe_contract.rs | attach failure reaches the observer | 27L n/a 1t | T |  |
| tests/render.rs | render purity | 207L n/a 5t | T |  |

**crates/pii-uds/**, **crates/pii-wasm/**, **crates/pii-web/**, **crates/pii-worker/** (22 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| pii-uds/Cargo.toml | manifest | 27L | C |  |
| pii-uds/src/lib.rs | client-side UDS helpers | 158L 80% 3t | H |  |
| pii-wasm/Cargo.toml | manifest | 28L | C |  |
| pii-wasm/src/config.rs | host configuration | 66L 100% 0t | H |  |
| pii-wasm/src/host.rs | WasmHost: compile and run modules | 480L 90% 0t | H | not fuzzed (122's own limit) |
| pii-wasm/src/hub.rs | late-bound slot from `[wasm]` | 40L 100% 0t | H |  |
| pii-wasm/src/lib.rs | crate root | 35L n/a 0t | U-cov |  |
| pii-wasm/src/state.rs | counters and resource limiter | 81L 75% 0t | H |  |
| pii-wasm/tests/host.rs | host limits and refusals | 487L n/a 18t | T |  |
| pii-web/Cargo.toml | manifest | 24L | C |  |
| pii-web/src/index.html | viewer page | 145L | C |  |
| pii-web/src/main.rs | read-only web viewer | 597L 65% 12t | S | F6: test scratch at :505 uses `std::env::temp_dir()`; 457 `/tmp/pii-web-mem-*` dirs leaked |
| pii-worker/Cargo.toml | manifest | 25L | C |  |
| pii-worker/src/control.rs | supervisor control plane | 679L 91% 5t | H |  |
| pii-worker/src/frame.rs | worker frames | 120L 88% 2t | H |  |
| pii-worker/src/health.rs | worker health | 27L 0% 0t | S |  |
| pii-worker/src/lib.rs | crate root | 15L n/a 0t | U-cov |  |
| pii-worker/src/observe.rs | supervisor observability seam | 102L 100% 1t | H |  |
| pii-worker/src/seams.rs | test seams | 65L 100% 0t | H |  |
| pii-worker/src/spec.rs | worker spec | 95L 100% 0t | H |  |
| pii-worker/src/supervisor.rs | supervisor | 369L 82% 0t | H |  |
| pii-worker/tests/supervisor.rs | supervisor integration | 445L n/a 10t | T |  |

**deploy/** (2 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Dockerfile | daemon image (104) | 29L | C | user directive: a spare; untested here |
| pii.service | systemd unit (104) | 50L | C | never run as a service on this host; no unit is installed |

**examples/** (18 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| crash-resume.toml | crash-resume graph | 38L ref11 | R |  |
| cut-release.sop.toml | worked SOP example | 41L ref3 | R |  |
| diamond.toml | diamond graph | 51L ref30 | R |  |
| emit-path.toml | emit-path graph | 14L ref7 | R |  |
| hal-blink.toml | blink an LED on the HAL | 72L ref4 | R |  |
| lint-bad-port.toml | lint fixture | 22L ref7 | R |  |
| lint-cap-escalation.toml | lint fixture | 14L ref6 | R |  |
| lint-one-armed-race.toml | lint fixture | 23L ref6 | R |  |
| lint-unbounded-cycle.toml | lint fixture | 28L ref6 | R |  |
| memory-recall.toml | read a fact back (126) | 23L ref4 | R | literal names; pillar 3 demonstrated with it (C-run) |
| memory-remember.toml | write a fact (126) | 40L ref4 | R | literal names (F9) |
| pii.production.toml | production config sample | 143L ref11 | R | stale: :108 says channel kind is 'cli' or 'discord' (5 kinds exist) |
| race-multi-arm.toml | race graph | 64L ref2 | R |  |
| race.toml | race graph | 51L ref184 | R |  |
| routed-diamond.toml | routed diamond | 66L ref12 | R |  |
| unsatisfiable.toml | unsatisfiable graph | 66L ref40 | R |  |
| wasm-plugin.toml | run a WASM plugin node | 14L ref7 | R |  |
| wasm/wrap.wat | plugin fixture | 19L ref3 | R |  |

**fixtures/** (13 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| phase2/fail.json | scripted model fixture | 3L ref6 | R |  |
| phase2/hello.json | scripted model fixture | 10L ref27 | R |  |
| phase2/selfchange.json | scripted model fixture | 35L ref6 | R |  |
| phase2/slow.json | scripted model fixture | 11L ref1 | R |  |
| phase3/discord/routes.json | Discord route fixture | 26L ref2 | R |  |
| phase3/memory/corpus.jsonl | memory eval corpus | 240L ref5 | R |  |
| phase3/memory/queries.jsonl | memory eval queries | 20L ref5 | R |  |
| phase3/model/assistant.json | scripted model fixture | 6L ref13 | R |  |
| phase3/model/atoms.json | scripted model fixture | 5L ref2 | R |  |
| phase3/ollama/chat.json | retired-kind fixture | 5L ref3 | R |  |
| phase3/soak.toml | rss-gate daemon config | 26L ref58 | R |  |
| phase3/workers.toml | worker-only daemon config | 7L ref56 | R |  |
| sop/cut-release-model.json | SOP model fixture | 6L ref2 | R |  |

**graphs/** (14 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| discord-digest.toml | recall + post a digest (128) | 73L ref6 | R | not wired into any deployment; section 3.2's proposed first win |
| discord-triage.toml | rule-based triage and reply (128) | 50L ref6 | R | not wired into any deployment |
| starter/assistant.toml | agent assistant graph | 52L ref89 | R | used for the pillar 2 demo (C-run) |
| starter/boss-watch.toml | boss watch loop | 93L ref14 | R |  |
| starter/consolidate.toml | memory consolidation | 34L ref55 | R |  |
| starter/crash.toml | deliberate crash trigger | 9L ref90 | R | used to demonstrate F2 (C-run) |
| starter/failover.toml | failover ladder | 181L ref34 | R |  |
| starter/heal-fix.toml | heal-fix graph | 196L ref25 | R |  |
| starter/heal-watch.toml | heal watch | 64L ref16 | R |  |
| starter/react.toml | react graph | 38L ref46 | R |  |
| starter/slow.toml | long-running test graph | 21L ref55 | R |  |
| starter/soak.toml | soak graph | 12L ref58 | R |  |
| starter/soak_cron.toml | soak cron graph | 22L ref4 | R |  |
| starter/verify-fix.toml | verify-fix graph | 63L ref37 | R |  |

**scripts/** (17 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| flake_hunt.py | flake hunting harness | 182L ref1 | R |  |
| leak_bisect.py | RSS leak bisect | 247L ref2 | R |  |
| mvp-verify.sh | MVP verification suite | 156L ref7 | R |  |
| phase2/selfchange.sh | E13 multi-file change | 65L ref8 | R |  |
| phase2/uds-vs-loopback.sh | E7 loopback vs UDS | 42L ref7 | R |  |
| phase3/crash.sh | F5.4 panic writes a crash file | 64L ref6 | R | expects `<runtime>/crashes/<ms>`; none was written for a node panic in my run (F2) |
| phase3/cron.sh | cron wiring | 82L ref4 | R |  |
| phase3/discord-e2e.sh | Discord channel end to end | 82L ref4 | R |  |
| phase3/hot-reload.sh | hot reload drops nothing | 83L ref4 | R |  |
| phase3/secret-scan.sh | bot token never leaks | 61L ref4 | R |  |
| phase3/shutdown.sh | SIGTERM with an active run | 69L ref5 | R |  |
| phase3/worker-kill.sh | kill -9 a live worker | 65L ref4 | R |  |
| run-batchA-implement.sh | workhorse lane dispatch | 46L ref0 | D? | no reference (F11) |
| run-batchC-claude.sh | batch C dispatch | 77L ref3 | R |  |
| run-rereview-opus.sh | re-review dispatch | 136L ref1 | R |  |
| run-rereview-resume.sh | finish the 044 re-review | 171L ref0 | D? | no reference (F11) |
| run-spec-direct.sh | direct implementation lane | 50L ref1 | R |  |

**specs/** (148 files)

| path | what it does | size | st | finding |
|---|---|---|---|---|
| 01-smoke-test.md | Smoke Test v2 | 26L | NC |  |
| 02-phase0-workspace.md | Phase 0 workspace skeleton | 236L | NC |  |
| 03-parallel-workers.md | Parallel workers | 36L | NC |  |
| 03-phase1-core-engine-part1.md | Phase 1 core engine 1/4 | 719L | NC |  |
| 03-phase1-core-engine-part2.md | Phase 1 core engine 2/4 | 719L | NC |  |
| 03-phase1-core-engine-part3.md | Phase 1 core engine 3/4 | 719L | NC |  |
| 03-phase1-core-engine-part4.md | Phase 1 core engine 4/4 | 719L | NC |  |
| 03-phase1-core-engine.md | Phase 1 core engine | 699L | NC |  |
| 04-phase2-client-server-part1.md | Phase 2 client-server 1 | 279L | NC |  |
| 04-phase2-client-server-part2.md | Phase 2 client-server 2 | 279L | NC |  |
| 04-phase2-client-server-part3.md | Phase 2 client-server 3 | 278L | NC |  |
| 04-phase2-client-server-part4.md | Phase 2 client-server 4 | 278L | NC |  |
| 04-phase2-client-server.md | Phase 2 client-server | 270L | NC |  |
| 05-phase3-assistant-gateway-part1.md | Phase 3 gateway 1 | 264L | NC |  |
| 05-phase3-assistant-gateway-part2.md | Phase 3 gateway 2 | 214L | NC |  |
| 05-phase3-assistant-gateway-part3.md | Phase 3 gateway 3 | 230L | NC |  |
| 05-phase3-assistant-gateway-part4.md | Phase 3 gateway 4 | 256L | NC |  |
| 05-phase3-assistant-gateway.md | Phase 3 gateway | 390L | NC |  |
| 06-phase3-mvp-hardening.md | MVP hardening | 184L | NC |  |
| 07-self-healing-slice-neg1-and-0.md | self-healing slices -1 and 0 | 213L | NC |  |
| 08-self-healing-slice1-local-worktree.md | self-healing slice 1 | 261L | NC |  |
| 09-global-memory.md | global memory v2 | 188L | NC |  |
| 10-graph-visualize.md | `pii graph show` | 247L | NC |  |
| 11-journal-hardening.md | journal hardening | 271L | NC |  |
| 12-web-viewer.md | `pii web` viewer | 209L | NC |  |
| 13-phase4-turn-loop-hardening.md | turn-loop hardening 4.1a | 186L | NC |  |
| 14-phase4-turn-loop-hardening-4.1b.md | turn-loop hardening 4.1b | 144L | NC |  |
| 15-phase4-streaming.md | streaming | 150L | NC |  |
| 16-phase4-session-durability-part1.md | session durability 1/2 | 134L | NC |  |
| 16-phase4-session-durability-part2.md | session durability 2/2 | 148L | NC |  |
| 17-phase4-tls.md | TLS | 148L | NC |  |
| 18-phase4-service.md | service | 133L | NC |  |
| 19-phase4-web-viewer.md | web viewer | 152L | NC |  |
| 20-phase4-boss-approval-gate-fix.md | boss approval gate fix | 163L | NC |  |
| 21-phase4-pii-mcp.md | pii-mcp | 165L | NC |  |
| 22-phase4-agent-loop-works.md | agent loop works | 176L | NC |  |
| 23-phase4-pii-evolve.md | pii-evolve | 225L | NC |  |
| 24-silent-noop-check.md | silent-noop check | 243L | NC |  |
| 25-pii-evolve-honesty.md | pii-evolve honesty | 224L | NC |  |
| 26-promotion-prerequisites.md | promotion prerequisites | 246L | NC |  |
| 27-boss-loop-graph.md | boss loop graph | 189L | NC |  |
| 28-model-routing.md | model routing | 179L | NC |  |
| 29-cancellation-brake.md | cancellation and brake | 181L | NC |  |
| 30-timer-nodes-resume.md | timer nodes resume | 132L | NC |  |
| 31-protected-paths.md | protected paths | 121L | NC |  |
| 32-spawn-node.md | spawn node and worktree | 132L | NC |  |
| 33-slice-e-live.md | slice E live | 135L | NC |  |
| 34-slice-f-dev-prod-promotion.md | slice F dev/prod promotion | 98L | NC |  |
| 35-in-process-llama-cpp.md | in-process llama.cpp | 107L | NC |  |
| 36-in-process-llama-cpp-tool-calls.md | llama.cpp tool calls | 95L | NC |  |
| 36-part1-llamacpp-tools.md | llamacpp tools helpers | 57L | NC |  |
| 36-part2-integrate.md | tool-call grammar integration | 155L | NC |  |
| 37-failover-reason-classify.md | failover reason classify | 265L | NC |  |
| 38-http-fetch-tool-via-pii-net.md | HTTP fetch via pii-net | 62L | NC |  |
| 39-structured-provider-failure.md | structured provider failure | 278L | NC |  |
| 40-agent-fail-port.md | failure as payload | 310L | NC |  |
| 41-failover-ladder-graph.md | failover ladder graph | 230L | NC |  |
| 42-failover-reason-operator-surface.md | failover reason on operator surface | 198L | NC |  |
| 43-split-agent-module.md | split agent.rs | 138L | NC |  |
| 44-split-silent-noop-module.md | split silent_noop.rs | 148L | NC |  |
| 45-split-service-module.md | split service.rs | 164L | NC |  |
| 46-heal-attempt-generic-extra-field.md | HealAttempt extra field | 169L | NC |  |
| 47-pii-server-module-boundaries.md | pii-server module boundaries | 164L | NC |  |
| 48-split-mem-store-module.md | split mem.rs | 129L | NC |  |
| 49-split-run-module.md | split run.rs | 135L | NC |  |
| 50-split-pii-tui-lib.md | split pii-tui lib | 155L | NC |  |
| 51-extract-xtask-cli-parse.md | extract xtask arg parsing | 156L | NC |  |
| 52-sync-blocking-seam-d4-d7.md | sync-blocking seam fixes | 168L | NC |  |
| 53-mem-store-contract-tests.md | MemStore contract tests | 146L | NC |  |
| 54-bounded-accept-pool.md | bounded accept pool | 133L | NC |  |
| 55-pii-proto-wire-roundtrip-tests.md | wire round-trip tests | 138L | NC |  |
| 56-pii-tools-bash-fs-worktree-tests.md | tools test coverage | 151L | NC |  |
| 57-pii-channels-discord-tests.md | discord tests | 138L | NC |  |
| 58-pii-cli-util-ndjson-tests.md | cli util tests | 127L | NC |  |
| 59-pii-web-arg-parsing-tests.md | web flag parsing tests | 132L | NC |  |
| 60-f11-pilot-store-error-taxonomy.md | F-11 pilot | 136L | NC |  |
| 61-fix-hardcoded-tmp-paths-and-relocate-docs.md | hardcoded `_tmp` paths | 175L | NC |  |
| 62-worktree-signature-validation.md | worktree signature validation | 140L | NC |  |
| 63-fix-pii-web-memory-search-key.md | pii-web memory key | 87L | NC |  |
| 64-apply-worker-restart-backoff.md | worker restart backoff | 110L | NC |  |
| 65-wire-same-layer-only-consolidation.md | same_layer_only | 173L | NC |  |
| 66-f11-pass2-store-op-constructor-session.md | F-11 pass 2 | 146L | NC |  |
| 67-gate-hygiene-lintfiles-clippy-counter.md | gate hygiene | 186L | NC |  |
| 68-pii-cli-test-gaps-and-hygiene-batch.md | pii-cli test gaps | 176L | NC |  |
| 69-lint-files-report-mode.md | lint-files report mode | 109L | NC |  |
| 70-chunked-decoder-split-safety.md | chunked decoder split safety | 81L | NC |  |
| 71-bash-render-byte-boundary-truncate.md | bash render byte boundary | 59L | NC |  |
| 72-reachability-check-measures-real-callers.md | reachability real callers | 66L | NC |  |
| 73-boss-watch-load-robust-deadline.md | boss_watch deadline | 56L | NC |  |
| 74-mutation-closure-of-partly-guarded-fixes.md | mutation closure | 68L | NC |  |
| 75-docs-correction-pass-043-s9.md | docs correction pass | 52L | NC |  |
| 76-xtask-path-hygiene-and-feature-gate.md | xtask paths and llama gate | 42L | NC |  |
| 77-journal-flush-lossless-and-must-use.md | journal flush lossless | 45L | NC |  |
| 78-resume-fails-loud.md | resume fails loud | 36L | NC |  |
| 79-lagged-control-event-recovers.md | lagged control event | 58L | NC |  |
| 80-heavy-rpc-off-the-async-worker.md | heavy RPC off the worker | 62L | NC |  |
| 81-methods-moves-to-engine.md | methods.rs to engine | 39L | NC |  |
| 82-f11-pass3-store-structured-errors.md | F-11 pass 3 | 42L | NC |  |
| 83-lanes-tests-hermetic.md | lanes tests hermetic | 54L | NC | incomplete: `db()` at lanes.rs:30 (F4) |
| 84-fresh-tree-gate-run.md | fresh-tree gate run | 65L | NC |  |
| 85-heartbeat-tests-hermetic.md | heartbeat tests hermetic | 46L | NC |  |
| 86-fresh-tree-sweep-and-tool-hygiene.md | fresh-gate affordable | 60L | NC |  |
| 87-reachability-is-stateful.md | reachability stateful | 73L | NC | landed (C-read) |
| 88-lag-recovery-idempotent-approvals.md | lag recovery approvals | 71L | NC | landed (C-read) |
| 89-gate-compiles-llamacpp.md | gate compiles llama | 51L | NC | landed (C-run) |
| 90-doctest-comment-truth.md | doctest comment truth | 51L | NC | landed (C-run) |
| 91-mutation-verification-batch-cd.md | mutation verification C/D | 87L | NC |  |
| 92-close-green-mutants.md | close green mutants | 67L | CL |  |
| 93-replay-err-divergence-measurement.md | replay error measurement | 53L | NC |  |
| 94-replay-handles-completion-error.md | replay completion error | 74L | NC |  |
| 95-bounded-shutdown-and-measure-inline.md | bounded shutdown | 91L | CL |  |
| 96-scratch-helper-kills-n1-class.md | scratch helper | 87L | NC | the lint it created does not cover `temp_dir()` (F6) |
| 97-pii-observe-contract-crate.md | pii-observe contract crate | 88L | NC |  |
| 98-lint-observability-enforcement.md | lint-observability | 109L | CL |  |
| 99-health-identity-store-mounts-version-counts.md | health identity | 105L | NC |  |
| 100-adoption-sweep-server-side.md | adoption sweep 1 | 97L | NC |  |
| 101-adoption-sweep-clients-channels.md | adoption sweep 2 | 87L | NC |  |
| 102-harden-observability-enforcement.md | harden enforcement | 87L | NC |  |
| 103-trajectory-substrate-for-self-improvement.md | trajectory substrate | 110L | NC |  |
| 104-p0-deployable.md | P0 deployable | 97L | NC | its unit was never installed on this host (F1) |
| 105-p1-operable.md | P1 operable | 87L | CL |  |
| 106-operator-surfaces-must-not-lie.md | operator surfaces | 124L | NC |  |
| 107-p2-confinement-and-secrets.md | P2 confinement and secrets | 110L | NC |  |
| 108-p3-coverage-and-adequacy.md | P3 coverage | 126L | CL |  |
| 109-no-hang-and-no-dead-names.md | no hang, no dead names | 158L | CL |  |
| 110-p5-load-and-soak.md | P5 load and soak | 102L | NC | no closure section, unlike 111-132 |
| 111-provider-kinds.md | provider kinds | 106L | CL |  |
| 112-channels.md | channels | 111L | CL |  |
| 113-sop-engine.md | SOP engine | 94L | CL |  |
| 114-model-tiers.md | model tiers | 117L | CL |  |
| 115-operator-surfaces-under-load.md | operator surfaces under load | 108L | CL |  |
| 116-cost-findings.md | cost findings | 108L | CL |  |
| 117-mqtt.md | MQTT | 101L | CL |  |
| 118-the-ungated-checks.md | ungated checks gated | 89L | CL |  |
| 119-load-stable-suite.md | load-stable suite | 115L | CL |  |
| 120-supervisor-reload-race.md | supervisor reload race | 115L | CL |  |
| 121-hardware-hal.md | hardware HAL | 97L | CL |  |
| 122-wasm-plugin-host.md | WASM plugin host | 102L | CL |  |
| 123-extraction.md | extraction | 134L | CL |  |
| 124-verifier-shadow-runs.md | verifier shadow runs | 102L | CL |  |
| 125-promotion-and-lifecycle.md | promotion and lifecycle | 118L | CL | L1-L5 reproduced on HEAD (C-run) |
| 126-entities-and-relationships.md | entities and relationships | 109L | CL |  |
| 127-memory-tiering.md | memory tiering | 106L | CL |  |
| 128-discord-graphs.md | post node and Discord graphs | 144L | CL | Q7 flake now explained |
| 129-retire-local-ollama.md | retire local ollama | 140L | CL | the deployed binary predates it (F1) |
| 130-evolve-dials-through-uds.md | evolve dials through UDS | 101L | CL |  |
| 131-refused-run-is-a-decision.md | refused run is a decision | 133L | CL |  |
| 132-discord-forum-post.md | Discord forum post | 147L | CL | F7 nuance on declared egress |

**xtask/** (28 files)

| path | what it does | size·cov·tests | st | finding |
|---|---|---|---|---|
| Cargo.toml | manifest | 28L | C |  |
| observability-allowlist.txt | per-file raw-print budget | 24L | C |  |
| observability-exemptions.txt | crate exemptions (2) | 2L | C |  |
| src/bench.rs | cold-start benchmark gate | 140L 7% 1t | S |  |
| src/cli_parse.rs | xtask argument parsers | 138L 15% 0t | S |  |
| src/coverage_floor.rs | coverage ratchet | 81L 53% 3t | S |  |
| src/deps.rs | dependency DAG purity gate | 562L 91% 5t | H |  |
| src/doc_drift.rs | doc-drift check | 171L 74% 4t | H |  |
| src/fresh_gate.rs | CI-equivalent export run | 397L 50% 8t | S | skips coverage by default |
| src/heal_collect.rs | heal-collect | 398L 45% 8t | S |  |
| src/heal_prune.rs | heal-prune | 84L 0% 0t | S |  |
| src/heal_seed.rs | heal-seed | 214L 0% 0t | S |  |
| src/lib.rs | xtask library surface | 58L 86% 0t | H |  |
| src/lint_files.rs | 450-line effective gate | 336L 81% 10t | H |  |
| src/lint_obs_io.rs | exemption validity | 69L 100% 3t | H |  |
| src/lint_observability.rs | observability seam and print ban | 709L 85% 16t | H |  |
| src/lint_scratch.rs | bare `_tmp` scratch-site check | 138L 62% 4t | S | F6: checks only `tempdir_in(` on `_tmp`; a `temp_dir()` site passes |
| src/main.rs | xtask runner | 506L 54% 7t | S |  |
| src/memory_eval.rs | memory-eval | 197L 13% 0t | S |  |
| src/net_check.rs | net-check gate | 286L 63% 4t | S |  |
| src/promote_build.rs | promote-build | 225L 15% 2t | S |  |
| src/proto_gen.rs | proto-gen | 102L 0% 0t | S |  |
| src/reachability.rs | reachability-check (stateful since 87) | 627L 98% 11t | H |  |
| src/rss_gate.rs | rss-gate | 168L 0% 0t | S |  |
| src/silent_noop/helpers.rs | silent-noop helpers | 142L 95% 0t | H |  |
| src/silent_noop/mod.rs | silent-noop check | 320L 72% 8t | H |  |
| src/silent_noop/scans.rs | the five silent-noop scans | 262L 72% 0t | H |  |
| tests/spec72_reachability.rs | reachability integration tests | 266L n/a 4t | T |  |
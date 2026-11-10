<!-- Filed 2026-11-03T02:54:05Z from an Opus plan-mode analysis run.
     Produced by: Claude Code + claude-opus-5-5, plan mode, READ-ONLY (no project writes).
     Repo HEAD at analysis time: d35dcf3 (tree: 0 changed paths)

     STATUS: NOT a spec and NOT a plan of record. Nothing in this document is authorised work.
     Every proposal needs its own pre-registered gate before implementation, and any idea previously
     closed by a measurement stays closed until a NEW measurement reopens it.

     Anything marked "asserted-from-memory" is NOT verified — treat it as a lead, not a fact. -->

# pii — optimisation report and 90-day plan (read-only analysis, 2026-11-03, HEAD d35dcf3)

## Context
This was a read-only review of pii 0.2.0, the single-binary Rust daemon and CLI deployed on `the host`. The brief asked for a ranked list of findings, a per-crate coverage table, the premises in the brief that turned out to be wrong, and a sequenced plan for 90 days. Nothing was written to the repository.

Evidence came from:
- the source;
- `/tmp/gate-rel5.log`, the newest full-gate log;
- read-only commands on `the host` (`pii health` timings, `stat`, `sha256sum`);
- three read-only exploration passes.

`cargo` was not run, so all test timings come from the gate log.

---

## Part A — Premises in the brief that are wrong

| # | Brief says | Measured | How |
|---|---|---|---|
| A1 | 25 crates | **23 product crates + `xtask` = 24 workspace members.** `crates/_tmp/phase3/slice-e-it` is a stray directory that is not a member and has 0 `.rs` files. | `Cargo.toml [workspace].members`, `ls crates` |
| A2 | `pii-server` is 47.7k LOC over 224 files (implying a god-crate) | **src = 22,331 LOC in 97 files** (average 230 lines). **tests = 25,366 LOC in 127 files.** More than half the crate is tests. | `find … -path '*/tests/*' -prune` |
| A3 | 30 node kinds | **28 registered kinds**: 7 in `pii-graph/src/node.rs:81-87` and 21 in `pii-server` (`engine/registry.rs`, `lib.rs:284-294`). The 30 entries in `nodes/` include about 14 helper files. `call` is **not a kind**: it is a compile-time weave (`pii-graph/src/fragment/call.rs`). | registry grep |
| A4 | `store.redb` 18.8 MB / `memory.redb` 12.6 MB "after weeks of production" | The host stores have **byte-identical sizes to the dev machine's stores**: 18,812,928 and 12,587,008, also identical in the 2026-11-02 22:12 backup manifest. Host `pii health` reports `since_deploy.runs: 0`, `unattributed: 1850` of 1,895 runs, `cron_entries: 0`, `channel_posts: 0`, `mqtt_received: 0`. **The size is inherited from development, not grown in production.** The current production growth rate is about 0. | `stat`, `sha256sum`, `pii health` on the host |
| A5 | M7 "retirements are not executed (gemini, matrix, gpiod, ollama, telegram pending)" | `_docs/05-operations/m7-decisions.md` records only **one** retirement: the local `ollama.service`, which is an operator host step. gemini, matrix and gpiod are **postponed**, which means "code stays, compiled and linted" (line 20). telegram is **kept** ("near future"). Nothing is waiting for an agent to delete. | m7-decisions.md:8-15 |
| A6 | "The product is one Rust binary" (as a fact) | The workspace builds **4 binaries**: `pii`, `pii-evolve`, `pii-mcp` and `pii-web`. pii-evolve, pii-mcp, pii-web and pii-http-fetch have **zero reverse dependencies**, so they are not in `pii`, yet every gate compiles, lints and tests them. The `pii-mcp` dependency tree has 242 entries (rmcp) and `pii-web` has 95 (axum). | `cargo tree`, Cargo.toml |
| A7 | §2.3: backoff "honours `retry_after` as a floor" | In the code, `retry_after` is the **base of the doubling series** (`poll.rs:191-193`: `max(interval, retry_after·2^(n-1))`), not only a floor. Also, the doc comment at `poll.rs:175` still says `2^min(n-1, 4)`, which is the old 16× cap the fix removed. | read poll.rs |
| A8 | 32 CLI subcommands | There are 32 dispatch arms. They are **30 named subcommands** plus `-p` and `--version`. | grep of `main.rs:174-273` |
| A9 | 364 specs | 367 | `ls specs \| wc -l` |

These premises check out: 287 test files, 57 files over 450 raw lines, 172 docs, the `pii 0.2.0 (0853699…)` live version, and 7 stores.

---

## Part B — Ranked findings
Rank is (benefit × confidence) / risk. Effort is in worker passes (WP).

| # | Title | Where | Evidence | Benefit | Risk | Measure (before/after) | Effort |
|---|---|---|---|---|---|---|---|
| **1** | **The test daemon-slot lock leaks, and the next daemon test waits about 180 s.** | `crates/pii-cli/tests/daemon_guard/mod.rs:34-38,82-86` | `Slot::acquire` makes the *first* acquirer own the file. Later concurrent acquirers get `path: None`. The file is removed only when the dropping slot sees `left == 1`. If the first owner drops while another test still holds the slot, the file is never removed, and the next binary waits for the 180 s stale-steal. The waits match exactly:<br>• `memory_learn` waited **152.89 s**. It ran after `health_drift` (4 daemon spawns) and about 27 s of other binaries: 180 − 27 = 153.<br>• `skill_cli_reads_through_the_daemon` waited **178.78 s**. It ran after `second_daemon_refuses` and about 1.6 s of other binaries.<br>A leaked file exists right now: `crates/pii-cli/_tmp/daemon-slot.lock` (Nov 2 17:51). | **About 330 s of the 634.5 s test runtime** (about 5.5 min per full gate) | ~0 (test code only) | Sum of `finished in` in a new full-gate log. Expected: both binaries under 10 s. A new test in `daemon_guard_holds.rs`: acquire A, acquire B, drop A, drop B, then assert the lock file is absent. **It fails on HEAD.** | 1 |
| **2** | **CLI store commands default to the tmpfs runtime dir, so they act on stale stores.** | Defaults via `pii_uds::runtime_dir("default")`: `cmd_backup.rs:32`, `cmd_store.rs:24`, `cmd_skill.rs:119,136`, `cmd_entity.rs:43`, `cmd_learn.rs:58`, `graph_dirs.rs:34`, `cmd_graph_installed.rs:206,254`. The daemon's state dir is resolved by `daemon/mod.rs:207` (`PII_STATE_DIR`, then `config::state_dir`). That rule is copied three times: `daemon/mod.rs:207`, `cmd_treat.rs:53`, `cmd_daemon.rs:145`. | On the host, `/run/user/1000/pii/store.redb` is **561,152 B, dated Oct 24**. The live store is `~/.local/state/pii/store.redb`, 18,812,928 B. In an ssh shell there is no `PII_*` env and no `~/.pii/pii.toml`, so `pii backup <p>` without `--dir` **backs up the stale tmpfs store set**. | Backups and compaction act on the real data | Low–medium (changes the CLI default) | New test: with `PII_RUNTIME_DIR=A` and `PII_STATE_DIR=B`, and stores only in B, `pii backup out` must produce a manifest whose `store.redb` blake3 equals the hash of `B/store.redb`. **It fails on HEAD.** | 1 |
| **3** | **The timer scheduler can spin, and it loses a timer if a resume fails.** | `daemon/scheduler.rs:61,65-68,87` | `take_due(..).unwrap_or_default()`. On a redb error the due row stays due and the loop `continue`s without an await, which busy-loops a tokio worker. `take_due` deletes the row *before* `spawn_resume`. If the process crashes or `spawn_resume` returns `Err`, the run stays `Suspended` forever. The boot path never rescans Suspended runs: `lag::recover_suspended` (`daemon/lag.rs:35`) runs only after a bus `Lagged` (`mod.rs:328`). | Prevents a permanently stuck run and a CPU spin | Low | Test A: a wheel whose `take_due` fails must emit an event and back off; no more than 2 polls in 1 s of paused time. Test B: a Suspended run with no timer row is re-armed or resumed by `Daemon::build` + boot. **Both fail on HEAD.** | 1 |
| **4** | **A claimed queue row stays `Running` forever after a crash.** | `pii-store/src/queue.rs:251-290`, `decide` at `:356`; `session_durability.rs:325-368` | `decide` returns `AlreadyRunning` while any item is `Running`, so one stuck row blocks the whole queue. There is no lease, and no code sets `Pending` again outside tests. Boot reconcile touches only `store.redb`. `graphs/night.toml` depends on this queue. | The night composition cannot wedge | Low | Test: claim; reconcile (the claiming run becomes `Failed`); call `claim_next` again. Expected: `Claimed`, plus a `queue_event` `requeued_after_restart`. **It fails on HEAD.** | 1 |
| **5** | **`pii health` replays every journal of every run on every call.** | `daemon/identity.rs:289` → `pii-graph/src/agent_calls.rs:237-273`; second scan `daemon/since_deploy.rs:42-98`; runs inline (`engine/dispatch.rs:43`, not in `is_heavy` `conn.rs:213`) | On the host: **83 / 85 / 94 ms** for 1,895 runs and 41,150 events. Cost is linear, so 10× is about 0.85 s and blocks one of 4 async workers. | Constant-time health; no worker stall | Low | `for i in $(seq 10); do t0; pii health; t1; done` on the host. Expected p50 under 20 ms. Synthetic test: 20,000 runs, `sys.status` under 50 ms. | 1 |
| **6** | **The channel cursor drops errors, and a poll that panics counts as a success.** | `daemon/router.rs:56` (`.unwrap_or(Ok(()))` on `JoinError`), `:168` (cursor read error becomes `None`, so the channel replays from the start), `:205` (`let _ = advance`); `pii-store/src/channel.rs:3-4` claims "a crash cannot duplicate" | The cursor advances *after* `start_run_with`, so delivery is at-least-once and the doc comment is false. | No silent replay storm; the backoff sees panics | Low | Tests: (a) a poll closure that panics gives `CHANNEL_DEGRADED`; (b) a failed cursor read gives a failed poll, not a replay; (c) the channel.rs doc reads "at-least-once". | 1 |
| **7** | **Memory df is double-counted on re-put, and each put is O(all postings).** | `pii-store/src/mem/write.rs:71-82` (`bump_df` runs even when `existed`), `:128-141` (iterates the whole `MEM_TERM` table inside a write txn; `seen: Vec` makes it O(t²) per doc) | BM25 IDF is skewed for re-indexed docs, and total ingest is O(N²). 1,801 docs on the host. | Correct ranking; ingest becomes linear | Low | Test: `put(d)` twice, then df(term) == 1. **It should fail on HEAD; write it first to confirm.** Bench: time for 2,000 puts before and after. | 1 |
| **8** | **A run that panics, or a resume with unreadable input, ends silently.** | `engine/run_pool.rs:83` (`let _ = catch_unwind`: the header stays `Running`, the handle leaks, no event); `pii-graph/src/run/resume.rs:123-129` (store or parse error gives `Value::Null` input); `engine/runs.rs:514-518` (resume failure goes to `eprintln!` only) | Read in code | No run is left lying about its status | Low | Tests: a factory that panics leads to run `Failed("panicked")` plus an event; a corrupt input body leads to `RUN_RESUME_REFUSED`. | 1 |
| **9** | **The lint registry is missing two kinds, and registration is listed three times.** | `engine/registry.rs:83,86` (`debug_crash`, `spawn`) are absent from `lint_registry_with_log` `:197-272`; the third list is `lib.rs:279-299` | `pii graph lint graphs/starter/crash.toml` gives "unknown node kind" | Lint matches the runtime; about 40 lines deleted | Low | `pii graph lint graphs/starter/crash.toml` exits 0. A test asserts that the kind-name sets of the daemon and lint registries are equal. | 1 |
| **10** | **A `pii-channels` test pulls in `pii-server`, a dev-dependency cycle.** | `crates/pii-channels/tests/poll_backoff.rs:16-18,130,…` uses `pii_server::daemon::router`; `pii-channels/Cargo.toml` dev-deps include `pii-server`, `pii-learn`, `pii-tools` | `cargo test -p pii-channels` compiles the 209-entry server dependency tree | The quick "touched crates" gate for channels drops the server compile | ~0 | `cargo build --timings -p pii-channels --tests`. Afterwards the units list contains no `pii-server`. | 1 |
| **11** | **Cron and cancel tests wait on the wall clock.** | `pii-server/tests/cron_runtime.rs:75` (waits for a real `* * * * *` minute, plus 6 s settle), `cancelled_run_disarms.rs` (a 2-minute graph); `daemon/cron.rs:108-110` re-arms through `std::thread::sleep` | 50.5 s and 67.7 s in rel5 | About 110 s per gate | Medium (cron must take the injected `Clock`; `Daemon::build` already takes `Arc<dyn Clock>`) | Both binaries under 5 s. The assertions are unchanged (diff of the `assert!` lines is empty). | 2 |
| **12** | **Two store tables are written but never read.** | `RUN_SNAPSHOT`: written by `pii-graph/src/journal.rs:341` every 32 events; the only reader is `pii-store/tests/store_contract.rs:132`. `RESUME_INDEX`: written by `sched/dispatch.rs:423`; no reader. | One extra fsync write txn per 32 events and per resume-key node, and unbounded growth | Fewer fsyncs; less growth | Medium (a future spec might want snapshots) | `cargo test -p pii-graph` (resume, crash tests) green. `store.info()` shows the snapshot table length stops growing. **The safety proof for this deletion is the pii-graph resume and suspend test set.** | 1 |
| **13** | **`graph.lint` creates four redb files on disk per call.** | `engine/registry.rs:174,241,247,258` (`Database::create` of `lint-*.redb`), inline on an async worker | On the host, `/run/user/1000/pii/lint-{approval,heal,memory,queue}.redb` are 36,864 B each | About 8 fsync txns per lint RPC removed | Low | `strace -c -e fsync` around one `pii graph lint` call. Expected: 0 fsyncs (switch to `redb::backends::InMemoryBackend`). | 1 |
| **14** | **The promotion sweep re-parses every file every 5 s.** | `daemon/learn_loop.rs:19-29` → `pii-learn/src/promote/mod.rs:52-58,386`, `storage.rs:118-135` | Each sweep does a truncating `File::create(".writer.lock")`, a flock, `read_dir`, and a JSON parse of up to 10,000 files. That is **17,280 sweeps a day** on an idle host. | The idle daemon becomes nearly free | Low | On the host, `strace -c -f -p $(pidof pii)` for 60 s, before and after. Expected `openat` count down at least 90%. Skip the sweep when the directory mtime is unchanged. | 1 |
| **15** | **The worker limit `max_in_flight` is configured but not enforced, and the worker event log is unbounded.** | `pii-worker/src/spec.rs:16`; `daemon/mod.rs:376` passes it; `control.rs:28-75` never reads it; `supervisor.rs:100,180` `events: Vec` grows forever | The config value does nothing | Bounded load | Low | Test: `max_in_flight = 1` with two invokes; the second is refused with a typed error. Events are capped at 1,024. | 1 |
| **16** | **Node-kind boilerplate is repeated.** | `pii_graph::nodes::eval_ctx` is `pub(crate)` (`nodes.rs:9-15`), with 11 copies (`worker.rs:27`, `spawn.rs:35`, `tool.rs:59`, …). The deep `$.` resolve has 3 copies (`worker.rs:93-112`, `spawn.rs:127-146`, `tool.rs:81-101`). Required-string config extraction appears about 30 times; `coder/helpers.rs:10-28` is private. `ok`/`err` outcome code appears 21 times. | — | About 550–650 lines deleted (about 6% of `node_kinds`) | Low | `git diff --stat` shows net deletions of at least 400 lines; the full gate is green; the test count is unchanged (inventory, see W1). | 2 |
| **17** | **CLI helpers are copied, and one copy loses observability.** | A local `fail()` shadows `cli_util::fail` and **skips `CliObserver`** (`cmd_skill.rs:106`, `cmd_entity.rs:88`, `cmd_learn.rs:107`, `cmd_index.rs:38`). `usage()` has 9 copies. `print_result` is re-inlined 5 times (`cmd_ctl.rs:119,236`, `cmd_sys.rs:247`, `cmd_metrics.rs:407`, `cmd_health.rs:159`). There are 28 or more hand-written flag loops. | — | About 400 lines deleted; CLI failures become observable | Low | `rg -c 'fn fail\(' crates/pii-cli/src` = 1 (only `cli_util`). The `lint-observability` step is green. | 2 |
| **18** | **The store has three error styles and copied CRUD code.** | `PiiError::store_op` (146 uses) vs `queue.rs:139` `internal` (40) vs inline `Internal(format!)` (130: `mem/*`, `timer.rs`, `approval.rs`, `channel.rs`). get/put/open bodies are copied in `channel.rs:72`, `approval.rs:143`, `heal.rs:193`, `queue.rs:210`, and so on. | — | About 200–270 lines deleted; one error shape | Low | `rg -c 'PiiError::Internal\(format!' crates/pii-store/src` = 0. | 2 |
| **19** | **There is no retention policy, and no measurement to base one on.** | Append-only forever: RUN, RUN_JOURNAL, RUN_SNAPSHOT, GRAPH_BODY, RESUME_INDEX, SESSION_*, queue_*, APPROVAL, MEM_* (only MEM_DOC shrinks). `compact.rs:10-12` admits it. Compaction is offline only (`pii store compact`). | Per run: 9.9 KB per run (upper bound, 18.8 MB / 1,895) and 21.7 events per run. Memory: 7.0 KB per doc. | Growth becomes visible before it hurts | Low | **Measure first:** `pii store stats` prints rows and bytes per table. Record it weekly on the host for 4 weeks, then write the policy (W6). | 1 + 2 |
| **20** | **Two config claims are stale or false.** | `poll.rs:175` doc (16× cap); `daemon/mod.rs:441-452` reload updates `poll_interval_ms`, but `router.rs:40` captured the interval at spawn, so reload does not change the pace | Read in code | The doc and reload tell the truth | Low | Test: reload a new interval and the next sleep uses it, or document "restart required" and remove the field from reload. | 1 |

Operator decisions (not worker passes): see W9.

---

## Part C — Answers to the questions

**Q1 Crate boundaries.**
- Only two boundaries cost more than they return.
  - **The `pii-channels` → `pii-server` dev-dependency cycle (finding 10).** One test file causes it.
  - **Four crates outside the product** (pii-web, pii-evolve, pii-mcp, pii-http-fetch). They add rmcp (242-entry tree) and axum (95) to every clippy, build and test step, while contributing nothing to `pii`.
- Next is `pii-wasm`. wasmtime is non-optional (122-entry tree) for a kind with 0 graphs in `graphs/`.
- Merging the small crates (pii-uds at 181 src LOC, pii-abi, pii-observe) saves nothing measurable and churns 4 to 21 dependents. **Do not merge them.**
- Measure the compile effect with `cargo build --timings --workspace` before and after any removal.

**Q2 Is `pii-server` a god-crate?**
- **No. Leave it.**
- src is 22.3k LOC over 97 files (average 230 lines), in three balanced slices: node_kinds 9.9k, engine 5.9k, daemon 5.3k.
- `node_kinds` → `engine` has one real code edge (`nodes/queue.rs:301` `learn_now_secs`). `engine` → `node_kinds` has five edges (`registry.rs:18`, `session_durability.rs:30`, `runs.rs:105`, `run_scope.rs:116-119`, `orphans.rs:18-20`).
- The only slice worth extracting, if ever, is the **coder family**: 3,094 LOC, 0 references to `daemon`, 1 to `engine::runs`. Extracting it would break:
  - the 5 engine imports above;
  - `crate::tiers` (it would have to move);
  - `compaction`;
  - 131 test paths `pii_server::node_kinds::…`, which a `pub use` re-export would keep working.
- The compile benefit is small, because the server still depends on it. Not in the 90 days.

**Q3 Node kinds (28).**
- **Keep all 28 now.** No registered kind can be expressed as a `call`, because `call` is a compile-time splice, not a runtime kind.
- Actions:

| Kind(s) | Status | Action |
|---|---|---|
| `debug_crash`, `spawn` | Missing from the lint registry (finding 9) | Register them |
| `emit` | 0 graphs, examples only, shared `node.rs` | Keep: tiny |
| `remember` / `recall` | `remember` has 0 graphs; they are a pair | Keep |
| `hal` | Covered by the M7 "postponed" decision | Keep |
| `wasm` | 0 graphs; no operator decision | Operator decision |
| `mqtt` node | Overlaps the `mqtt` tool; the mqtt channel is in the production path (M7) | Operator decision after the graphs are checked |
| `memory` | A single op (consolidate) | Keep |
| `sop`, `require` | 0 TOML uses but generated (`sop/compile.rs:96`, `call_needs.rs:138`) | **Live — must stay primitive** |
| `transform`, `route`, `wait`, `timer`, `refuse`, `agent`, `tool`, `gate`, `coder`, `post`, `select`, `judge`, `record` | Used | **Primitive: keep** |

- `agent_worker.rs` (546 raw) is misnamed: it is a `coder` helper.

**Q4 Hot path.** The host is nearly idle: 0 runs since deploy and 0 cron entries. Its steady-state time goes to:
1. the 5 s promotion sweep (finding 14);
2. channel polls (one read txn each when healthy; cheap);
3. `pii health` at 85 ms and linear (finding 5).

The three highest-leverage changes:

| # | Change | Predicted effect | Measure |
|---|---|---|---|
| (a) | Health counters O(1) (finding 5) | p50 from 85 ms to under 20 ms; flat in run count | `pii health` timing on the host |
| (b) | Sweep only when the directory mtime changes (finding 14) | `openat` at least 90% lower | `strace -c` for 60 s |
| (c) | Lint in-memory and stop the write-only snapshot txn (findings 13 and 12) | 8 fsyncs per lint go to 0; one fsync fewer per 32 events | `strace -c -e fsync` |

- redb commits are all `Immediate` (no `set_durability`). This is correct for a crash-safe journal; do not change it (Part D).

**Q5 Tests.**
- The test runtime is 634.5 s. About 52% of it is **lock wait, not tests** (finding 1). About 17% is wall-clock cron (finding 11).
- The remaining step time (about 270 s) is compile and link of about 287 test binaries at `-j 2`.
- The smallest honest partition:
  1. Fix 1 and 11; the full step is predicted at about 7–8 min.
  2. Keep the existing `--quick` list (touched crates) for the inner loop.
  3. Add a control: `xtask test-inventory` writes `cargo test --workspace -- --list` (name set plus count; rel5 had 2,084 passed and 14 ignored) to `gate-test-inventory.txt`. The full gate fails if the live set differs from the file and the file did not change in the same commit. Then no reorder, partition or consolidation can drop a test silently.
- Overlap verified: `poll_backoff.rs` exists in both `pii-channels` and `pii-server`. They merge when finding 10 moves the file.

**Q6 Duplication.** Findings 16, 17 and 18 come to about 1,150–1,300 deletable lines in total. Shared helpers already exist but are under-exported: `eval_ctx`, `coder/helpers::text/texts`, `cli_util::fail`, `print_result`, `PiiError::store_op`.

**Q7 Data growth.**
- The growth law cannot be read from file size, because redb never shrinks and the host files are a copy of dev (A4).
- Derived rates: about 9.9 KB per run (upper bound), 21.7 events per run, 7 KB per memory doc. The production rate is about 0 runs a day.
- At 10× (about 19k runs, about 190 MB):
  - health grows to about 0.85 s (fixed by finding 5);
  - memory ingest becomes 100× slower because of O(N²) `bump_df` (fixed by finding 7);
  - the queue and channel stores stay tiny.
- Policy, written after 4 weekly `pii store stats` samples:
  - It lives in `pii-store` as `prune_runs(before_ms, statuses)`.
  - Config: `[retention] terminal_run_days = 30` for Succeeded and Refused runs, `failed_run_days = 90`, never Running or Suspended.
  - It deletes RUN, RUN_JOURNAL, RUN_SNAPSHOT and RUN_BY_STATUS rows, then the unreferenced GRAPH_BODY rows.
  - The daemon runs it daily from the timer wheel. It is followed by an offline `pii store compact` at install time (the installer already stops the daemon).

**Q8 Errors and observability.**
- Errors whose cause is lost: `scheduler.rs:61,87`; `router.rs:56,168,173,205`; `run_pool.rs:83`; `resume.rs:123-129`; `runs.rs:514-518`; `session_durability.rs:80,106,113,285-296` (empty blob or empty transcript on a serde or store error); `pii-store/src/timer.rs:107,142,159` (unreadable rows skipped); `mem/write.rs:55,117,150`; `pii-worker/src/control.rs:243-247`; `lib.rs:360` (`next_session_id().unwrap_or(1)` can reuse `s1`); `daemon/cron.rs:~62` (a store error starts a duplicate cron run).
- Paths that write **no event**:
  - queue claim;
  - timer fired, and `take_due` errors;
  - resume start, success and failure;
  - cursor advance failure;
  - a poll that panics;
  - a run job that panics;
  - a worker respawn whose thread spawn fails.

**Q9 Concurrency.**
- The invariant that is currently implicit: **"every durable claim (timer row taken, queue row Running, channel message started) is either completed or re-offered at boot."** It is broken three times: findings 3, 4 and 6.
- What breaks first under load:
  1. the run-pool queue: an **unbounded** `std::sync::mpsc` in front of 8 threads (`run_pool.rs:60`, no backpressure);
  2. the 8-thread blocking pool shared by polls, heavy RPCs and the sweep (`cmd_daemon.rs:194-203`);
  3. inline heavy work on async workers: `sys.status`, `graph.lint`, cron re-arm `start_run_with` (`cron.rs:~140`), `fire_due_sync` → `spawn_resume` prep (`runs.rs:376-460`).
- Other unbounded structures:
  - the `sessions` map (`lib.rs:152`);
  - `channel_runs` and `cron_runs` (only cleaned up when the finish event arrives);
  - the worker events `Vec`;
  - per-connection `unbounded_channel` (`conn.rs:135`).
- Untracked spawns: `suspend.rs:24`, and one raw thread per cron re-arm (`cron.rs:108`).

**Q10 What NOT to do.** Ranked by the harm each would do:
1. **Do not split `pii-server` into crates** (Q2 numbers).
2. **Do not merge the small crates** (no measurable gain, wide churn).
3. **Do not switch redb to `Durability::Eventual` or batch fsync.** The journal promises crash safety, and the gain is irrelevant at about 0 runs a day.
4. **Do not write a retention policy before `pii store stats` exists** (finding 19); it would be a guess.
5. **Do not replace redb, add SQLite, or add online compaction.** redb compaction needs exclusivity, and offline compaction already exists.
6. **Do not convert node kinds into `call` compositions.** `call` is a splice; nothing qualifies.
7. **Do not rewrite the CLI on clap.** Duplication is fixed by helpers (finding 17), without a dependency or a 30-command rewrite.
8. **Do not consolidate all 287 test binaries at once.** Shared process state would interfere (the `HELD` static, env vars, cwd). Pilot only `pii-graph`, and only after W1 if link time is still above 3 min.
9. **Do not adopt nextest now.** It is a new gate tool dependency, and findings 1 and 11 recover about 440 s for free. Re-measure first.
10. **Do not remove the per-node barrier thread** (`sched/barrier.rs:23`) **or make the poll path async.** There is no load to justify it.
11. **Do not delete postponed M7 code** (gemini, matrix, gpiod, telegram). The recorded decision says it stays compiled (m7-decisions.md:20).
12. **Do not touch a file at 446–450 effective lines without splitting it first:** `daemon/config.rs` 448, `pii-tools/src/mqtt.rs` 450, `cmd_treat.rs` 448, `pii-memory/src/entity/store.rs` 446. Any added line fails `lint-files`.

---

## Part D — Coverage (one row per workspace member; 24, see A1)

| Crate | src / test LOC | What it is | The one thing to change |
|---|---|---|---|
| pii-server | 22,331 / 25,366 | Daemon: engine, daemon, node kinds | Scheduler spin and timer loss (finding 3) |
| pii-cli | 12,915 / 10,025 | The `pii` binary | Store-dir default points at tmpfs (finding 2) |
| pii-graph | 8,641 / 8,545 | Graph engine, journal, `call` weave | `resume.rs:123` resumes with Null input on a store error (finding 8) |
| pii-learn | 4,209 / 2,613 | Learning loop, incidents | 5 s full re-parse sweep (finding 14) |
| pii-store | 5,365 / 582 | The only crate that knows redb | df double-count and O(N²) put (finding 7) |
| pii-memory | 3,797 / 1,701 | BM25, vector and entity memory (JSON files) | OK. `entity/store.rs` is at 446/450 effective; split it before the next edit |
| pii-channels | 3,402 / 1,598 | Inbound poll and outbound send | Dev-dependency cycle on `pii-server` (finding 10); stale doc at `poll.rs:175` |
| pii-net | 2,899 / 1,257 | The one socket (rustls) | OK (not deep-read) |
| pii-model | 2,600 / 1,143 | Provider seam | OK (not deep-read) |
| pii-observe | 3,452 / 75 | Events, file log | OK. `filelog.rs:158-176`'s unbuffered write per event is fine at current load |
| pii-tools | 2,980 / 313 | Built-in tools | `mqtt.rs` at exactly 450 effective; split it before any edit |
| pii-core | 2,915 / 177 | Capability context, clock, env | Home for the one `state_dir()` rule (finding 2) |
| pii-proto | 1,925 / 548 | Wire contract | OK |
| pii-worker | 1,477 / 441 | Worker supervision | `max_in_flight` not enforced; events unbounded (finding 15) |
| pii-abi | 1,509 / 0 | Kernel types, `PiiError` | OK. `store_op` is under-used (finding 18) |
| pii-hal | 1,106 / 298 | GPIO HAL | OK. M7 #5 postponed |
| pii-wasm | 702 / 487 | WASM host (wasmtime, not optional) | Operator decision: feature-gate or keep (W9) |
| pii-tui | 854 / 234 | TUI client | OK |
| pii-evolve | 392 / 401 | Separate binary, not in `pii` | Operator decision: retire (W9) |
| pii-http-fetch | 447 / 344 | GET fetch library; **no dependents** | Operator decision: retire (W9) |
| pii-web | 599 / 0 | Separate axum binary, not in `pii`, unauthenticated surface (opus-status-20261102 §2.1) | Operator decision: retire (W9) |
| pii-mcp | 431 / 65 | Separate MCP binary, not in `pii` | Operator decision: is it used? (W9) |
| pii-uds | 181 / 95 | Client socket dial and runtime-dir resolution | OK. Correct for the socket; stores must stop using it (finding 2) |
| xtask | 6,644 / 403 | Gate and tooling | Add `test-inventory` (W1) |

**NOT COVERED**

| Item | Why |
|---|---|
| pii-net, pii-model, pii-proto, pii-tui, pii-hal internals | Low churn (7–46 file-commits since Oct 1). Read only through Cargo.toml and lib docs. |
| `_docs/` (172) and `specs/` (367) | Only `m7-decisions.md`, `opus-status-and-path-20261102.md` and `067-…gate…` were read. |
| `scripts/`, `deploy/` installer, `modules/devbox` | Shell tooling; not product logic. |
| Release build time and binary size; the coverage-floor step | `cargo` was not run (plan mode, and a 13-minute cost). The timings are from `/tmp/gate-rel5.log` only. |
| Host behaviour under load | The host is idle (0 runs since deploy), so there is no load to observe. |
| `~/.config/pii/pii.toml` on the host | It holds `/root` paths (`runtime_dir = "/run/user/0/pii"`). pii's resolution order does not read it: the CLI reads `~/.pii/pii.toml` and the service reads `~/.local/pii/pii.toml`. **I could not rule out that Hermes or another job reads it.** |

---

## Part E — The plan: 9 workstreams over about 90 days

Every workstream ends with the full `cargo xtask fresh-gate` green. A worker never commits.

| WS | Content | Closes findings | Depends on | Gate (command → expected) | Risk | WP |
|---|---|---|---|---|---|---|
| **W1 — Honest, faster gate** | (a) Make the slot's file owned by a process-wide static and remove it when `HELD` reaches 0. (b) Move `pii-channels/tests/poll_backoff.rs` into `pii-server/tests/` and merge it with the server copy; drop the server, learn and tools dev-deps. (c) `xtask test-inventory`. | 1, 10, Q5 control | — | `cargo xtask fresh-gate` → PASS. In the log, `memory_learn` and `skill_cli_reads_through_the_daemon` both `finished in` under 10 s. `cargo xtask test-inventory --check` → 0. The new slot test fails when (a) is reverted. | ~0 | 3 |
| **W2 — Store paths tell the truth** | One `pii_core::state_dir()` (`PII_STATE_DIR`, then `$HOME/.local/state/pii`) replaces the 3 copies. The 8 CLI store defaults use it; the daemon's default `--store` becomes `<state_dir>/store.redb`. The socket stays in `runtime_dir`. | 2 | — | New `pii-cli/tests/store_dir_default.rs` (the blake3 test from finding 2) passes and fails on HEAD. On the host, after install: `pii backup /tmp/b` (daemon stopped) has a manifest `store.redb` size equal to the live file. | Low–medium | 2 |
| **W3 — Every durable claim completes or is re-offered** | Scheduler: error backoff, re-arm-before-delete or boot `recover_suspended` (finding 3). Queue requeue at boot (finding 4). Router cursor and `JoinError` (finding 6). Run-pool panic and resume Null (finding 8). | 3, 4, 6, 8 | W1 (cheaper gate cycles) | Each finding's named test passes and fails on HEAD. `incident-cases.sh` selftest is still 42/42. | Low | 4 |
| **W4 — Silent paths speak** | Events for queue claim, timer fired, timer error, resume start/fail, cursor advance failure, poll panic, worker respawn failure. Add the names to `lint-observability`. | Q8 list | W3 (same files) | `cargo xtask lint-observability` → 0. Per path, a test asserts the event name appears. | Low | 2 |
| **W5 — Daemon idles cheaply** | Health O(1): add `SysStatus` to `is_heavy`, and memoise totals keyed by journal length (finding 5). Sweep gated on mtime (finding 14). Lint on `InMemoryBackend` (finding 13). Enforce `max_in_flight`; cap events (finding 15). Reload-interval truth and stale doc (finding 20). | 5, 13, 14, 15, 20 | W1 | the host: health p50 under 20 ms (10 runs). `strace -c -f -p $(pidof pii)` for 60 s shows `openat` down at least 90%. Synthetic test: 20k runs, `sys.status` under 50 ms. | Low | 4 |
| **W6 — Data you can see, then bound** | `pii store stats` (rows and bytes per table, all 7 stores). Fix memory df and bump_df (finding 7). Stop writing RUN_SNAPSHOT and RESUME_INDEX (finding 12). **After 4 weekly the host samples:** `prune_runs` plus `[retention]` (Q7 policy). | 7, 12, 19 | W2 (correct dirs); the retention part waits about 4 weeks for data | `pii store stats` on the host prints all 7 stores. The df test passes. `cargo test -p pii-graph` is green after the snapshot write removal. A prune test: a 31-day-old Succeeded run is gone, and a Suspended run of any age is kept. | Medium (retention deletes data) | 5 |
| **W7 — Delete the copies** | Node-kind helpers and a single registration table (findings 16, 9). CLI `fail`, `usage`, `print_result`, flag helper (finding 17). Store `get_json`, `put_json`, `open_aux`, one error style (finding 18). | 9, 16, 17, 18 | W3 and W4 merged first (they touch the same files) | `pii graph lint graphs/starter/crash.toml` → exit 0. `git diff --stat` shows at least 1,000 net deleted lines. `test-inventory --check` → 0 (no test dropped). | Low | 6 |
| **W8 — Tests off the wall clock** | Cron consumes the injected `Clock`; `cron_runtime` and `cancelled_run_disarms` use `FixedClock` (`pii-core/src/clock.rs:33`) with identical assertions. Then re-measure, and run the `pii-graph` single-binary pilot only if link time is still above 3 min. | 11 | W1 | Both binaries under 5 s. The diff of their `assert!` lines is empty. Full test step under 8 min. | Medium | 3 |
| **W9 — Operator decisions (no worker pass until decided)** | (1) Retire pii-web, pii-evolve and pii-http-fetch? (2) Is pii-mcp used? (3) Feature-gate wasm the way `hal-gpiod` is (default off, plus one gate step)? (4) The installer keeps the previous binary as `~/.local/bin/pii.prev` (a §3 open item; installer shell is tooling). (5) Delete the dead `~/.config/pii/pii.toml` on the host. | A6, Q1, §3 | — | Per decision: a removal spec whose gate is `cargo tree -p pii-cli` unchanged (proving the removed crate was never in the product) plus a full gate PASS. For (4): `ls ~/.local/bin/pii.prev` after an install, and `pii.prev --version` prints the prior revision. | Low once decided | 1–4 |

**Sequence.**
- Weeks 1–2: W1, then W2.
- Weeks 2–5: W3, then W4.
- Weeks 3–6: W5 in parallel with W3. They touch disjoint files except `identity.rs`, so W5's health work merges after W3's.
- Weeks 4–6: the first half of W6 (stats, df fix, stop the snapshot writes).
- Weeks 6–9: W7.
- Weeks 5–8: W8.
- About week 10: the retention half of W6, once 4 samples exist.
- W9 whenever the operator answers.
- About 30–34 worker passes in total.

**Do first: W1.**
- It is three cheap passes with no product risk, and the evidence is arithmetic: the two waits match the 180 s steal minus the measured gap, and a leaked lock file is on disk now.
- It removes about 5.5 minutes from **every** later gate run, and the 30 or so later passes each pay that gate.
- Its inventory control is what makes W7's large deletions and W8's test edits provably safe, because no test can vanish without the gate failing.
- W2 is the most important *production* fix and starts immediately after. It goes second only because W1 makes its own gate cycle cheaper.

## Verification of this report
- Every finding names a test that fails on HEAD, or a measurement with a numeric threshold.
- Re-check the gate numbers by running `cargo xtask fresh-gate` once before W1, saving the log, and running the same `finished in` sum and heartbeat extraction used here on `/tmp/gate-rel5.log`.
- Re-check the host numbers with the same read-only `ssh <operator>@<host> 'pii health'` timing loop and `stat` of `~/.local/state/pii/*.redb`.

I read the contract sections (`001` Phase-3 scope + exit list, `002` §1.3/§1.4/§1.6, §2.7, §4, §5, §6.2, §8.7, §8.8, §12.2), the current tree, and verified the environment facts this spec depends on. Here is the spec. — Part 4/4

**Verified premises** (checked in this container, 2026-09-03, do not re-derive):

- Workspace members today: `pii-abi, pii-core, pii-store, pii-graph, pii-proto, pii-model, pii-tools, pii-server, pii-tui, pii-cli, xtask`.
- **`Cargo.lock` contains no HTTP client and no TLS stack** — no `ureq`, `rustls`, `ring`, `hyper`, `reqwest`, `axum`, `tantivy`, `hnsw_rs`, `fastembed`. `pii-model` has exactly five deps (`pii-abi`, `pii-core`, `serde`, `serde_json`, `thiserror`) and `provider_from_env("ollama:…")` returns `Invalid` — the HTTP provider was never landed.
- `agent-memory-core` is **not** vendored: `_repos/` holds only `grok-build, hermes-agent, pi_agent_rust, yantrikclaw, zeroclaw`.
- `crates.io` is reachable (`index.crates.io` 200, `static.crates.io` 200 for a real `.crate`), and `api.discord.com` answers — but **no exit criterion in this spec uses the network**.
- `PiiError::is_retryable()` is true for `Timeout` only, and `Ctx::deadline` is inert (HANDOFF caveat 12), so the scheduler's retry path in `sched/dispatch.rs:246` currently has **no live producer**. This spec gives it one.
- `pii-graph`'s registry has four built-in kinds (`transform, route, emit, wait`); `pii-server` registers `agent` and `tool` through `NodeRegistry::register`.
- `pii-store` owns the only `redb` dependency; tables are `meta, graph_body, run, run_journal, run_snapshot, run_by_status, resume_index`.
- `clippy.toml` already bans `tokio::net::{TcpListener::bind, TcpStream::connect, UnixListener::bind}`, `std::process::Command::{spawn,output,status}`, `std::time::SystemTime::now`, `std::env::var`. `std::net::*` is **not** yet banned.
- `cargo xtask` has `deps-check, lint-files, proto-gen, proto-check, bench-gate, ci`. `lint-files` hard-fails at 450 effective lines, warns at 380.

---

# Spec: Phase 3 - Assistant / daemon, networking, workers, memory


**Parent-Spec**: `05-phase3-assistant-gateway.md`
**Part**: 4 of 4
**Covers**: E16, E22-E30 (daemon lifecycle, timer wheel, cron, approvals, Discord end to end, the RSS/leak gate)
**Depends-On-Parts**: parts 1-3 (`NetService` + `FakeHttpServer`, the worker supervisor, `pii-memory` for `starter/assistant`)

## Overview

**Project**: /projects/pi-improved
**Phase**: 3 - Assistant / gateway (daemon, networking, workers, layered memory)

**Spec of record**: `_docs/06-research/001-pii-architecture-plan.md` §"Phase 3 — Assistant / gateway" (scope + the five exit criteria); `_docs/06-research/002-pii-architecture-plan-pass2.md` §4.1–§4.5 (`pii-net`, egress discipline), §5.1–§5.5 (`pii-memory`, L0–L3, RRF, embedder constraint), §6.2 (pidag subsumed — `after` edges, budget in observable units, delta verification), §8.7 (cron = graphs + timer wheel), §8.8 (approvals), §2.7 (daemon debuggability), §12.2 (dependency DAG), §1.3/§1.4/§1.6 (lints, 450-line gate, CI), §14 Phase 3.

**The load-bearing requirement.** `cargo xtask net-check` must pass: **`pii-net` is the only place in the workspace that opens a socket or resolves a name.** Every byte that leaves or enters this process passes `NetService::authorize()` — which resolves once, pins the `SocketAddr` set, validates address class, refuses mixed answers, and charges the grant (§4.1, §4.3). Phase 2's load-bearing invariant was a dependency edge that must not exist; Phase 3's is a call site that may exist in exactly one crate. Everything else in this phase — channels, workers, memory, cron, approvals — is subordinate to keeping that chokepoint honest, because it is the only thing that makes "the daemon owns sockets" a fact rather than a diagram.

The five phase exit criteria from `001` are behavioural and are all delivered (E21–E24, E12–E17); the chokepoint is the structural invariant that keeps them from being satisfied by a shortcut.

**Scope cuts — stated explicitly, as instructed.**

1. **The gateway is CUT** → `specs/06-phase3b-gateway-mcp.md`. No `axum`, no REST, no WebSocket, no dashboard, no SSE. Reason: none of the five load-bearing exit criteria touch it, the control protocol (`pii-proto`) already serves every client Pii has, and `axum + hyper + tower + http` is the largest new third-party subtree in the whole plan. `pii-obs` stage 2, `pii logs -f`, `pii top`, `pii prof` and pass-2 §14's exit 8 move with it, as does `pii-mcp` (`rmcp` needs HTTP+SSE and a quarantined `reqwest`).
2. **TLS is CUT** → 3b, and with it `https://` egress, remote transport, `pii pair`, bearer tokens, and talking to the real `api.discord.com`. `pii-net` speaks plaintext HTTP/1.1 over TCP. Reason (verified above): the workspace contains **no** TLS stack today, so TLS is a new third-party subtree (`rustls` + `ring`/`aws-lc-rs`, i.e. a C toolchain) that fights pass-1 §0's "no C deps in the default build" and that **no container-local exit criterion can exercise**. Phase 3 therefore ships the whole egress *policy* (grants, resolve-once-pin, class validation, deadlines, byte and connection budgets) and the plaintext transport; 3b adds a `TlsConnector` behind the unchanged `NetService::connect`. Honest consequence: **E15 runs Discord against a Discord-API-compatible fake on loopback**, and the live form (`PII_DISCORD_API_BASE=https://api.discord.com`) is documented in `scripts/phase3/README.md` as blocked on 3b. The `ollama:` provider, by contrast, becomes fully real — local Ollama is plaintext `http://127.0.0.1:11434`.
3. **`hickory-resolver` is CUT.** Resolution is `std::net::ToSocketAddrs` behind a `Resolver` trait, called **exactly once** per authorize. §4.3.2's actual requirement is resolve-once-then-pin, not a particular resolver; the trait keeps hickory a drop-in, and a `StaticResolver` makes the entire egress policy testable with no DNS at all.
4. **`agent-memory-core` is CUT as a dependency; `pii-memory` is first-party.** §5.1 says "embed, do not reimplement" — unexecutable here: the crate is not vendored and not published to this machine. Consequence: no `tantivy`, no `hnsw_rs`, no `fastembed`/ONNX. `pii-memory` implements pass-1 §6.3 directly: hand-rolled BM25 over redb postings, i8-quantised vectors with a streaming brute-force scan, a co-occurrence graph lane, RRF k=60, temporal decay. §5.5 itself says brute force wins below ~10k docs; HNSW is deferred until a measured corpus exceeds that.
5. **L2 scene *labelling* and L3 persona *generation* are CUT** (both are LLM calls). L2 communities are **computed** by label propagation (§5.2 is emphatic that membership must not be a model's opinion) and labelled deterministically from their top three concepts; L3 is a pinned document written by `memory.put{layer:"persona"}`. The L0–L3 *structure* and the §5.2 retrieval order ship; the two generative triggers do not.
6. **Consolidation's LLM conflict validation is CUT.** §5.4's incident class is made structurally impossible: `consolidate` makes **zero** model calls in this phase. The hard-won constants (`MIN_SHARED_CONCEPTS = 2`, `MIN_CONCEPT_JACCARD = 0.15`, `NEGATION_WINDOW = 8`) and a hard candidate cap ship as the near-duplicate merge rule, with the cap asserted by a test.
7. **The 72 h soak is not an exit criterion; the leak gate is.** No session in this container runs 72 h. `cargo xtask rss-gate` performs the *same measurement* — sample `VmRSS` from `/proc/<pid>/status` every N seconds, assert the **max**, not the mean (`001` is explicit about this) — and the phase gates on a 300 s window plus a **steady-state slope** assertion across 3,000 runs, which is what actually detects the failure mode a 72 h soak detects. The 72 h form is the identical command with `--secs 259200 --interval 60`, documented in `scripts/phase3/README.md` as the pre-release ritual and recorded there when run.
8. **Reactive `wait{Event{topic}}` triggers are CUT** → Phase 4 (§14 gives Phase 4 IoT/reactive). Cron ships, because §8.7's timer wheel is what makes an idle daemon cheap, and E21's RSS budget is meaningless without it.
9. **MQTT, BLE, serial, `ListenSpec::Uds`, WASM `protocol-handler`, `pii-sync`, `pii-mcu`, `pii-hw` are not in this phase** (§14 Phases 4–6).
10. **The control UDS listener stays in `pii-server::uds`** and is not migrated into `NetService`. It is the control plane, it already carries Phase 2's peercred gate and its own exit criteria, and moving it risks a working security boundary for no gain. `NetService::listeners()` reports it for `sys.status`; `net-check`'s allowlist names its two files. This is the one stated exception to the chokepoint.
11. **`pii acp` — planner decision recorded here.** Phase 2 deferred it (HANDOFF caveat 1). It is **cut to `specs/06-phase3b-gateway-mcp.md`**: ACP is a second client of the same protocol and belongs with the other client surfaces, not with the daemon's internals.

**Protocol version bumps to `pii.v2`.** The `Method` and `Event` enums are closed by design (Phase 2 F1.3/F1.7), so adding methods and event kinds is a breaking wire change and must be announced, not smuggled. `PROTOCOL_VERSION` becomes `"pii.v2"`. **This makes one existing test wrong, and updating it is required, not a weakening**: `server::hello_skew_is_hard_refusal` offers `"pii.v2"` as the *bad* version — it must now offer `"pii.v99"` and still assert code `100` with `data.expected == "pii.v2"`. No other Phase 1/2 test may change.

**Phase 2 dependencies this spec assumes** (all verified present): `pii_core::{Ctx, CapSet, CapKind, Budget, CancelToken, env_var}`, `Ctx::{fs, net, proc, child, cancelled, elapsed_ms, caps, budget, observer}`, `ProcCap::run`, `pii_store::Store::{open, next_run_id, put_graph, get_graph, append_event, get_run, read_journal, put_resume_key, runs_by_status}`, `pii_graph::{run_with_sink, resume, validate, NodeRegistry::register, GraphSpec, NodeSpec, RetryPolicy, GraphLimits, EdgeKind::{Data, Order}}`, `pii_abi::{RunEvent, RunHeader, Terminal, Outcome, SuspendReason, ResumeKey, NodeInput, PiiError, Effects, CapKind}`, `pii_proto::{Envelope, Method, Event, parse_frame, write_frame, Client}`, `pii_server::{Engine, EngineConfig, Bus, uds, conn::serve_connection, loopback}`, `pii_model::{ModelProvider, ScriptedProvider, ChatRequest, ChatResponse}`, `pii_tools::{Tool, ToolSet, sandbox}`. Where Phase 2 already provides an equivalent of something below, **reuse it and delete the duplicate requirement** rather than adding a second path.

---

## Requirements

### Functional

#### W5 — the daemon: lifecycle, timer wheel, cron, approvals

- **F5.1** `pii-server` gains `src/daemon/` (`mod.rs`, `config.rs`, `scheduler.rs`, `approvals.rs`, `router.rs`, `status.rs`). `pub struct Daemon` owns the `Engine`, the `NetService`, the worker `Supervisor` map, the channel router, the scheduler and the listener set. `pub async fn run(self, shutdown: ShutdownSignal) -> Result<(), PiiError>`.
- **F5.2** One config file: `pii daemon --config <path.toml>` (default: `<runtime_dir>/pii.toml` when present, else an empty config). Sections: `[net]` (`grants`, `allow_private`), `[[worker]]` (F3.3 fields), `[[channel]]` (`name`, `kind`, `graph`, `poll_interval_ms`), `[memory]` (`path`, `embedder`, `cache_bytes`), `[[cron]]` (`name`, `graph`, `schedule`, `input`). Parsed with the existing `toml` workspace dependency. An unknown key is an **error**, not a warning (`deny_unknown_fields`) — a typo'd grant must not silently become no grant.
- **F5.3** Graceful shutdown: `SIGTERM`/`SIGINT` (tokio `signal` feature) → stop accepting connections, cancel running runs, `shutdown` every worker, close listeners, flush the store, **exit 0 within 2 s**. A second signal aborts immediately with exit 130.
- **F5.4** Panic hook (§2.7): writes `<runtime_dir>/crashes/<now_ms>.json` = `{ts_ms, thread, location, message, run: Option<String>}` **synchronously** before the process dies, then delegates to the previous hook. Installed by `pii daemon` before anything is served. `panic = "abort"` in release does not skip hooks, so this works in both profiles.
- **F5.5** Timer wheel (§8.7): `pii-store` gains table `timer_wheel: (u64, &str) -> &str` (`(fire_at_ms, run_id) -> resume_key`) with `put_timer`, `next_timer`, `take_due(now_ms)`. `Scheduler` holds **one** `tokio::time::sleep_until` on the earliest entry, **rearmed on every insert** — not a poll loop (§8.7 is explicit). Firing is **one redb write transaction** that appends `Resumed` to the journal and pushes the run onto the ready set, so cross-process safety comes from redb's write lock. `Scheduler::wakeups()` exposes a counter for N5's test.
- **F5.6** `timer` node kind (`pii-graph`, a fifth built-in): cfg `{ in_ms: u64 }` **or** `{ cron: "<5 fields>" }`; returns `Outcome::Suspend { reason: SuspendReason::Timer { in_ms }, key: ResumeKey("timer:<run>:<node>:<fire_at_ms>") }`; on resume it takes port `fired`. **The existing `wait` node is not touched** — it blocks a thread by design and Phase 1/2 tests depend on it.
- **F5.7** `src/cron.rs`: a 5-field UTC cron parser (`min hour dom mon dow`) supporting `*`, `n`, `a-b`, `a,b`, `*/n` and any combination, with `pub fn next_after(&self, now_ms: u64) -> u64`. Pure, table-tested (F6 rows). An invalid expression is `PiiError::Invalid("cron: <detail>")` returned from the node factory's `build`, so `pii graph lint` rejects it at V0.
- **F5.8** Cron wiring: for each `[[cron]]` entry the daemon starts one run of the named graph at startup (the graph's entry is a `timer` node with that `cron`), which suspends immediately; when the wheel fires it, the graph body runs to completion and the daemon **starts a fresh run** for the next firing. One run per firing keeps each firing independently traceable in `pii graph trace` — which is the property §8.7 claims over hermes.
- **F5.9** Approvals (§8.8): `gate` node kind in `pii-server` (`src/nodes/gate.rs`), cfg `{ policy: String, prompt: String }`. On dispatch it consults an injected `ApprovalResolver`: if a decision for its deterministic approval id (`blake3(run_id, node_id)[..16]`) exists in the store, it returns `Next { port: "allow"|"deny", value }`; otherwise it returns `Outcome::Suspend { reason: Approval { policy, prompt }, key: ResumeKey("approval:<id>") }`. Because the decision is read from the store rather than from a future, **replay after a daemon restart is deterministic** — this is what makes "a 3-day approval and a 200 ms tool call are the same shape" true.
- **F5.10** `pii-store` gains table `approval: &str -> &[u8]` (`{approval_id, run_id, node, policy, prompt, requested_at_ms, decision: Option<…>, note, decided_at_ms}`) with `put_approval`, `get_approval`, `resolve_approval`, `pending_approvals`. `approve.resolve` writes the decision and appends `Resumed { key, payload_hash }` in one transaction, then continues the run via `pii_graph::resume`. Resolving twice is idempotent (`resumed: false` the second time). Resolving an unknown id is `ErrorCode::NotFound`.
- **F5.11** Events: suspension emits `run_suspended`, resumption `run_resumed`, and a gate additionally emits `approval_requested` / `approval_resolved`. CLI: `pii approve ls`, `pii approve allow <id> [--note N]`, `pii approve deny <id>`.
- **F5.12** `sys.status` is populated per F1.10, including `rss_kb` from `/proc/self/status`. `sys.reload` re-reads the config file and applies it to channels and workers (a worker whose `program` or `args` changed is reloaded per F3.6); it never restarts the process.
- **F5.13** `pii soak --connect <socket> --runs N --graph G [--concurrency C]` — a CLI driver that starts N runs through the control protocol, waits for each terminal, and prints `soak: runs=<N> ok=<n> fail=<n> ms=<total>`. It exists so `rss-gate` can drive real load through a real daemon over the real transport.
- **F5.14** `xtask rss-gate [--secs 300] [--interval 10] [--max-mb 50] [--runs 3000]`: starts `target/debug/pii daemon` with `fixtures/phase3/soak.toml` and a scripted model under `_tmp/phase3/soak/`, runs `pii soak` against it, samples `VmRSS` every `--interval` seconds for `--secs`, then asserts: (a) **max** RSS < `--max-mb` (assert the max, not the mean — `001` says so explicitly); (b) `rss_last - rss_at_10_percent < 2048 KiB` (the steady-state slope — this is the leak detector that a 72 h soak would eventually surface); (c) sample count ≥ `secs/interval - 1`; (d) the daemon is still answering `sys.status` at the end and exits 0 on `SIGTERM`. Prints `rss-gate: samples=<n> max_kb=<n> slope_kb=<n>` and exits 1 with `RSS-GATE FAIL: <which>` on any breach.

#### W6 — `pii-channels`: Discord in, reply out

- **F6.1** New crate `crates/pii-channels`. Dependencies: `pii-abi`, `pii-core`, `serde`, `serde_json`, `thiserror`. **No `pii-net`, no sockets** — everything goes through `Arc<dyn HttpTransport>` (F1.4), which is what makes the channel testable with a fake and keeps `net-check` clean.
- **F6.2** `pub trait Channel: Send + Sync { fn name(&self) -> &str; fn kind(&self) -> &str; fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError>; fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError>; }` with `Incoming { channel: String, chat: String, user: String, text: String, cursor: u64 }`. Polling, not listening — inbound webhooks need TLS and a public port, both cut.
- **F6.3** `DiscordChannel` (`src/discord.rs`): base URL from `PII_DISCORD_API_BASE` (default `https://api.discord.com`, which under cut 2 fails with the explicit `https requires the tls feature` error — the fake sets it to `http://127.0.0.1:<port>`), bot token from `PII_DISCORD_TOKEN` via `pii_core::env_var`, and the channel id from `PII_DISCORD_CHANNEL_ID`. Auth is the `Authorization: Bot <token>` header (never a URL path). `poll` issues `GET {base}/channels/{channel}/messages?after={cursor}&limit=20`; `send` issues `POST {base}/channels/{channel}/messages` with `{"content":…}`. A non-2xx or unparseable body is `PiiError::Unavailable` (the router retries next tick) and increments `ChannelInfo.errors`; it never kills the daemon.
- **F6.4** `CliChannel` (`src/cli.rs`): reads `Incoming` from newline-delimited JSON on a file path and appends replies to another — the channel used by tests and by `pii -p` style scripting, with no I/O beyond two files.
- **F6.5** Router (`pii-server/src/daemon/router.rs`): per channel, every `poll_interval_ms`, `poll(cursor)`; for each `Incoming`, start a run of the channel's graph with input `{"text":…, "chat":…, "channel":…, "user":…}`; **advance the cursor and record the run in the same store write transaction** (`Store::channel_advance(channel, cursor, run_id)`, new table `channel_cursor: &str -> &[u8]`), so a crash cannot duplicate or lose a message at the granularity of "run started"; emit `channel_message { channel, chat, run_id }`; subscribe to the run and, on `Terminal::Success { output }`, `send(chat, output.text)`; on `Terminal::Failure { error }`, `send(chat, "error: <error>")`. A send failure is logged and retried once, then dropped.
- **F6.6** `graphs/starter/assistant.toml` — `starter/assistant`: entry `agent` (kind `agent`, `memory = { enabled = true, top_k = 6 }`, `tools = ["fs_read"]`, `max_turns = 4`) → port `ok` → `reply` (kind `transform`, `expr` producing `{"text": …}`); port `err` → `fail` (kind `transform`); port `budget` → `fail`. Graph caps `["fs.read"]`. Must pass `pii graph lint`.
- **F6.7** `graphs/starter/consolidate.toml` — `starter/consolidate`: entry `tick` (kind `timer`, `cron = "*/15 * * * *"`) → port `fired` → `run` (kind `memory`, `op = "consolidate"`) → `done` (kind `transform`). Graph `limits.max_tokens` and `max_model_calls` are declared even though this phase makes no model calls, so the §5.4 budget wiring exists the day a generative step is added. Must pass `pii graph lint`.
- **F6.8** **Secrets never cross the wire** (extends Phase 2 N9): `PII_DISCORD_TOKEN` is read by the channel only. It must not appear in any `Event`, `run.trace`, `sys.status`, `channel.list` output, journal entry, or log line — the `Authorization` header is never logged, and any URL that would carry a credential is redacted to `<redacted>` before it is recorded. E20 greps for it.

#### W7 — CLI, gates, fixtures, scripts

- **F7.1** `pii-cli` subcommands, all dispatched on `argv[1]` before heavy init (preserving N6's `pii --version` budget) and all speaking the protocol: `pii net ls`, `pii worker ls|reload <name>`, `pii memory search|put|consolidate`, `pii approve ls|allow|deny`, `pii cron ls`, `pii soak`, `pii worker-exec`. `pii daemon` gains `--config`.
- **F7.2** `xtask` gains `net-check` (F2.13), `memory-eval` (F4.16) and `rss-gate` (F5.14); `cargo xtask ci` runs `deps-check`, `net-check`, `lint-files`, `proto-check`, `bench-gate`, fmt, clippy and tests. `rss-gate` and `memory-eval` are **not** in `ci` (they are minutes-long); they are separate exit criteria.
- **F7.3** `xtask deps-check` gains **invariant 5**: only `pii-store` may have a direct `redb` dependency. Violation → `DEPS-CHECK FAIL: redb outside pii-store — <crate>`. It gains a unit test `deps_check_rejects_redb_outside_store` feeding a synthetic package graph, matching the existing `deps_check_rejects_tui_graph_edge` pattern. The four existing invariants are unchanged and must still pass.
- **F7.4** Fixtures under `fixtures/phase3/`: `discord/routes.json` (Discord API fake: one `GET /channels/{id}/messages` batch with two messages, then empty batches; `POST /channels/{id}/messages` → `{"ok":true}`), `model/assistant.json` (scripted turns for `starter/assistant`), `model/atoms.json` (a turn emitting structured `atoms` for `memory` `extract`), `memory/corpus.jsonl`, `memory/queries.jsonl`, `soak.toml`, `workers.toml`, `ollama/chat.json` (fake Ollama response for E11).
- **F7.5** Scripts under `scripts/phase3/`, all writing only under `_tmp/phase3/` and `mv`-ing (never removing) a stale scratch dir: `discord-e2e.sh`, `worker-kill.sh`, `hot-reload.sh`, `shutdown.sh`, `secret-scan.sh`, `crash.sh`, `cron.sh`, and `README.md` documenting the live forms (`--secs 259200` soak; `PII_DISCORD_API_BASE=https://api.discord.com` once 3b lands; `PII_MODEL=ollama:<model>` against a local Ollama).

### Non-Functional

- **N1** Edition 2024; every new crate carries `[lints] workspace = true`. `unsafe_code = "forbid"`; `unwrap_used`/`expect_used`/`panic`/`indexing_slicing`/`todo`/`unimplemented` stay `deny` outside tests. Test modules keep the repo's existing `#![allow(...)]` header.
- **N2** No file exceeds **450 effective lines** (`cargo xtask lint-files`); split along the module boundaries named in W1–W7 rather than growing a file. `pii-store/src/lib.rs` is already at the 380-line warn threshold — put `MemStore`, the timer wheel, the approval table and the channel cursor in **new files** (`mem.rs`, `timer.rs`, `approval.rs`, `channel.rs`), not in `lib.rs`.
- **N3** **No new third-party crates.** This phase adds four workspace members and **zero** external packages. `Cargo.lock` must gain no `[[package]]` other than the new members. Asserted by E4. (The only new *features* enabled are `tokio`'s `signal` and `process` in `pii-server`/`pii-cli`, which pull no new packages.)
- **N4** Daemon RSS: **max** < 50 MB over the gate window with the configured listener, worker, channel and cron set active, and a steady-state slope < 2 MB across 3,000 runs (E21). The memory crate's streaming scan (F4.7) and redb-resident postings (F4.5) exist for this reason.
- **N5** The scheduler is **not a poll loop**: one `sleep_until`, rearmed on insert. A daemon whose earliest timer is 10 minutes out must record **zero** scheduler wakeups over a 2 s observation (asserted with `FixedClock` and `Scheduler::wakeups()`).
- **N6** `pii --version` stays p95 < 20 ms (`cargo xtask bench-gate`): the new subcommands must not construct an engine, open a store, or touch the network before `argv[1]` is matched.
- **N7** No panics on the daemon path. A dead worker, a channel returning HTML instead of JSON, a malformed HTTP response, a 10 MB stderr burst, an unparseable cron entry, or a memory store that fails to open must all surface as errors, never as an aborted process. Every parser added by this phase (HTTP, cron, worker frames, JSONL fixtures) is total.
- **N8** All artifacts, sockets, scratch stores, worker programs and logs go under `/projects/pi-improved/_tmp/phase3/`. Never `/tmp`, never `$HOME`.
- **N9** Secrets never cross the wire: `PII_DISCORD_TOKEN`, `OLLAMA_API_KEY` and any grant credential are read by the daemon only and must not appear in any `Event`, `run.trace`, `sys.status`, `channel.list`, journal record or log line (E20).
- **N10** Every environment read goes through `pii_core::env_var`, and the phase's complete env surface is enumerated in one doc comment in `pii-server/src/daemon/config.rs`: `PII_CONFIG`, `PII_RUNTIME_DIR`, `PII_MODEL`, `PII_MEMORY_PATH`, `PII_DISCORD_TOKEN`, `PII_DISCORD_CHANNEL_ID`, `PII_DISCORD_API_BASE`, `OLLAMA_HOST`, `OLLAMA_API_KEY`.
- **N11** Never hold a lock across `.await` (`clippy::await_holding_lock` is `deny`). The supervisor, connection registry and scheduler use `parking_lot` for short critical sections only; anything that awaits uses `tokio::sync`.
- **N12** Every decision function added by this phase is pure and table-tested with no I/O: `authorize`, `classify`, `HostPattern::matches`, `cron::next_after`, `bm25`, `rrf`, `decay` (with `FixedClock`), `quantise`, `parse_url`, HTTP response parsing, worker frame parsing.

---

## Architecture

```
                          ┌──────────────────────────────────────────────┐
                          │ pii-cli:  daemon | net | worker | memory     │
                          │           approve | cron | soak | tui | -p   │
                          └───────────────────────┬──────────────────────┘
                                                  │ ndjson JSON-RPC "pii.v2"
     ══════════════════════════════════════════════╪═══════════ pii-proto ONLY ═════
                                                  ▼
 ┌─────────────────────────────────────────────────────────────────────────────────┐
 │ pii-server :: Daemon                                                            │
 │  ┌────────────┐  ┌───────────────┐  ┌───────────────┐  ┌──────────────────────┐ │
 │  │ conn/      │  │ scheduler     │  │ approvals     │  │ router (channels)    │ │
 │  │ dispatch   │  │ ONE sleep_    │  │ gate → Suspend│  │ poll → run → reply   │ │
 │  │ sys/run/   │  │ until, rearmed│  │ store-backed  │  │ cursor+run in ONE tx │ │
 │  │ graph/…    │  │ NOT a poll    │  │ decision      │  │                      │ │
 │  └────────────┘  └───────┬───────┘  └───────┬───────┘  └──────────┬───────────┘ │
 │        nodes: agent · tool · worker · memory · gate · timer                     │
 └───────┬───────────────┬──────────────┬──────────────┬──────────────┬────────────┘
         │               │              │              │              │
   ┌─────▼─────┐  ┌──────▼──────┐ ┌─────▼──────┐ ┌─────▼──────┐ ┌─────▼─────────┐
   │ pii-graph │  │ pii-worker  │ │ pii-memory │ │ pii-channels│ │ pii-net       │
   │ scheduler │  │ Supervisor  │ │ BM25 +i8   │ │ discord     │ │ NetService    │
   │ +timer    │  │ kill→respawn│ │ +graph→RRF │ │ cli         │ │ authorize()   │
   │ +resume   │  │ drain→reload│ │ decay      │ │ (no sockets)│ │ resolve-once  │
   └─────┬─────┘  └──────┬──────┘ └─────┬──────┘ └─────┬───────┘ │ pin · classes │
         │               │              │              │         │ leases·budgets│
         │        ProcCap::spawn_child  │       HttpTransport ────► HTTP/1.1      │
         │          (stdio ndjson)      │        (pii-core)       └───────┬───────┘
         │               │              │                                 │
         └───────────────┴──────┬───────┴─────────────────────────────────┘
                                ▼                    ══════ THE ONE SOCKET ══════
                        pii-store (the only redb crate)          │
      run_journal · resume_index · timer_wheel · approval ·      ▼
      channel_cursor   ‖   memory.redb: mem_doc/term/vec/edge  TCP  (plaintext;
                                                                    TLS = 3b)

  cargo xtask net-check  ✗ sockets outside pii-net   ← THE GATE (E2/E3)
  cargo xtask deps-check ✗ redb outside pii-store    ← (E5/E6)
```

Worker death and reload:

```
invoke ──► Supervisor ──stdin──► child (pii worker-exec)
   ▲                                │ kill -9
   │                                ▼
   └── every in-flight id ← PiiError::Unavailable("WorkerDied: …")   is_retryable()==true
                                    │                                        │
                        worker_died ▼                                        ▼
                        respawn (<2 s, target <100 ms) ──► worker_restarted   scheduler retries
                                                                             ──► run completes

reload:  Draining ─► queue new invokes (bounded 256) ─► drain in-flight ─► shutdown
         ─► spawn from the replaced program path ─► hello ─► Ready ─► flush queue
         (no request is ever answered with an error caused by a reload)
```

Approval / cron suspension (one shape, §8.7 + §8.8):

```
gate|timer node ─► Outcome::Suspend{reason,key} ─► journal Suspended ─► resume_index
                                                                      │
   approve.resolve{id,decision}  ── or ──  timer_wheel fires (one tx) ─┘
                          │
                          ▼
     journal Resumed{key} ─► pii_graph::resume ─► gate reads the STORED decision ─► port
```

---

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `cron::next_after_table` | `*/15 * * * *`, `0 3 * * *`, `30 2 1,15 * *`, `0 0 * * 1-5` at fixed epochs | exact next-fire epochs (UTC), table-driven |
| `cron::rejects_invalid_expressions` | `"* * *"`, `"60 * * * *"`, `"*/0 * * * *"` | `PiiError::Invalid` for each; `pii graph lint` reports V0 |
| `sched::arms_once_not_polls` | one timer 10 minutes out, `FixedClock`, 2 s of observation | `Scheduler::wakeups() == 0`; after `advance(600_000)` it fires exactly once |
| `sched::fires_in_one_transaction` | timer due for 3 runs | each run's journal gains exactly one `Resumed`; no partial write on a simulated mid-batch failure |
| `approvals::gate_suspends_then_resumes_on_decision` | graph with a `gate`, then `approve.resolve{allow}` | `Suspended` journal entry + `approval_requested` event; after resolve, `Resumed` and terminal `Success` on the `allow` port |
| `approvals::survive_daemon_restart` | suspend, drop the `Engine`, rebuild it from the same store, resolve | run completes; decision read from the store, not from memory |
| `approvals::resolve_is_idempotent_and_typed` | resolve twice; resolve unknown id | second returns `resumed: false`; unknown returns `NotFound` |
| `channels::discord_polls_and_replies` | `FakeHttpServer` with two messages | two runs started; two `POST /channels/{id}/messages` recorded with the reply text; `channel_message` emitted per message |
| `channels::cursor_advances_exactly_once` | two updates, daemon dropped after the first, restarted | update 1 is not reprocessed, update 2 is processed; exactly two runs total |
| `channels::api_failure_does_not_kill_daemon` | fake returning 500, then HTML, then valid JSON | `ChannelInfo.errors == 2`; the third poll succeeds; daemon alive throughout |
| `channels::token_is_never_emitted` | full run with a token set | the token appears in no event, journal record, `sys.status` or `channel.list` payload; the `Authorization` header is never logged |
| `daemon::sigterm_exits_zero_within_two_seconds` | `SIGTERM` to a daemon with an active run | exit code 0 within 2 s; workers reaped; socket removed |
| `daemon::panic_hook_writes_crash_file` | a node that panics | `crashes/<ms>.json` exists with `location` and `message`; the daemon reports the run as failed |
| `daemon::status_reports_rss_and_subsystems` | live daemon | `rss_kb > 0`, `workers`/`listeners`/`channels`/`memory` populated, `protocol == "pii.v2"` |

---

## Exit Criteria

- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo build --workspace --all-targets`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && ./quality-gate.sh`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask lint-files`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && bash scripts/phase3/discord-e2e.sh`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-server --test channels 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-server --test approvals 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-server --test scheduler 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && bash scripts/phase3/cron.sh`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && bash scripts/phase3/shutdown.sh`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && bash scripts/phase3/crash.sh`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && bash scripts/phase3/secret-scan.sh`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && ./target/debug/pii graph lint graphs/starter/assistant.toml && ./target/debug/pii graph lint graphs/starter/consolidate.toml`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask rss-gate --secs 300 --interval 10 --max-mb 50 --runs 3000`

**Script contracts** (each script exits non-zero on any failed assertion, writes only under `_tmp/phase3/`, and `mv`s a stale scratch dir to `<name>.old-$(date +%s)` rather than deleting it):

- `discord-e2e.sh` — starts `FakeHttpServer` on an ephemeral loopback port from `fixtures/phase3/discord/routes.json`; starts the daemon with `PII_DISCORD_API_BASE=http://127.0.0.1:<port>`, `PII_DISCORD_TOKEN=TEST`, `PII_DISCORD_CHANNEL_ID=123456789`, `PII_MODEL=scripted:fixtures/phase3/model/assistant.json`, and a `[[channel]]` bound to `starter/assistant`; waits for the reply; asserts (a) the fake recorded a `POST /channels/123456789/messages` whose `content` matches the scripted answer, (b) `pii graph trace <run_id>` prints the full journal including the `agent` node and the terminal, (c) `run_id` came from a `channel_message` event. This is `001` exit criterion 2.
- `cron.sh` — starts the daemon with a `[[cron]]` firing every minute against a trivial graph, uses `PII_TEST_CLOCK_MS` to drive `FixedClock` forward two intervals (the daemon honours it only when set), and asserts exactly two runs exist with distinct run ids and both terminal `Success`.
- `shutdown.sh` — starts the daemon, starts a long run, sends `SIGTERM`, asserts exit code 0 within 2 s and that the control socket is gone.
- `crash.sh` — runs a graph containing a node that panics, asserts a `crashes/*.json` file appears with `location` and `message`, and that the daemon is still answering `sys.status`.
- `secret-scan.sh` — runs the Discord flow with `PII_DISCORD_TOKEN=SUPERSECRETTOKEN`, captures the full event ndjson, `run.trace`, `sys.status` and `channel.list`, and asserts `SUPERSECRETTOKEN` appears in **none** of them and that the `Authorization` header is never logged.

**Not exit criteria, documented in `scripts/phase3/README.md`** — the 72 h soak (`cargo xtask rss-gate --secs 259200 --interval 60`), the live Discord form (blocked on 3b's TLS), and the live model form (`PII_MODEL=ollama:<model>` against a local Ollama). Each has a place in the README to record the date, the command and the observed maximum.


---

## Guardrails

- Do NOT open a socket or resolve a hostname anywhere except `crates/pii-net/src/{dial,listen,resolve}.rs` — not in a test, not in a fixture helper, not in `xtask`. `pii_net::testing::FakeHttpServer` exists precisely so tests never need one. `cargo xtask net-check` is the gate and it must fail when violated.
- Do NOT add a third-party crate. This phase is deliberately dependency-free: no `axum`, `hyper`, `tower`, `reqwest`, `ureq`, `rustls`, `ring`, `hickory-resolver`, `tantivy`, `hnsw_rs`, `fastembed`, `ort`, `rmcp`. If a task looks like it needs one, the task is out of scope — re-read the cuts.
- Do NOT implement TLS, HTTPS, WebSocket, the gateway, a dashboard, MCP, `pii acp`, `pii logs`, `pii top`, `pii prof`, MQTT, BLE, serial, WASM plugin networking, or `pii-sync`. Their absence is not a gap; it is `specs/06-phase3b-gateway-mcp.md` and Phases 4–6.
- Do NOT let `pii-model`, `pii-channels` or `pii-memory` depend on `pii-net`. They take `Arc<dyn HttpTransport>` from `pii-core`. That is what keeps their tests offline and the DAG (§12.2) intact.
- Do NOT let any crate other than `pii-store` depend on `redb`. `MemStore` lives in `pii-store`, even though `pii-memory` is its only user.
- Do NOT let `pii-proto` gain a dependency, and do NOT let `pii-tui` gain one either — Phase 2's invariants (`deps-check` 1–4) must keep passing untouched.
- Do NOT weaken an existing Phase 1 or Phase 2 test to make a new one pass. The single authorised change is `hello_skew_is_hard_refusal`, which must now offer `"pii.v99"` and assert `expected == "pii.v2"`. Anything else that breaks means the new code is wrong.
- Do NOT modify the `wait` node. Timer suspension is the new `timer` kind. Phase 1 tests depend on `wait` blocking.
- Do NOT make the scheduler a poll loop, a thread-per-timer, or a 60-second tick. One `sleep_until`, rearmed on insert (§8.7).
- Do NOT let `consolidate` — or anything else in `pii-memory` — call a model. Zero model calls in that crate, no exceptions; the §5.4 incident is why.
- Do NOT tune the memory eval corpus to make the gate pass. The anti-rigging test (F4.17) is part of the deliverable, and `memory-eval` prints all four lanes so a rigged corpus is visible in the output.
- Do NOT answer a request with an error because a reload was in progress. Queue it. That is the entire content of exit criterion 4.
- Do NOT retry a dead worker forever: the restart window cap exists so a crash loop cannot become a spawn storm.
- Do NOT construct a `std::process::Command` outside `ProcCap::{run, spawn_child}`, do NOT call `std::fs` from a tool or node (use `Ctx::fs()`), and do NOT call `SystemTime::now` outside `SystemClock`.
- Do NOT log, journal, emit or return a token, API key, or grant credential. The `Authorization` header is never logged; any URL that would carry a credential is redacted to `<redacted>` before it is recorded.
- Do NOT write outside `/projects/pi-improved/_tmp/phase3/` for any artifact, socket, scratch store, worker program or log. Never `/tmp`, never `$HOME`.
- Do NOT delete or `rm -rf` anything that existed before the run. Scripts `mv` a stale scratch dir to `<name>.old-$(date +%s)`.
- Do NOT modify `_repos/`, `_docs/`, `AGENTS.md`, `HANDOFF.md`, `.pidag/`, or any file under `specs/`.
- Do NOT run `git commit`, `git add`, `git push`, or create branches. Files only.
- Do NOT exceed 450 effective lines in any file, and do NOT silence a workspace lint with a blanket `#![allow]`. Per-item `#[allow]` with a `// reason:` comment only, and only where this spec names one: `SystemClock`, `ProcCap::spawn_child`, `pii-net`'s three socket files, and `pii-server::uds` / `pii-cli::cmd_daemon` (pre-existing).

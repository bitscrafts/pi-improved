I read the contract sections (`001` Phase-3 scope + exit list, `002` §1.3/§1.4/§1.6, §2.7, §4, §5, §6.2, §8.7, §8.8, §12.2), the current tree, and verified the environment facts this spec depends on. Here is the spec. — Part 3/4

**Verified premises** (checked in this container, 2026-09-03, do not re-derive):

- Workspace members today: `pii-abi, pii-core, pii-store, pii-graph, pii-proto, pii-model, pii-tools, pii-server, pii-tui, pii-cli, xtask`.
- **`Cargo.lock` contains no HTTP client and no TLS stack** — no `ureq`, `rustls`, `ring`, `hyper`, `reqwest`, `axum`, `tantivy`, `hnsw_rs`, `fastembed`. `pii-model` has exactly five deps (`pii-abi`, `pii-core`, `serde`, `serde_json`, `thiserror`) and `provider_from_env("ollama:…")` returns `Invalid` — the HTTP provider was never landed.
- `the workspace memory-core` is **not** vendored: `_repos/` holds only `grok-build, hermes-agent, pi_agent_rust, yantrikclaw, zeroclaw`.
- `crates.io` is reachable (`index.crates.io` 200, `static.crates.io` 200 for a real `.crate`), and `api.telegram.org` answers — but **no exit criterion in this spec uses the network**.
- `PiiError::is_retryable()` is true for `Timeout` only, and `Ctx::deadline` is inert (HANDOFF caveat 12), so the scheduler's retry path in `sched/dispatch.rs:246` currently has **no live producer**. This spec gives it one.
- `pii-graph`'s registry has four built-in kinds (`transform, route, emit, wait`); `pii-server` registers `agent` and `tool` through `NodeRegistry::register`.
- `pii-store` owns the only `redb` dependency; tables are `meta, graph_body, run, run_journal, run_snapshot, run_by_status, resume_index`.
- `clippy.toml` already bans `tokio::net::{TcpListener::bind, TcpStream::connect, UnixListener::bind}`, `std::process::Command::{spawn,output,status}`, `std::time::SystemTime::now`, `std::env::var`. `std::net::*` is **not** yet banned.
- `cargo xtask` has `deps-check, lint-files, proto-gen, proto-check, bench-gate, ci`. `lint-files` hard-fails at 450 effective lines, warns at 380.

---

# Spec: Phase 3 - Assistant / daemon, networking, workers, memory


**Parent-Spec**: `05-phase3-assistant-gateway.md`
**Part**: 3 of 4
**Covers**: E5, E6, E17-E21 (pii-memory: BM25 + i8 vectors + graph lane + RRF, persistence, consolidation, the recall gate)
**Depends-On-Parts**: part 1 (`Clock`, `HttpTransport`, proto v2 memory types)

## Overview

**Project**: <repo>
**Phase**: 3 - Assistant / gateway (daemon, networking, workers, layered memory)

**Spec of record**: `_docs/06-research/001-pii-architecture-plan.md` §"Phase 3 — Assistant / gateway" (scope + the five exit criteria); `_docs/06-research/002-pii-architecture-plan-pass2.md` §4.1–§4.5 (`pii-net`, egress discipline), §5.1–§5.5 (`pii-memory`, L0–L3, RRF, embedder constraint), §6.2 (pidag subsumed — `after` edges, budget in observable units, delta verification), §8.7 (cron = graphs + timer wheel), §8.8 (approvals), §2.7 (daemon debuggability), §12.2 (dependency DAG), §1.3/§1.4/§1.6 (lints, 450-line gate, CI), §14 Phase 3.

**The load-bearing requirement.** `cargo xtask net-check` must pass: **`pii-net` is the only place in the workspace that opens a socket or resolves a name.** Every byte that leaves or enters this process passes `NetService::authorize()` — which resolves once, pins the `SocketAddr` set, validates address class, refuses mixed answers, and charges the grant (§4.1, §4.3). Phase 2's load-bearing invariant was a dependency edge that must not exist; Phase 3's is a call site that may exist in exactly one crate. Everything else in this phase — channels, workers, memory, cron, approvals — is subordinate to keeping that chokepoint honest, because it is the only thing that makes "the daemon owns sockets" a fact rather than a diagram.

The five phase exit criteria from `001` are behavioural and are all delivered (E21–E24, E12–E17); the chokepoint is the structural invariant that keeps them from being satisfied by a shortcut.

**Scope cuts — stated explicitly, as instructed.**

1. **The gateway is CUT** → `specs/06-phase3b-gateway-mcp.md`. No `axum`, no REST, no WebSocket, no dashboard, no SSE. Reason: none of the five load-bearing exit criteria touch it, the control protocol (`pii-proto`) already serves every client Pii has, and `axum + hyper + tower + http` is the largest new third-party subtree in the whole plan. `pii-obs` stage 2, `pii logs -f`, `pii top`, `pii prof` and pass-2 §14's exit 8 move with it, as does `pii-mcp` (`rmcp` needs HTTP+SSE and a quarantined `reqwest`).
2. **TLS is CUT** → 3b, and with it `https://` egress, remote transport, `pii pair`, bearer tokens, and talking to the real `api.telegram.org`. `pii-net` speaks plaintext HTTP/1.1 over TCP. Reason (verified above): the workspace contains **no** TLS stack today, so TLS is a new third-party subtree (`rustls` + `ring`/`aws-lc-rs`, i.e. a C toolchain) that fights pass-1 §0's "no C deps in the default build" and that **no container-local exit criterion can exercise**. Phase 3 therefore ships the whole egress *policy* (grants, resolve-once-pin, class validation, deadlines, byte and connection budgets) and the plaintext transport; 3b adds a `TlsConnector` behind the unchanged `NetService::connect`. Honest consequence: **E15 runs Telegram against a Bot-API-compatible fake on loopback**, and the live form (`PII_TELEGRAM_API_BASE=https://api.telegram.org`) is documented in `scripts/phase3/README.md` as blocked on 3b. The `ollama:` provider, by contrast, becomes fully real — local Ollama is plaintext `http://127.0.0.1:11434`.
3. **`hickory-resolver` is CUT.** Resolution is `std::net::ToSocketAddrs` behind a `Resolver` trait, called **exactly once** per authorize. §4.3.2's actual requirement is resolve-once-then-pin, not a particular resolver; the trait keeps hickory a drop-in, and a `StaticResolver` makes the entire egress policy testable with no DNS at all.
4. **`the workspace memory-core` is CUT as a dependency; `pii-memory` is first-party.** §5.1 says "embed, do not reimplement" — unexecutable here: the crate is not vendored and not published to this machine. Consequence: no `tantivy`, no `hnsw_rs`, no `fastembed`/ONNX. `pii-memory` implements pass-1 §6.3 directly: hand-rolled BM25 over redb postings, i8-quantised vectors with a streaming brute-force scan, a co-occurrence graph lane, RRF k=60, temporal decay. §5.5 itself says brute force wins below ~10k docs; HNSW is deferred until a measured corpus exceeds that.
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

#### W4 — `pii-memory`: BM25 + i8 vectors + graph, fused by RRF

- **F4.1** New crate `crates/pii-memory`. Dependencies: `pii-abi`, `pii-core`, `pii-store`, `serde`, `serde_json`, `thiserror`, `parking_lot`. No third-party search, vector or embedding crate (cut 4).
- **F4.2** `pii-store` gains `src/mem.rs`: `pub struct MemStore` opened on a **second redb file** (`memory.redb`), keeping the vector index isolated from the run journal (§5.1: redb's lock is per file). Tables: `mem_doc: &str -> &[u8]`, `mem_term: (&str, &str) -> u32` (`(term, doc_id) -> tf`), `mem_df: &str -> u64`, `mem_vec: &str -> &[u8]` (i8 vector + f32 scale), `mem_edge: (&str, &str) -> u64` (concept co-occurrence counts), `mem_meta: &str -> u64` (`n_docs`, `sum_dl`, `dims`), `mem_archive: &str -> &[u8]`. **redb stays confined to `pii-store`** — `xtask deps-check` gains invariant 5 (F7.3).
- **F4.3** `src/doc.rs`: `Doc { id: String, layer: Layer, kind: DocKind, text: String, concepts: Vec<String>, importance: f32, created_ms: u64, last_used_ms: u64, evidence: Option<(String, u64)>, archived: bool }`; `Layer ∈ L1|L2|L3`; `DocKind ∈ Atom|Scene|Persona|Skill`. Ids are `blake3(text)[..16]` hex, so the same fact put twice is the same doc (idempotent put).
- **F4.4** `src/tokenize.rs`: `tokenize(&str) -> Vec<String>` — lowercase, split on `!char::is_alphanumeric()`, drop tokens shorter than 2 chars, drop a fixed 40-word stoplist declared in the file. No stemming: morphology is the vector lane's job, and that is precisely why the hybrid can beat BM25 (F4.16).
- **F4.5** `src/bm25.rs`: standard BM25, `k1 = 1.2`, `b = 0.75`, `idf = ln(1 + (N - df + 0.5) / (df + 0.5))`, document length from `mem_meta`. `bm25_search(&self, q: &str, k: usize) -> Vec<(String, f32)>` reads postings from `mem_term` — **no in-RAM inverted index**; the RSS budget is why.
- **F4.6** `src/embed.rs`: `pub trait Embedder: Send + Sync { fn dims(&self) -> usize; fn name(&self) -> &str; fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, PiiError>; }`.
  - `TrigramEmbedder` (**the default, deterministic, offline**): dims 256; lowercase, pad the text with `_`, take character 3-grams, hash each with FNV-1a into `dims` buckets, weight `sqrt(tf)`, L2-normalise. It captures morphology and typos that exact-token BM25 misses, which is a real retrieval gain and not a rigged one.
  - `OllamaEmbedder` — `POST {host}/api/embeddings` through `Arc<dyn HttpTransport>` (no socket code in `pii-memory`).
  - `NullEmbedder` — `dims() == 0`, used to prove the degraded path.
  - Quantisation (`src/quant.rs`): `q[i] = clamp(round(v[i] * 127), -127, 127)` stored with an f32 scale; similarity is an `i32` dot product converted to f32. Round-trip error must stay under 1 % on unit vectors (a test asserts it).
- **F4.7** Vector lane (`src/vector.rs`): a brute-force scan **streaming from `mem_vec` in batches of 512 docs**, holding at most one batch plus a top-k heap in memory, with an LRU cache bounded by `cache_bytes` (default 8 MiB, configurable). No full-corpus resident array — this is the requirement that keeps E21's 50 MB honest at 5,000 docs.
- **F4.8** Graph lane (`src/graph_lane.rs`): `mem_edge` counts concept co-occurrence within a doc. A query's concepts (its tokens that exist as concepts) expand to the top 8 co-occurring concepts, and documents carrying them are scored by summed edge weight. Communities (L2) are computed by **label propagation** over `mem_edge` (deterministic: iterate concepts in sorted order, 10 rounds, ties broken by lexicographic order), and a scene doc is written per community with `text` = the top three concepts joined by `", "`. No model call anywhere in this file.
- **F4.9** Fusion (`src/search.rs`): RRF with `k = 60` over the three lanes (`score += 1 / (60 + rank)`), then temporal decay `score *= exp(-λ * age_days)` with `λ`: atom `0.02`, scene `0.005`, skill `0.001`, persona `0.0` (never decays). `age_days` uses `Ctx::now_ms()`, so decay is table-testable with `FixedClock`. Returns `MemoryHit { doc, score, lanes }` where `lanes` names which lanes contributed — required by E17's per-lane report.
- **F4.10** **Degrade honestly** (§5.3): if the embedder errors or `dims() == 0`, search runs the two remaining lanes, sets `degraded: true` with `lanes: ["bm25","graph"]` in `MemorySearchResult` and `sys.status.memory`, and emits `memory_degraded { lanes, reason }` **once per transition** (not per query). It must never silently return worse results.
- **F4.11** **Vectors are persisted, not memory-resident** — the specific `the workspace memory-core` bug §5.3 names: every `put` writes `mem_vec` inside the same write transaction as `mem_doc`, and every search reads from redb. E18 writes 1,000 docs, reopens the store in a new process, and asserts recall@10 for the vector lane is **identical**.
- **F4.12** `src/consolidate.rs`: `consolidate(&self, ctx: &Ctx, cfg: &ConsolidateCfg) -> Result<ConsolidateReport, PiiError>` — decay refresh, near-duplicate merge, archive. Candidate generation uses the constants ported from the §5.4 incident: `MIN_SHARED_CONCEPTS = 2`, `MIN_CONCEPT_JACCARD = 0.15`, `NEGATION_WINDOW = 8` (a negation token must occur within 8 tokens of a shared concept), and a **hard cap `MAX_CANDIDATES = 200`**: on reaching it, generation stops and `candidates_capped: true` is reported. **Zero model calls.** Merging keeps the higher-importance doc and unions the concepts; the loser is **archived, never deleted** (§5.4: forgetting is the compression step, and archives are recoverable).
- **F4.13** `memory` node kind in `pii-server` (`src/nodes/memory.rs`): cfg `{ op: "search"|"put"|"extract"|"consolidate", … }`. `extract` takes `$.payload.atoms` — an array of `{text, concepts, importance}` produced upstream by an `agent` node with a structured-output prompt — and writes each with `evidence = (run_id, seq)` (L1 built from L0, §5.2). Ports `ok`, `err`. Required caps: `CapKind::MEMORY_WRITE` for `put`/`extract`/`consolidate`, none for `search`.
- **F4.14** `agent` node cfg gains `memory = { enabled: bool (default false), top_k: u32 (default 6), include_persona: bool (default true) }`. When enabled, **before the first provider call** the node retrieves and injects exactly one extra system message: the persona document (if any), then the top-k hits with their doc ids, in RRF order, truncated deterministically at **6,800 characters** (≈1,700 tokens, §5.2's budget; Context Rot is the reason the budget is small). Retrieval happens in prompt assembly, **not as a separate graph node** (§5.2) — a `memory` node exists only for deliberate, author-placed operations.
- **F4.15** RPCs `memory.search`, `memory.put`, `memory.consolidate`; CLI `pii memory search <q> [--top-k N]`, `pii memory put --layer L --text T`, `pii memory consolidate`. All through the protocol.
- **F4.16** `xtask memory-eval --corpus <f> --queries <f> [--top-k 10]`: builds a fresh `MemStore` under `_tmp/phase3/memeval/`, indexes the corpus, and prints exactly four lines — `lane=bm25 recall@10=<x.xxx>`, `lane=vector recall@10=<x.xxx>`, `lane=graph recall@10=<x.xxx>`, `lane=hybrid recall@10=<x.xxx>`. Exit 0 **iff** `hybrid > bm25` strictly **and** `hybrid >= 0.75`; otherwise print `MEMORY-EVAL FAIL: …` and exit 1. Recall@10 is `mean over queries of |relevant ∩ top10| / |relevant|`.
- **F4.17** Fixtures `fixtures/phase3/memory/corpus.jsonl` (**≥ 200 documents** of real domain prose — notes about this project's own subsystems, operations, and preferences; one JSON object per line `{id, text, concepts}`) and `queries.jsonl` (**exactly 20** `{q, relevant: [ids]}`, 1–3 relevant docs each). **Anti-rigging constraints, all machine-checked by a test in `xtask`**: no query string may appear verbatim as a substring of any relevant document; every query must have at least one relevant document that BM25-alone ranks outside the top 10; `bm25` recall@10 on this set must be **≤ 0.65** (printed, so a corpus tuned to make BM25 look bad is visible); and a unit test must assert that with the vector and graph lanes disabled the hybrid ranking is **identical** to BM25 — proving the fusion is doing work rather than relabelling one lane.

### Non-Functional

- **N1** Edition 2024; every new crate carries `[lints] workspace = true`. `unsafe_code = "forbid"`; `unwrap_used`/`expect_used`/`panic`/`indexing_slicing`/`todo`/`unimplemented` stay `deny` outside tests. Test modules keep the repo's existing `#![allow(...)]` header.
- **N2** No file exceeds **450 effective lines** (`cargo xtask lint-files`); split along the module boundaries named in W1–W7 rather than growing a file. `pii-store/src/lib.rs` is already at the 380-line warn threshold — put `MemStore`, the timer wheel, the approval table and the channel cursor in **new files** (`mem.rs`, `timer.rs`, `approval.rs`, `channel.rs`), not in `lib.rs`.
- **N3** **No new third-party crates.** This phase adds four workspace members and **zero** external packages. `Cargo.lock` must gain no `[[package]]` other than the new members. Asserted by E4. (The only new *features* enabled are `tokio`'s `signal` and `process` in `pii-server`/`pii-cli`, which pull no new packages.)
- **N4** Daemon RSS: **max** < 50 MB over the gate window with the configured listener, worker, channel and cron set active, and a steady-state slope < 2 MB across 3,000 runs (E21). The memory crate's streaming scan (F4.7) and redb-resident postings (F4.5) exist for this reason.
- **N5** The scheduler is **not a poll loop**: one `sleep_until`, rearmed on insert. A daemon whose earliest timer is 10 minutes out must record **zero** scheduler wakeups over a 2 s observation (asserted with `FixedClock` and `Scheduler::wakeups()`).
- **N6** `pii --version` stays p95 < 20 ms (`cargo xtask bench-gate`): the new subcommands must not construct an engine, open a store, or touch the network before `argv[1]` is matched.
- **N7** No panics on the daemon path. A dead worker, a channel returning HTML instead of JSON, a malformed HTTP response, a 10 MB stderr burst, an unparseable cron entry, or a memory store that fails to open must all surface as errors, never as an aborted process. Every parser added by this phase (HTTP, cron, worker frames, JSONL fixtures) is total.
- **N8** All artifacts, sockets, scratch stores, worker programs and logs go under `_tmp/phase3/`. Never `/tmp`, never `$HOME`.
- **N9** Secrets never cross the wire: `PII_TELEGRAM_TOKEN`, `OLLAMA_API_KEY` and any grant credential are read by the daemon only and must not appear in any `Event`, `run.trace`, `sys.status`, `channel.list`, journal record or log line (E20).
- **N10** Every environment read goes through `pii_core::env_var`, and the phase's complete env surface is enumerated in one doc comment in `pii-server/src/daemon/config.rs`: `PII_CONFIG`, `PII_RUNTIME_DIR`, `PII_MODEL`, `PII_MEMORY_PATH`, `PII_TELEGRAM_TOKEN`, `PII_TELEGRAM_API_BASE`, `OLLAMA_HOST`, `OLLAMA_API_KEY`.
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
   │ scheduler │  │ Supervisor  │ │ BM25 +i8   │ │ telegram    │ │ NetService    │
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
| `xtask::deps_check_rejects_redb_outside_store` | synthetic graph with `pii-memory → redb` | violation naming invariant 5 |
| `memory::bm25_matches_reference_scores` | 5-doc corpus, hand-computed BM25 | scores equal to 3 decimals |
| `memory::quantise_roundtrip_under_one_percent` | 100 random unit vectors | max cosine error < 0.01 |
| `memory::vector_lane_survives_restart` | put 1,000 docs, drop the store, reopen in a new `MemStore` | vector-lane recall@10 identical before and after; `mem_vec` non-empty on disk |
| `memory::rrf_equals_bm25_when_lanes_disabled` | hybrid search with vector+graph lanes off | ranking identical to `bm25_search` — the fusion is not relabelling |
| `memory::degrades_honestly_without_embedder` | `NullEmbedder` | `MemorySearchResult.degraded == true`, `lanes == ["bm25","graph"]`, exactly one `memory_degraded` event for N queries |
| `memory::decay_orders_by_age_with_fixed_clock` | two equal-score atoms 1 day and 90 days old | the younger ranks first; persona docs are unaffected by any age |
| `memory::consolidate_cap_is_hard` | a corpus generating > 200 candidate pairs | `candidates_capped == true`, exactly 200 pairs evaluated, **zero** model calls, nothing deleted (losers are `archived`) |
| `memory::merge_requires_two_shared_concepts` | pair sharing one concept; pair sharing two with Jaccard 0.2 | first not merged, second merged |
| `memory::agent_injects_capped_context` | agent with `memory.enabled`, 50 matching docs | exactly one extra system message, ≤ 6,800 chars, persona first, doc ids present |
| `memory::eval_fixture_is_not_rigged` | `corpus.jsonl` + `queries.jsonl` | ≥ 200 docs; exactly 20 queries; no query is a substring of a relevant doc; every query has ≥ 1 relevant doc outside BM25's top 10 |

---

## Exit Criteria

- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo build --workspace --all-targets`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && ./quality-gate.sh`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask lint-files`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask deps-check`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p xtask deps_check_rejects_redb_outside_store -- --exact 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask memory-eval --corpus fixtures/phase3/memory/corpus.jsonl --queries fixtures/phase3/memory/queries.jsonl`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-memory --test persistence 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-memory --test lanes 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-memory --test consolidate 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p xtask memory_eval_fixture_is_not_rigged -- --exact 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`

**Not exit criteria, documented in `scripts/phase3/README.md`** — the 72 h soak (`cargo xtask rss-gate --secs 259200 --interval 60`), the live Telegram form (blocked on 3b's TLS), and the live model form (`PII_MODEL=ollama:<model>` against a local Ollama). Each has a place in the README to record the date, the command and the observed maximum.


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
- Do NOT log, journal, emit or return a token, API key, or grant credential. Redact `/bot<token>/` to `/bot<redacted>/` before any URL is recorded.
- Do NOT write outside `_tmp/phase3/` for any artifact, socket, scratch store, worker program or log. Never `/tmp`, never `$HOME`.
- Do NOT delete or `rm -rf` anything that existed before the run. Scripts `mv` a stale scratch dir to `<name>.old-$(date +%s)`.
- Do NOT modify `_repos/`, `_docs/`, `AGENTS.md`, `HANDOFF.md`, `.pidag/`, or any file under `specs/`.
- Do NOT run `git commit`, `git add`, `git push`, or create branches. Files only.
- Do NOT exceed 450 effective lines in any file, and do NOT silence a workspace lint with a blanket `#![allow]`. Per-item `#[allow]` with a `// reason:` comment only, and only where this spec names one: `SystemClock`, `ProcCap::spawn_child`, `pii-net`'s three socket files, and `pii-server::uds` / `pii-cli::cmd_daemon` (pre-existing).

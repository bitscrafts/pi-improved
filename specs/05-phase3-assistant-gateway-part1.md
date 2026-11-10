I read the contract sections (`001` Phase-3 scope + exit list, `002` §1.3/§1.4/§1.6, §2.7, §4, §5, §6.2, §8.7, §8.8, §12.2), the current tree, and verified the environment facts this spec depends on. Here is the spec. — Part 1/4

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
**Part**: 1 of 4
**Covers**: E1-E12, E31 (net-check, egress policy, HTTP/1.1, proto v2, Ollama over the transport)
**Depends-On-Parts**: none — this part is self-contained

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

#### W1 — foundations: `pii-abi`, `pii-core`, `pii-proto` v2

- **F1.1** `pii-abi` gains `src/http.rs`: `pub enum HttpMethod { Get, Post, Put, Delete }` (serde `SCREAMING_SNAKE`), `pub struct HttpRequest { pub method: HttpMethod, pub url: String, pub headers: Vec<(String, String)>, pub body: Vec<u8>, pub timeout_ms: u64 }`, `pub struct HttpResponse { pub status: u16, pub headers: Vec<(String, String)>, pub body: Vec<u8> }` with `fn header(&self, name: &str) -> Option<&str>` (ASCII-case-insensitive) and `fn is_success(&self) -> bool` (200..=299). Pure data, no sockets — these types are part of the stable ABI so a worker or WASM guest can name them (§12.2 keeps `pii-abi` at `serde, thiserror, schemars, bitflags`).
- **F1.2** `pii-abi` gains `PiiError::Unavailable(String)` — "a transient dependency failure; the operation may succeed on retry". `is_retryable()` returns **true** for it; `is_terminal()` returns false. This is the fix for the dead retry path: today only `Timeout` is retryable and nothing constructs it, so `sched/dispatch.rs:246` can never fire. Worker death (F3.4) and HTTP 429/5xx (F2.12) are its producers. Wire mapping: `ErrorCode::Unavailable = 108`.
- **F1.3** `pii-core` gains `src/clock.rs`: `pub trait Clock: Send + Sync { fn now_ms(&self) -> u64; }`, `pub struct SystemClock;` (**the only `SystemTime::now` call site in the workspace** — one `#[allow(clippy::disallowed_methods)]` with a `// reason:` comment), `pub struct FixedClock` (`AtomicU64` + `pub fn advance(&self, ms: u64)`). `Ctx` gains a `clock: Arc<dyn Clock>` field, `pub fn now_ms(&self) -> u64`, and `pub fn with_clock(self, clock: Arc<dyn Clock>) -> Self`; `Ctx::root` defaults to `SystemClock`; `child`/`child_with_token` inherit the parent's clock. Every wall-clock read added by this phase (timer wheel, cron, grant expiry, memory decay, channel cursors) goes through it, so all of them are table-testable.
- **F1.4** `pii-core` gains `src/net.rs`: `pub trait HttpTransport: Send + Sync { fn request(&self, ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError>; }` (sync — `GraphNode::run` is sync and runs on `spawn_blocking`, matching `ModelProvider`). The trait lives here, not in `pii-net`, so `pii-model` and `pii-channels` can consume HTTP **without** a dependency on the socket crate (§12.2 keeps them off `pii-net`), and so their tests need no network.
- **F1.5** `pii-core::ProcCap` gains `pub fn spawn_child(&self, argv: &[String], cwd: &Path, env: &[(String, String)]) -> Result<ChildProc, PiiError>` — the **second and last** `std::process::Command` construction site in the workspace (one `#[allow(clippy::disallowed_methods)]` with a `// reason:`). It applies the same `CapSet::proc_argv0` check as `run`, requires `CapKind::SPAWN`, pipes stdin/stdout/stderr, and returns `ChildProc { pid: u32, stdin: ChildStdin, stdout: ChildStdout, stderr: ChildStderr }` with `fn kill(&mut self) -> Result<(), PiiError>`, `fn try_wait(&mut self) -> Result<Option<i32>, PiiError>`, `fn wait_timeout(&mut self, d: Duration) -> Result<Option<i32>, PiiError>`.
- **F1.6** Deadlines become live (closes HANDOFF caveat 12): `Ctx::with_deadline(self, ms_from_now: u64) -> Self` sets an absolute deadline read from `Ctx::now_ms()`; `Ctx::check()` returns `PiiError::Timeout` once past it; the run context built by `pii-server` derives its deadline from `GraphLimits::max_wall_ms` when that is non-zero. A node that overruns now produces a *retryable* failure instead of running forever.
- **F1.7** `pii-proto`: `pub const PROTOCOL_VERSION: &str = "pii.v2";` and `ErrorCode` gains `Unavailable = 108`, `WorkerDied = 109`. `Method` gains exactly: `approve.list`, `approve.resolve`, `net.listeners`, `net.grants`, `worker.list`, `worker.reload`, `memory.search`, `memory.put`, `memory.consolidate`, `channel.list`, `cron.list`, `sys.reload`. The enum stays closed; unknown strings stay `MethodNotFound`.
- **F1.8** `pii-proto` new param/result modules (one file each, ≤450 effective lines):
  - `src/approve.rs`: `ApprovalInfo { approval_id, run_id, node, policy, prompt, requested_at_ms }`, `ApproveListResult { pending: Vec<ApprovalInfo> }`, `ApproveResolveParams { approval_id: String, decision: Decision, note: Option<String> }` where `Decision ∈ allow|deny`, `ApproveResolveResult { run_id: String, resumed: bool }`.
  - `src/net.rs`: `ListenerInfo { id, kind, addr, proto, conns, since_ms }`, `GrantInfo { principal, key, connect: Vec<String>, allow_private: bool, max_conns: u32, max_bytes_out: u64, max_bytes_in: u64, expires_at_ms: Option<u64>, bytes_out: u64, bytes_in: u64 }`, `NetListenersResult { listeners: Vec<ListenerInfo> }`, `NetGrantsResult { grants: Vec<GrantInfo> }`.
  - `src/worker.rs`: `WorkerInfo { name, pid: Option<u32>, state: WorkerState, restarts: u32, in_flight: u32, tag: Option<String> }`, `WorkerState ∈ starting|ready|draining|dead|failed`, `WorkerListResult { workers: Vec<WorkerInfo> }`, `WorkerReloadParams { name: String }`, `WorkerReloadResult { name: String, old_pid: Option<u32>, new_pid: u32, drained: u32 }`.
  - `src/memory.rs`: `MemoryDoc { id, layer, kind, text, concepts: Vec<String>, importance: f32, created_ms, evidence: Option<Evidence> }`, `Evidence { run_id: String, seq: u64 }`, `MemorySearchParams { query: String, top_k: u32, layers: Vec<String> }`, `MemorySearchResult { hits: Vec<MemoryHit>, lanes: Vec<String>, degraded: bool }`, `MemoryHit { doc: MemoryDoc, score: f32, lanes: Vec<String> }`, `MemoryPutParams { text, layer, concepts, importance }` → `MemoryPutResult { id }`, `MemoryConsolidateResult { scanned: u32, merged: u32, archived: u32, candidates_capped: bool }`.
  - `src/channel.rs`: `ChannelInfo { name, kind, graph, cursor: Option<u64>, last_ok_ms: Option<u64>, errors: u32 }`, `ChannelListResult { channels: Vec<ChannelInfo> }`, `CronInfo { name, graph, schedule, next_fire_ms, last_run: Option<String> }`, `CronListResult { crons: Vec<CronInfo> }`.
- **F1.9** `pii-proto::Event` gains exactly these variants (closed, `#[serde(tag = "kind", rename_all = "snake_case")]`): `run_suspended { run_id, key, reason }`, `run_resumed { run_id, key }`, `approval_requested { run_id, node, approval_id, policy, prompt }`, `approval_resolved { approval_id, decision, note }`, `worker_died { worker, pid, code: Option<i32>, in_flight: u32 }`, `worker_restarted { worker, pid, restarts, took_ms }`, `channel_message { channel, chat, run_id }`, `net_denied { target, reason }`, `memory_degraded { lanes: Vec<String>, reason: String }`. `reason` in `run_suspended` is a string discriminant (`approval|timer|event|human_input`) — the wire vocabulary must not re-export `SuspendReason` from `pii-abi` (F1.1 of Phase 2: the wire contract does not inherit the engine's tree).
- **F1.10** `SysStatus` result is extended (same struct, new fields): `rss_kb: u64`, `workers: Vec<WorkerInfo>`, `listeners: Vec<ListenerInfo>`, `channels: Vec<ChannelInfo>`, `memory: MemoryStatus { docs: u64, lanes: Vec<String>, degraded: bool, backend: String }`, `timers_pending: u32`, `protocol: String`. `rss_kb` is read from `/proc/self/status` `VmRSS` (Linux; `0` elsewhere).
- **F1.11** `cargo xtask proto-gen` regenerates `crates/pii-proto/schema/*.json` including the new modules; `cargo xtask proto-check` must pass. Schemas are committed.

#### W2 — `pii-net`: the one socket

- **F2.1** New crate `crates/pii-net`. Dependencies, exactly: `pii-abi`, `pii-core`, `serde`, `serde_json`, `thiserror`, `parking_lot`. **No `tokio`** — `NetService` is synchronous and thread-based, matching `GraphNode::run`, `ModelProvider::chat` and `HttpTransport` (all sync). Features: `default = []`, `testing = []` (F2.10).
- **F2.2** `src/grant.rs`: `pub struct NetGrant { pub connect: HostScope, pub allow_private: bool, pub max_conns: u32, pub max_bytes_out: u64, pub max_bytes_in: u64, pub expires_at_ms: Option<u64> }` — **no `Default` impl**; a zero value is a downgrade (§4.3). `pub struct HostScope(Vec<HostPattern>)` with `HostPattern::parse("host:port" | "*.suffix:port" | "host:*")`; `*.example.com` matches `a.example.com` and `a.b.example.com` but **not** `example.com`. `pub trait GrantStore: Send + Sync { fn grant(&self, principal: &str, key: &str) -> Option<NetGrant>; fn all(&self) -> Vec<(String, String, NetGrant)>; }` plus `StaticGrants` (a `BTreeMap`). The store-backed impl lives in `pii-server`, so `pii-net` needs no `pii-store` dependency.
- **F2.3** `src/authorize.rs` — the pure decision function, the heart of the phase:
  `pub fn authorize(target: &Target, resolved: &[IpAddr], grant: &NetGrant, now_ms: u64) -> Result<Pinned, NetDenied>` where `Target { host: String, port: u16 }` and `Pinned { addrs: Vec<SocketAddr>, deadline_ms: u64 }`. Rules, in this order:
  1. `grant.expires_at_ms` in the past → `NetDenied::Expired`.
  2. `host:port` not matched by `grant.connect` → `NetDenied::NotAllowed`.
  3. `resolved` empty → `NetDenied::NoAddress`.
  4. Classify every address: `pub enum AddrClass { Public, Loopback, Private, LinkLocal, CloudMetadata, Multicast, Unspecified, Nat64MappedPrivate }`. `CloudMetadata` = `169.254.169.254` and `fd00:ec2::254`. `Nat64MappedPrivate` = `64:ff9b::/96` whose embedded IPv4 is private/loopback/link-local. IPv4-mapped IPv6 (`::ffff:a.b.c.d`) is classified by its embedded IPv4.
  5. **Mixed answers are always refused** → `NetDenied::MixedAnswer`: if the set contains a `Public` address *and* any non-`Public` address, refuse regardless of `allow_private`. A hostname resolving to one public and one private address is an attack, not a config (§4.3.3).
  6. Any `CloudMetadata`, `Multicast`, `Unspecified` or `Nat64MappedPrivate` → `NetDenied::BlockedClass`. `Loopback`, `Private`, `LinkLocal` → refused unless `grant.allow_private`.
  7. `deadline_ms = now_ms + req.timeout_ms` — **one deadline spans authorize + resolve + dial + request + response** (§4.3.4). No per-stage budgets.
  `NetDenied` is `Display` and converts to `PiiError::Denied("net: <reason>: <detail>")`.
- **F2.4** `src/resolve.rs`: `pub trait Resolver: Send + Sync { fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, PiiError>; }`, `SystemResolver` (uses `ToSocketAddrs` — the only name-resolution call site in the workspace), `StaticResolver(BTreeMap<String, Vec<SocketAddr>>)` for tests. **`NetService::connect` resolves exactly once and then dials only `Pinned.addrs`, in order, never re-resolving** (§4.3.2). A test resolver that returns a different answer on its second call must not change the dialled address.
- **F2.5** `src/lib.rs`: `pub struct NetService(Arc<Inner>)`, built by `NetService::new(NetConfig { resolver: Arc<dyn Resolver>, grants: Arc<dyn GrantStore>, clock: Arc<dyn Clock> })`. `Clone` shares the `Arc` — the **connection registry is process-wide and keyed by canonical principal**, so cloning the service cannot multiply a `max_conns` lease (§4.3.5; zeroclaw hit exactly this bug). API: `connect(&self, ctx: &Ctx, target: &Target, key: &str) -> Result<Conn, PiiError>`, `request(&self, ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError>`, `listen(&self, ctx: &Ctx, spec: ListenSpec, proto: Protocol, handler: Arc<dyn HttpHandler>) -> Result<ListenerId, PiiError>`, `close(&self, id: ListenerId)`, `listeners(&self) -> Vec<ListenerInfo>`, `grants(&self) -> Vec<GrantInfo>`, `register_external_listener(&self, info: ListenerInfo)` (how the control UDS socket appears in `sys.status` without moving its bind).
- **F2.6** `src/dial.rs`: the only `TcpStream::connect` in the workspace. Dials each pinned address in order with the remaining deadline, returns the first success, and on total failure returns `PiiError::Unavailable("net: no address reachable for <host>")`. Requires `CapKind::NET` via `ctx.net()?` **before** anything else. Every denial emits `net_denied { target, reason }` on the bus via the observer.
- **F2.7** `src/conn.rs`: `pub struct Conn` wraps the stream and charges the grant: reads add to `bytes_in`, writes to `bytes_out`; exceeding `max_bytes_in`/`max_bytes_out` closes the connection and returns `PiiError::Denied("net: byte budget exhausted")`. `Drop` releases the connection lease. Counters are visible in `GrantInfo`.
- **F2.8** `src/http/codec.rs` — hand-written HTTP/1.1, no dependency:
  - Request: `<METHOD> <path-and-query> HTTP/1.1`, headers `Host`, `User-Agent: pii/0.1`, `Connection: close`, `Content-Length` when the body is non-empty, plus caller headers (caller headers may not override `Host` or `Connection`).
  - Response: status line (reject a non-`HTTP/1.` version), headers until the blank line (ASCII-case-insensitive names, value trimmed, header count ≤ 100, header block ≤ 64 KiB), body by `Content-Length` **or** `Transfer-Encoding: chunked` (chunk-size line, chunk, CRLF, terminating `0`), body cap **8 MiB** → `PiiError::Invalid("http: body too large")`.
  - No redirects (a 3xx is returned verbatim), no cookies, no keep-alive, no compression. Parsing is total: every malformed input is an `Err`, never a panic, never an index-out-of-range.
  - `pub fn parse_url(url: &str) -> Result<(Scheme, Target, String), PiiError>`; `https` → `PiiError::Invalid("https requires the tls feature (Phase 3b)")`.
- **F2.9** `src/http/client.rs`: `impl HttpTransport for NetService` — `parse_url` → `authorize` → `dial` → write request → read response with the shared deadline → `HttpResponse`. Status 429 and 5xx are returned as an `Ok(HttpResponse)`; mapping them to `PiiError::Unavailable` is the *caller's* job (F2.12), because a webhook caller may legitimately want the 503 body.
- **F2.10** `src/listen.rs` + `src/http/server.rs`: `ListenSpec::Tcp { addr: SocketAddr }`, `Protocol::Http`, `pub trait HttpHandler: Send + Sync { fn handle(&self, req: &HttpRequest) -> HttpResponse; }`. One accept thread per listener (non-blocking accept + 20 ms sleep, so `close()` is prompt and no thread is parked forever), one thread per connection, hard-capped by `max_conns` (excess connections get `503` and close). Request body ≤ 1 MiB. The server **always answers and never panics**: a handler that returns an error yields `500` with an empty body. `addr` with port `0` binds an ephemeral port; `ListenerInfo.addr` reports the bound port so tests need no fixed ports.
- **F2.11** `src/testing.rs` (feature `testing`): `pub struct FakeHttpServer` — built from a routes fixture `{"routes":[{"method":"GET","path_prefix":"/botTEST/getUpdates","status":200,"body":{…}}]}`, serves them through `NetService::listen`, records every received request (method, path, query, body) into a `Vec` readable with `requests()` and dumpable with `dump(path)`. It is the Bot API fake (E15), the Ollama fake (E11) and the egress test target. **Because it uses `NetService::listen`, no test needs a raw socket** and `net-check` stays clean.
- **F2.12** `pii-model` gains a real `OllamaProvider` (`src/ollama.rs`) built from `Arc<dyn HttpTransport>` — **zero socket code in `pii-model`**, proven by `net-check`. `POST {host}/api/chat` with `{model, messages, tools, stream: false}`; `host` from `OLLAMA_HOST` (via `pii_core::env_var`), default `http://127.0.0.1:11434`; `Authorization: Bearer $OLLAMA_API_KEY` when set. 429/5xx → `PiiError::Unavailable`; other non-2xx → `PiiError::Invalid`; a body that does not parse → `PiiError::Invalid`. `provider_from_env` grows `ollama:<model>` (it currently returns `Invalid`), taking the transport as a new argument. Its tests run against `FakeHttpServer` — no network.
- **F2.13** `xtask net-check` (`xtask/src/net_check.rs`): scans every `crates/*/src/**/*.rs` and `xtask/src/**/*.rs` for the symbols `TcpListener::bind`, `TcpStream::connect`, `UdpSocket::bind`, `UdpSocket::connect`, `UnixListener::bind`, `UnixStream::connect`, `to_socket_addrs`, `ToSocketAddrs`, `lookup_host`. Every hit must be in the allowlist `["crates/pii-net/src/dial.rs", "crates/pii-net/src/listen.rs", "crates/pii-net/src/resolve.rs", "crates/pii-server/src/uds.rs", "crates/pii-cli/src/cmd_daemon.rs"]`. A violation prints `NET-CHECK FAIL: <file>:<line> — <symbol> outside pii-net` and exits 1. The scan skips `#[cfg(test)]`-only files? **No** — tests are in scope too; `FakeHttpServer` exists so they do not need sockets. `cargo xtask ci` runs it. `clippy.toml` gains `std::net::TcpListener::bind`, `std::net::TcpStream::connect`, `std::net::UdpSocket::bind` with reason `"use NetService — grants, accounting, resolve-once-pin (§4)"`.
- **F2.14** `xtask net-check` has a unit test feeding **synthetic in-memory file contents** (a fake path outside the allowlist containing `TcpStream::connect`) and asserting a violation is reported — the gate is proven to fail when violated, not merely to pass today (§1.6's rule, and the pattern `deps_check_rejects_tui_graph_edge` already follows).

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
| `net::authorize_refuses_mixed_answers` | host resolving to `93.184.216.34` **and** `10.0.0.1`, `allow_private: true` | `Err(NetDenied::MixedAnswer)` — refused even with private allowed |
| `net::authorize_blocks_cloud_metadata` | resolved `[169.254.169.254]`, `allow_private: true` | `Err(NetDenied::BlockedClass)` naming `CloudMetadata` |
| `net::authorize_blocks_nat64_mapped_private` | resolved `[64:ff9b::10.0.0.1]` | `Err(NetDenied::BlockedClass)` |
| `net::authorize_allows_loopback_only_with_flag` | resolved `[127.0.0.1]`, grant with/without `allow_private` | `Ok` with the flag; `Err(BlockedClass)` without |
| `net::authorize_respects_expiry_and_scope` | expired grant; then host outside `HostScope` | `Err(Expired)`; then `Err(NotAllowed)` |
| `net::host_pattern_wildcard_depth` | `*.example.com` vs `a.example.com`, `a.b.example.com`, `example.com`, `evilexample.com` | `true, true, false, false` |
| `net::connect_resolves_once_and_pins` | `FlipResolver` returning a public addr first, `10.0.0.1` after | the dialled `SocketAddr` is the first answer; resolver called **exactly once** |
| `net::conn_lease_is_shared_across_clones` | `max_conns: 1`, two clones of the same `NetService` | the second `connect` returns `Denied`, not a second lease |
| `net::byte_budget_closes_connection` | `max_bytes_in: 1024`, a 4 KiB response | `Err(PiiError::Denied)` containing `byte budget`; `GrantInfo.bytes_in` ≤ 1024 + one read |
| `net::http_parses_chunked_and_content_length` | both encodings of the same body | identical `HttpResponse.body` |
| `net::http_rejects_oversized_and_malformed` | 9 MiB body; `HTTP/2 200`; header block > 64 KiB; truncated status line | four `Err`s, no panic |
| `net::https_is_an_explicit_refusal` | `https://api.telegram.org/x` | `Err(Invalid)` whose message names `tls` and `Phase 3b` |
| `net::listener_reports_bound_port_and_closes` | `listen(Tcp 127.0.0.1:0)` then `close` | `ListenerInfo.addr.port() != 0`; a later connect fails; the accept thread exits |
| `net::deadline_spans_all_stages` | `timeout_ms: 200` against a fake that stalls 2 s | `Err` within 400 ms, one deadline, no stage-stacking |
| `model::ollama_provider_uses_transport` | `FakeHttpServer` serving `fixtures/phase3/ollama/chat.json` | `ChatResponse.text` matches; the fake recorded exactly one `POST /api/chat` |
| `model::ollama_maps_5xx_to_unavailable` | fake returning 503, then 429, then 400 | `Unavailable`, `Unavailable`, `Invalid`; the first two are `is_retryable()` |
| `xtask::net_check_rejects_socket_outside_pii_net` | synthetic file `crates/pii-x/src/a.rs` containing `TcpStream::connect` | one violation naming the file and symbol |
| `proto::v2_skew_is_still_a_hard_refusal` | hello with `"pii.v99"` | code `100`, `data.expected == "pii.v2"`, connection closed |
| `proto::new_events_roundtrip` | every new `Event` variant | serialize→parse equals the original; `kind` tag present; schema matches `proto-check` |

---

## Exit Criteria

- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo build --workspace --all-targets`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && ./quality-gate.sh`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask lint-files`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask net-check`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p xtask net_check_rejects_socket_outside_pii_net -- --exact 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && ! grep -qE '^name = "(tantivy|hnsw_rs|fastembed|ort|axum|hyper|tower|reqwest|ureq|rustls|ring|aws-lc-rs|hickory-resolver|rmcp)"' Cargo.lock`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask proto-check`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-net --test egress 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-net --test http_codec 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-model --test ollama_http 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask bench-gate`

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

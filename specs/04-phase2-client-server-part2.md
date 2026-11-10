I read the contract sections, the current tree, and verified the environment facts the spec depends on (Landlock ABI v2 present, cached crate versions, `setpriv`/`nobody` available, crates.io reachable). Here is the spec. — Part 2/4

**Parent-Spec**: `04-phase2-client-server.md`
**Part**: 2 of 4
**Covers**: Exit criteria 4, 9, 12, 16, 18, 22
**Depends-On-Parts**: none — this part is self-contained

## Overview

**Project**: <repo>
**Phase**: 2 - Client/server split + coding harness

**Spec of record**: `_docs/06-research/002-pii-architecture-plan-pass2.md` §3 (client–server), §12.2 (dependency DAG invariants), §14 Phase 2; `_docs/06-research/001-pii-architecture-plan.md` §3.3 (starter graphs).

**The load-bearing requirement.** `cargo tree -p pii-tui` must not contain `pii-graph`. The TUI is a pure protocol client; the server owns the engine. Every other deliverable in this phase is subordinate to keeping that boundary honest.

**Scope cuts — stated explicitly, as instructed.**

- **`pii-acp` is CUT to a follow-up spec** (`specs/05-phase2b-acp-bridge.md`). ACP is a second client of the same protocol; it adds no proof the protocol boundary works that `pii-tui` does not already give, and it drags in `agent-client-protocol` plus two stdio work-arounds. Pass-2 exit criterion 8 ("`pii acp` completes a turn from Zed") moves with it.
- **Remote transport is CUT**: no WebSocket, no TLS, no `pii pair`, no bearer tokens, no `viewer` role. UDS + in-process loopback only. `Transport` is generic over `AsyncRead + AsyncWrite`, so WS is a later addition, not a later rewrite.
- **Encoding negotiation is CUT to a refusal**: `hello` carries `encodings`, the server answers `"json"`, and any client offering only non-`json` encodings is refused. Postcard lands when the delta stream is measured, not before.
- **Not in this phase**: `pii-obs`, `pii-memory`, `pii-net`, `pii-wasm`, `pii-sync`, `pii-mcp`, approvals (`approve.*`), `log.*`, `run.await`, `graph.diff`, `graph.history`, `sys.reload`, `sys.profile`.

**Scope ADDITION — read this before planning.** Pass-2 §14 assigns `pii-model` and `pii-tools` to Phase 1. The Phase 1 implementation (see `HANDOFF.md`) delivered `pii-core`, `pii-store`, `pii-graph` and the lint/run surface **without** them. Phase 2's exit criterion "Pii makes a real multi-file change in its own repo driven by `starter/verify-fix`" is unreachable without a model provider and edit/bash tools, so **`pii-model` and `pii-tools` are absorbed into this phase**. Both are kept minimal: one provider trait with a deterministic scripted impl and one HTTP impl; five tools.

**Phase 1 dependencies this spec assumes.** `pii_core::Ctx`/`CapSet`/`Budget`, `pii_store::Store` (`append_event`, `read_journal`, `get_run`, `next_run_id`, `runs_by_status`, `put_graph`), `pii_graph::{load_toml, validate, run, NodeRegistry, GraphSpec}`, `pii_abi::{RunEvent, RunHeader, RunStatus, Terminal, Outcome, NodeInput, PiiError, RunId, NodeId, PortName}`. All verified present. Where Phase 1 has already added an equivalent of something this spec asks for (notably a journal sink on the run entry, or `pii graph run` wired to the store), **reuse it and delete the duplicate requirement** rather than adding a second path.

---

## Requirements

### Functional

#### W1 — `pii-proto`: the wire contract

- **F1.1** New crate `crates/pii-proto`. `Cargo.toml` dependencies: `serde`, `serde_json`, `schemars`, `thiserror` (all `workspace = true`). Optional: `tokio = { workspace = true, optional = true }`. Features: `default = []`, `client = ["dep:tokio"]`. **No `pii-core`, no `pii-abi`, no `axum`, no `reqwest`.** Wire types are duplicated from `pii-abi` deliberately — the wire contract must not inherit the engine's tree.
- **F1.2** `src/lib.rs` defines:
  - `pub const PROTOCOL_VERSION: &str = "pii.v1";`
  - `pub enum Id { Num(u64), Str(String) }` (serde untagged).
  - `pub struct Request { pub jsonrpc: String, pub id: Id, pub method: Method, pub params: serde_json::Value }`
  - `pub struct Response { pub jsonrpc: String, pub id: Id, pub result: Option<serde_json::Value>, pub error: Option<RpcError> }` with `Response::ok(id, value)` and `Response::err(id, RpcError)` constructors; exactly one of `result`/`error` is `Some` (`skip_serializing_if = "Option::is_none"` on both).
  - `pub struct Notification { pub jsonrpc: String, pub method: Method, pub params: serde_json::Value }`
  - `pub enum Envelope { Request(Request), Response(Response), Notification(Notification) }`, `#[serde(untagged)]` for **serialization only**.
  - `pub fn parse_frame(line: &str) -> Result<Envelope, RpcError>` — hand-written classification, not `untagged` deserialization: parse to `serde_json::Value`; reject `jsonrpc != "2.0"` with `ErrorCode::InvalidRequest`; then `method` present + `id` present → `Request`; `method` present + no `id` → `Notification`; `id` present + (`result` or `error`) → `Response`; otherwise `ErrorCode::InvalidRequest`. Unknown method string → `ErrorCode::MethodNotFound` carrying the offending string.
  - `pub struct RpcError { pub code: i32, pub message: String, pub data: Option<serde_json::Value> }` and `pub enum ErrorCode` with `ParseError = -32700`, `InvalidRequest = -32600`, `MethodNotFound = -32601`, `InvalidParams = -32602`, `Internal = -32603`, `ProtocolMismatch = 100`, `Unauthorized = 101`, `NotFound = 102`, `Denied = 103`, `BudgetBreach = 104`, `Cancelled = 105`, `Busy = 106`.
- **F1.3** `src/method.rs`: `pub enum Method` with serde renames, `Display`, and `FromStr`. Exactly these variants and no others: `sys.hello`, `sys.status`, `sys.version`, `sys.shutdown`, `session.create`, `session.prompt`, `session.interrupt`, `session.state`, `session.list`, `session.close`, `run.start`, `run.get`, `run.list`, `run.cancel`, `run.resume`, `run.trace`, `graph.apply`, `graph.get`, `graph.list`, `graph.lint`, `event.subscribe`, `event.unsubscribe`. The enum is **closed**: no `Other(String)` variant. An unknown method must surface as `MethodNotFound`, never as a value.
- **F1.4** `src/sys.rs`: `HelloParams { protocol: String, client: PeerInfo, encodings: Vec<String>, features: Vec<String> }`, `PeerInfo { name: String, version: String, build: String }`, `HelloResult { protocol: String, server: PeerInfo, encoding: String, capabilities: Vec<String>, limits: Limits }`, `Limits { max_frame_bytes: u64, max_subscriptions: u32 }`, `StatusResult { uptime_ms: u64, runs_active: u32, sessions: u32, sandbox_profile: String }`.
- **F1.5** `src/session.rs`: `SessionCreateParams { cwd: String, model: Option<String>, tools: Vec<String>, graph: Option<String> }` → `SessionCreateResult { session_id: String }`; `SessionPromptParams { session_id: String, content: Vec<ContentBlock> }` → `SessionPromptResult { run_id: String }`; `ContentBlock` is a closed enum `{ Text { text: String } }` (tagged, `#[serde(tag = "type", rename_all = "snake_case")]`); `SessionStateResult { session_id, cwd, status: SessionStatus, messages: Vec<WireMessage>, tokens_in: u64, tokens_out: u64, run_id: Option<String> }`; `SessionStatus` ∈ `idle|running|closed`.
- **F1.6** `src/run.rs`: `RunStartParams { graph: String, input: serde_json::Value, caps: Option<Vec<String>> }` → `RunStartResult { run_id: String }`; `WireRunHeader { run_id, graph, status: WireRunStatus, seq: u64, dur_ms: u32 }`; `RunListParams { status: Option<WireRunStatus>, since: Option<u64> }`; `TraceResult { header: WireRunHeader, events: Vec<SeqEvent> }` where `SeqEvent { seq: u64, event: Event }`; `GraphApplyParams { name: String, toml: String }` → `GraphApplyResult { name: String, hash: String }`; `GraphLintParams { toml: String }` → `GraphLintResult { ok: bool, errors: Vec<String>, warnings: Vec<String> }`.
- **F1.7** `src/event.rs`: `pub enum Event` — the **only** server→client push vocabulary, closed, `#[serde(tag = "kind", rename_all = "snake_case")]`:
  `run_started { run_id, graph }`, `node_started { run_id, node }`, `node_finished { run_id, node, port, value }`, `node_failed { run_id, node, error, retryable }`, `node_dead { run_id, node }`, `assistant_delta { run_id, session_id, text }`, `tool_call { run_id, node, tool, args }`, `tool_result { run_id, node, tool, ok, summary }`, `run_finished { run_id, terminal: WireTerminal }`, `run_cancelled { run_id }`.
  `WireTerminal` = `{ success { output } | failure { error } | suspended { key } }`.
  `EventSubscribeParams { filter: EventFilter } ` with `EventFilter { run: Option<String>, session: Option<String>, kinds: Vec<String>, since_seq: Option<u64> }` → `EventSubscribeResult { sub_id: String }`. The push notification uses method `event.subscribe` params `EventPush { sub_id: String, seq: u64, event: Event }`.
- **F1.8** `src/client.rs`, gated `#[cfg(feature = "client")]`: `pub struct Client` built from any `R: AsyncRead + Unpin + Send + 'static` and `W: AsyncWrite + Unpin + Send + 'static`. API: `Client::connect(r, w, client_info) -> Result<(Self, HelloResult), ClientError>` (performs `sys.hello` and **fails hard** if `result.protocol != PROTOCOL_VERSION`), `async fn call(&self, m: Method, params: Value) -> Result<Value, RpcError>`, `fn events(&self) -> tokio::sync::mpsc::UnboundedReceiver<EventPush>`. One reader task demultiplexes responses (by `Id`) from notifications. Frames are newline-delimited; a line exceeding `max_frame_bytes` is a fatal `ParseError`.
- **F1.9** Version skew is a **hard refusal on both sides**. Server: `sys.hello` with `params.protocol != PROTOCOL_VERSION` → `RpcError { code: 100, message: "protocol mismatch", data: {"expected":"pii.v1","got":"<x>"} }`, then **close the connection**. No downgrade, no guessing, no partial service. Any method other than `sys.hello` before a successful hello → code `101`, connection closed.

#### W2 — `pii-server`: the daemon

- **F2.1** New crate `crates/pii-server`. Dependencies: `pii-proto` (features `["client"]` is **not** required here — the server needs only the wire types), `pii-abi`, `pii-core`, `pii-store`, `pii-graph`, `pii-model`, `pii-tools`, `tokio = { version = "1.53", features = ["rt-multi-thread","macros","net","io-util","sync","time","process"] }`, `rustix = { version = "1.1", features = ["process"] }`, `serde`, `serde_json`, `thiserror`, `parking_lot`.
- **F2.2** `Engine` (in `src/lib.rs`) owns: `Arc<Store>`, `Arc<NodeRegistry>` (built by `pub fn registry(cfg: &EngineConfig) -> NodeRegistry`, registering the built-ins plus `agent` and `tool`), `Arc<dyn ModelProvider>`, `ToolSet`, the session map, the run registry, the event bus, and `started_at: Instant`. `EngineConfig { workspace: PathBuf, store_path: PathBuf, model: String, sandbox: SandboxProfileName, graph_dir: PathBuf }`. `EngineConfig` has **no `Default`** — every field is a security-relevant choice (pass-2 §1.2).
- **F2.3** Run registry (`src/runs.rs`): `start(graph: GraphSpec, input: Value, caps: CapSet) -> RunId` allocates the id from `Store::next_run_id()`, writes the `RunHeader` + `RunEvent::Started`, spawns the graph on `tokio::task::spawn_blocking`, and **returns before the graph executes**. Per-run state: `RunId → { header, cancel: CancelToken, status }`. `cancel(run_id)` trips the token; the scheduler observes it via `Ctx::cancelled()` and terminates with `Terminal::Failure { error: "cancelled" }` after emitting `run_cancelled`.
- **F2.4** Journal→bus bridge (`src/bus.rs`): `struct JournalSink { store: Arc<Store>, header: Mutex<RunHeader>, tx: broadcast::Sender<SeqEvent> }` implementing `pii_graph::RunSink`. For every `RunEvent`: append to redb (obtaining `seq`), map to the wire `Event`, broadcast `SeqEvent { seq, event }`. **redb is written by the sink only** — no other task opens the store for writing.
- **F2.5** Subscriptions: `event.subscribe` with `since_seq: Some(n)` first **replays** `Store::read_journal(run)` filtered to `seq > n`, then attaches the live `broadcast::Receiver`, dropping any live event whose `seq <= last_replayed_seq`. This is the no-duplicate reattach guarantee. `max_subscriptions = 32` per connection; exceeding it → code `106`.
- **F2.6** Connection loop (`src/conn.rs`): `pub async fn serve_connection<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(engine: Arc<Engine>, io: S, peer: PeerIdentity)`. Split the stream; a writer task owns the write half and drains an `mpsc` so responses and notifications never interleave mid-line. Read loop: line → `parse_frame` → dispatch → response. A `Request` handler must never block the read loop: long work is spawned and answered by a notification.
- **F2.7** Dispatch (`src/dispatch.rs`) implements every `Method` in F1.3. Rules: `session.prompt` and `run.start` return their id immediately (< 50 ms, F/N budget below); `session.interrupt`/`run.cancel` are idempotent; `run.trace` reads the journal; `graph.lint` calls `pii_graph::validate` and returns the `V<n>`-prefixed strings verbatim; `graph.apply` validates before `Store::put_graph`, refusing invalid graphs with code `-32602`.
- **F2.8** UDS transport (`src/uds.rs`): resolve the socket dir as `$PII_RUNTIME_DIR` → `$XDG_RUNTIME_DIR/pii/<instance>` → `/run/user/<uid>/pii/<instance>`; error if none is usable. Create the dir mode `0700`, unlink a stale socket **only after** confirming no live daemon answers on it, bind, `chmod 0600`. On accept: `stream.peer_cred()?.uid()` must equal `rustix::process::getuid().as_raw()`; otherwise write one `RpcError` frame with code `101`, log `refused uid <n>`, and close. `pub fn authorize_peer(daemon_uid: u32, peer_uid: u32) -> Result<(), RpcError>` is the single decision point and is unit-testable.
- **F2.9** Loopback transport (`src/loopback.rs`): `pub fn loopback(engine: Arc<Engine>) -> (DuplexStream, JoinHandle<()>)` using `tokio::io::duplex(1 << 20)`; the server half is served by `serve_connection` with `PeerIdentity::InProcess` (hello still required, peercred not applicable). **Identical code path** to UDS — no `if loopback` branch anywhere in dispatch.
- **F2.10** `agent` node kind (`src/nodes/agent.rs`). Config: `{ system: String, tools: Vec<String>, max_turns: u32 (default 24), compact_at_tokens: u64 (default 24000), model: Option<String> }`. Ports: `ok`, `err`, `budget`. Behaviour: build the message list from `system` + `$.payload` (and `$.payload.feedback` when present); loop up to `max_turns`: call the provider; if the response has tool calls, execute each through the `ToolSet` under the node's `Ctx` (emitting `tool_call`/`tool_result` events) and append the results; otherwise return `Next { ok, {"text": .., "turns": n, "files_changed": [..]} }`. Exceeding `max_turns` → `Next { budget, .. }`. A provider error that `is_retryable()` is retried once, then `Next { err, .. }`.
- **F2.11** Compaction gate: before each provider call, if the estimated token count (`chars / 4`) exceeds `compact_at_tokens`, replace all but the system message and the last 4 messages with a single summary message produced by one provider call using the `summarize` prompt. `pub fn compact(msgs: &[ChatMessage], provider: &dyn ModelProvider, budget: u64) -> Result<Vec<ChatMessage>, PiiError>` is a free function with its own deterministic test.
- **F2.12** `tool` node kind (`src/nodes/tool.rs`). Config: `{ tool: String, args: serde_json::Value }` where `args` values may be `$.`-expressions resolved with the existing `pii_graph::expr::eval_or_literal`. Ports: `ok`, `err`. The value on `ok` is the tool's JSON result; a tool error yields `Next { err, {"error": ..} }` (a routable failure, not a run abort).

#### W3 — `pii-model` and `pii-tools`

- **F3.1** New crate `crates/pii-model`. Deps: `pii-abi`, `pii-core`, `serde`, `serde_json`, `thiserror`, `ureq = { version = "3.4", default-features = false, features = ["rustls","json"] }`. Types: `ChatMessage { role: Role, content: String, tool_call_id: Option<String>, tool_calls: Vec<ToolCall> }`, `Role ∈ system|user|assistant|tool`, `ToolCall { id, name, arguments: Value }`, `ToolSpec { name, description, schema: Value }`, `ChatRequest { model, messages, tools, max_tokens, temperature }`, `ChatResponse { text, tool_calls, tokens_in, tokens_out }`. `pub trait ModelProvider: Send + Sync { fn name(&self) -> &str; fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError>; }` — **sync**, because `GraphNode::run` is sync and the run executes on `spawn_blocking`.
- **F3.2** `ScriptedProvider` (`src/scripted.rs`): loads a JSON fixture `{ "turns": [ ChatResponse, ... ] }` and returns turn *n* on call *n*; running past the end is `PiiError::Invalid("scripted provider exhausted")`. It performs **no I/O beyond the fixture read** and is the provider used by every deterministic test and every CI exit criterion.
- **F3.3** `OllamaProvider` (`src/ollama.rs`): POSTs `{host}/api/chat` (OpenAI-compatible `/v1/chat/completions` when `host` ends in `/v1`) with `Authorization: Bearer $OLLAMA_API_KEY` when set. Host from `OLLAMA_HOST`, default `https://ollama.com`. Non-2xx → `PiiError::Retryable` for 429/5xx, `PiiError::Invalid` otherwise. **All `ureq` usage is confined to this one file** — if its API differs from what the implementer expects, only this file changes and no exit criterion moves.
- **F3.4** `pub fn provider_from_env(spec: &str) -> Result<Arc<dyn ModelProvider>, PiiError>`: `scripted:<path>` → `ScriptedProvider`; `ollama:<model>` → `OllamaProvider`; anything else → `PiiError::Invalid("unknown model spec")`. The CLI reads the spec from `--model`, else `$PII_MODEL`, else errors with exit code 5 and the message `no model configured (set PII_MODEL or --model)`.
- **F3.5** New crate `crates/pii-tools`. Deps: `pii-abi`, `pii-core`, `serde`, `serde_json`, `thiserror`, `dunce`, and `[target.'cfg(target_os = "linux")'.dependencies] nono = { version = "=0.75.0", default-features = false }`. `pub trait Tool: Send + Sync { fn name(&self) -> &'static str; fn spec(&self) -> ToolSpec; fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError>; }` plus `ToolSet` (a `BTreeMap<&'static str, Arc<dyn Tool>>` with `builtin()` and `subset(&[String])`).
- **F3.6** Tools (`src/fs.rs`): `fs_read { path, offset?, limit? }`, `fs_write { path, content }`, `edit { path, old_string, new_string }` (fails unless `old_string` occurs **exactly once**), `multi_edit { path, edits: [{old_string,new_string}] }` (applied in order, all-or-nothing, written once). All go through `Ctx::fs()` — never `std::fs` directly, so capability denial is enforced by `pii-core`, not by the tool.
- **F3.7** Bash mediation (`src/bash.rs`): `bash { command, cwd?, timeout_ms? }`. The mediator, in order: (1) require `CapKind::PROC` via `Ctx::proc()`; (2) reject the command if it matches the deny list (`rm -rf /`, `:(){`, `mkfs`, `dd of=/dev`, `shutdown`, `reboot`, and any command containing `sudo`) with `PiiError::Denied`; (3) canonicalise `cwd` (default: the workspace) with `dunce::canonicalize` and reject it if it is not inside the workspace; (4) execute via `ProcCap::run` under `sandbox-exec` when a profile is active; (5) cap combined stdout+stderr at 64 KiB (truncate with a marker) and enforce `timeout_ms` (default 120000, max 600000). Result: `{ exit_code, stdout, stderr, truncated }`.
- **F3.8** `pii-core` gains `ProcCap::run(&self, argv: &[String], cwd: &Path, timeout: Duration) -> Result<ProcOutput, PiiError>`, checking `argv[0]` against `CapSet::proc_argv0`. This is the only place in the workspace allowed to construct a `std::process::Command` (`clippy.toml` already disallows `Command::spawn`; add an `#[allow]` with a `// reason:` comment here and nowhere else).
- **F3.9** Sandbox (`src/sandbox.rs`): `SandboxProfileName ∈ Off | Workspace`. The `workspace` profile grants read on `/`, read-write on `[workspace, store_dir, runtime_dir, $TMPDIR]` and read-write on the openable device files `/dev/null`, `/dev/zero`, `/dev/urandom`, `/dev/tty` and directory `/dev/pts`. API mirrors the verified `nono` surface: `nono::CapabilitySet::new()`, `caps = caps.allow_path(path_str, nono::AccessMode::Read|ReadWrite)?`, `caps.allow_file_mut(path, AccessMode::ReadWrite)`, `nono::Sandbox::support_info()`, `nono::Sandbox::apply(&caps)`. A path that does not exist is skipped for read grants and created for read-write grants. **The contract is behavioural, not API-shaped**: if the `nono` 0.75 API differs, adapt it, or substitute `landlock = "0.4"` — what must hold is F3.10.
- **F3.10** Enforcement self-verification, fail-closed: after `apply`, write a probe file at `<probe_dir>/.pii-sandbox-probe-<pid>` where `probe_dir` defaults to `/etc`. If the write **succeeds**, delete the probe and abort with exit code 4 and `sandbox: enforcement could not be verified`. If `support_info().is_supported` is false, or `apply` errors, or `$PII_SANDBOX_FORCE_UNSUPPORTED=1` is set, abort with exit code 4 **before** serving anything. Pii never runs degraded when a profile was requested. (Verified in this container: `landlock_create_ruleset(NULL,0,1)` returns ABI **2**, so enforcement is available; the ruleset must be built best-effort against ABI 2 and must not require v3+ access rights.)

#### W4 — `pii-graph` additions

- **F4.1** `src/sink.rs`: `pub trait RunSink: Send + Sync { fn emit(&self, event: &RunEvent); }` plus `pub struct NullSink;`.
- **F4.2** `pub fn run_with_sink(spec, reg, ctx, run: &RunId, input, sink: &dyn RunSink) -> Result<Terminal, PiiError>`. The existing `run(..)` becomes `run_with_sink(.., &NullSink)` so all Phase 1 tests keep passing unchanged. Emitted, in order: `Started`, then per tick `TickStarted`, then per node `NodeStarted` / (`NodeFinished` | `NodeFailed` | `NodeCancelled`), `NodeDead` for dead-propagated nodes, and finally `Finished`.
- **F4.3** Node capability narrowing — **fixes a live bug**. `run.rs` currently builds each node context with `CapSet::empty()`, which denies `Ctx::fs()` to every node including `emit`. Add `CapSet::narrow_kinds(&self, kinds: CapKind) -> CapSet` to `pii-core` (clone `self`, `kinds &= kinds`; scopes untouched) and dispatch with `ctx.child(principal, &ctx.caps().narrow_kinds(graph_kinds & node_required_kinds), &BudgetShare::All)`. Path/host scopes are inherited from the run context; only the kind bits attenuate.
- **F4.4** Cancellation: `run_with_sink` checks `ctx.cancelled()` at the top of every tick and before each dispatch, returning `Terminal::Failure { error: "cancelled" }`.
- **F4.5** Loop re-entry — required by `starter/verify-fix`, absent today (`RunState::executed` is permanent, so a cycle can never iterate). In `apply_completion`, when the edge target is already in `st.executed` **and** its `NodeSpec::loop_budget` is `Some(b)`: increment `st.iters[target]`; if it exceeds `b`, return `Err("loop_budget exceeded for node '<id>'")` → `Terminal::Failure`. Otherwise, **before** marking the incoming edge `Fired`: BFS from `target` over all outbound edges (Data and Order), collecting the reachable set including `target`; for every node in it remove from `executed` and `dead` and reset every edge whose `from` is in the set to `EdgeState::Pending`. A target already executed with `loop_budget: None` is `Err("re-entry into node '<id>' without loop_budget")` — the validator's V3 makes this unreachable via `lint`, so it is a defence, not a path.
- **F4.6** `NodeRegistry::register(&mut self, factory: Arc<dyn NodeFactory>)` keyed on `factory.kind()`, so `pii-server` can add `agent`/`tool` without `pii-graph` depending on `pii-model`/`pii-tools`.

#### W5 — `pii-tui` and `pii-cli`

- **F5.1** New crate `crates/pii-tui`. Direct dependencies, **exactly**: `pii-proto = { path = "../pii-proto", features = ["client"] }`, `ratatui = "=0.30.2"`, `crossterm = { version = "0.29", features = ["event-stream"] }`, `tokio`, `serde_json`, `thiserror`. Nothing else, ever. *(ratatui 0.30 moved backends into separate crates: the `crossterm` backend is a default feature routed through `ratatui-crossterm 0.1.2`, which pins `crossterm ^0.29`. Keep ratatui's default features; add `crossterm` directly only for `event-stream`, which ratatui does not re-enable. Exact pin `=0.30.2` per pass-1 §17 — a 0.30.x bump changes widget layout behaviour.)*
- **F5.2** `pii-tui` holds **no domain state**. `AppState` contains only: the last `SessionStateResult` snapshot, a `VecDeque<Event>` of received deltas (capped at 500), the connection status, and the input buffer. Every field is populated from a protocol message. There is no scheduler, no graph, no store, no model. The TUI must not compute anything the server also computes — no local token counting, no local status derivation, no local retry logic.
- **F5.3** `pub async fn run_tui(conn: impl AsyncRead + AsyncWrite) -> Result<(), TuiError>`: hello → `session.create` → `event.subscribe` → snapshot via `session.state` → render loop. Reconnect path: re-hello, re-subscribe with `since_seq = last_seq`, re-fetch the snapshot.
- **F5.4** `pub fn render(state: &AppState, frame: &mut Frame)` is pure and separately testable against a `ratatui::backend::TestBackend`.
- **F5.5** `pii-cli` gains subcommands, dispatched on `argv[1]` before any heavy initialisation (pass-1 §4.2, preserving the `pii --version` p95 < 20 ms gate):
  - `pii -p "<prompt>" [--json] [--model M] [--graph G] [--workspace D] [--sandbox off|workspace]` — the headless coding entry point.
  - `pii daemon [--sandbox …] [--workspace …] [--store …]`
  - `pii tui [--connect auto|loopback|<socket path>]`
  - `pii sandbox-exec --profile <p> --workspace <d> -- <argv…>`
  - existing `pii graph {lint|run|resume|trace}` unchanged, plus `run` now emitting journal events through the same sink.
- **F5.6** `pii -p` transport selection: if a live daemon socket is reachable, use UDS; otherwise construct the engine in-process and use `loopback()`. `--connect` overrides. **The daemon is never auto-started.**
- **F5.7** `pii -p` output, documented and parseable: with `--json` (and by default when stdout is not a TTY), one JSON object per line — every `Event` exactly as it arrives, plus a final `{"kind":"run_finished",...}`. Exit codes: `0` success, `1` run failure, `2` usage error, `3` lint/validation failure, `4` sandbox verification failure, `5` no model configured. Without `--json` on a TTY, a human line format: `⏵ <node>`, `  ↳ <tool> <summary>`, `✔ done` / `✘ <error>`.
- **F5.8** The ndjson emitted over loopback and over UDS must be **byte-identical** after normalising `run_id` and any `*_ms` field. One execution path, proven by E7.

#### W6 — starter graphs, xtask gate, fixtures

- **F6.1** `graphs/starter/react.toml` — `starter/react`: entry `agent` (kind `agent`, `tools = ["fs_read","fs_write","edit","multi_edit","bash"]`, `max_turns = 24`, `compact_at_tokens = 24000`) → port `ok` → `done` (kind `transform`, `expr = "$.payload"`); port `err` → `fail` (kind `transform`); port `budget` → `fail`. Graph `caps = ["fs.read","fs.write","proc"]`.
- **F6.2** `graphs/starter/verify-fix.toml` — `starter/verify-fix`: `fix` (kind `agent`, `loop_budget = 3`) → `test` (kind `tool`, `tool = "bash"`, `args.command = "$.input.test_command"`, `args.cwd = "$.input.workspace"`) → `gate` (kind `route`, `on = "$.payload.exit_code"`, rules `[{eq = 0, port = "pass"}, {port = "fail"}]`); `pass` → `done` (`transform`); `fail` → `retry` (`transform` building `{"feedback": <test output>}`) → **edge back to `fix`**, exercising F4.5. Both graphs must pass `pii graph lint`.
- **F6.3** `xtask` gains `deps-check` (`cargo xtask deps-check`), asserting the four pass-2 §12.2 invariants from `cargo metadata --format-version 1 --no-deps` plus the resolved graph: (1) `pii-abi` has no `tokio`/`reqwest`/`redb`/`wasmtime`/`axum` in its tree; (2) `pii-proto` (default features) has no `pii-core`/`pii-abi`/`tokio`/`axum`; (3) **`pii-tui`'s tree contains no `pii-graph`**; (4) `pii-tui`'s direct dependencies are a subset of the F5.1 allowlist. Each violation prints `DEPS-CHECK FAIL: <invariant> — <detail>` and exits 1. `cargo xtask ci` runs `deps-check`, `lint-files`, fmt, clippy and tests.
- **F6.4** `xtask` unit test `deps_check_rejects_tui_graph_edge` feeds a synthetic in-memory package graph in which `pii-tui → pii-graph` and asserts the checker reports the violation — so the gate is proven to fail when violated, not merely to pass today.
- **F6.5** Fixtures under `fixtures/phase2/`: `hello.json` (one turn, no tool calls), `fail.json` (one turn whose text triggers a failing route), `selfchange.json` (a turn issuing two `edit` calls against two distinct files, then a turn with no tool calls), `compact.json`. `scripts/phase2/uds-vs-loopback.sh` and `scripts/phase2/selfchange.sh` implement E7 and E13; both write only under `_tmp/phase2/`.

### Non-Functional

- **N1** Edition 2024; workspace lints apply to every new crate (`[lints] workspace = true` in each `Cargo.toml`). `unsafe_code = "forbid"`; `unwrap_used`/`expect_used`/`panic`/`indexing_slicing`/`todo`/`unimplemented` are `deny` in non-test code. Test modules keep the existing `#![allow(...)]` header used across the repo.
- **N2** No file exceeds **450 effective lines** (`cargo xtask lint-files`). Split by the module boundaries listed in W1–W6 rather than growing a file.
- **N3** `pii-proto` with default features must not pull `tokio`, `axum`, `pii-core` or `pii-abi`. Asserted (E2).
- **N4** `session.prompt` and `run.start` return in **< 50 ms** regardless of run duration. Asserted (E10) with a scripted provider whose first turn sleeps 2 s.
- **N5** Never hold a lock across `.await` (`clippy::await_holding_lock` is already `deny`). The run registry uses `parking_lot::Mutex` for short critical sections only; anything that awaits goes through `tokio::sync`.
- **N6** `pii --version` stays p95 < 20 ms: `main` branches on `argv[1]` before constructing an engine, opening a store, or touching the network.
- **N7** No panics on the server path. A handler that would panic must return `ErrorCode::Internal` instead; a poisoned run must not take down the daemon.
- **N8** All artifacts, scratch repos, sockets and logs go under `_tmp/phase2/`. Never `/tmp`, never `$HOME`.
- **N9** Secrets never cross the wire: `OLLAMA_API_KEY` and any provider credential are read by the server only and must not appear in any `Event`, `session.state` or `run.trace` payload.
- **N10** New workspace dependencies to add to the root `Cargo.toml` `[workspace.dependencies]`: `tokio = { version = "1.53", default-features = false }`, `ureq = "3.4"`, `rustix = "1.1"`, `ratatui = "=0.30.2"`, `crossterm = "0.29"`, `nono = "=0.75.0"`. *(All except `ureq` and `nono` are already in the local cargo cache; crates.io is reachable from this container — verified. `nono` is exact-pinned because its rule-emission order is load-bearing, per pass-1 §1.4; 0.75.0 is chosen over grok-build's `=0.53.0` because its MSRV 1.95 matches this toolchain exactly and the 0.53 pin was motivated by macOS Seatbelt ordering, which this Linux-only phase does not exercise.)*

---

## Scope (Part 2 of 4)

Create or modify **only** these artifacts:

- `fail.json`
- `react.toml`
- `verify-fix.toml`

Anything else in the Architecture belongs to another part. Creating it
here duplicates that part's work and makes the split pointless.

Part 1 owns:
- `hello.json`
- `uds-vs-loopback.sh`

Part 3 owns:
- `selfchange.sh`

Part 4 owns:
- `quality-gate.sh`

## Architecture (full system — context, not a build list)

```
                    ┌───────────────────────────────────────────────┐
                    │  pii-cli  (the binary: argv → mode dispatch)  │
                    └───┬───────────┬───────────┬───────────┬───────┘
             pii -p     │   daemon   │    tui    │  sandbox-exec
                        │            │           │           │
        ┌───────────────▼──┐     ┌───▼────┐  ┌───▼─────┐  ┌──▼──────────┐
        │ loopback         │     │  UDS   │  │ pii-tui │  │ pii-tools:: │
        │ tokio::io::duplex│     │ 0600   │  │ CLIENT  │  │  sandbox    │
        └───────┬──────────┘     │peercred│  └───┬─────┘  │ nono/Landlock│
                │                └───┬────┘      │        └─────────────┘
                │   ndjson JSON-RPC 2.0          │ ndjson
                └────────────┬───────────────────┘
                             │            ▲
        ═════════════════════╪════════════╪══════════ THE BOUNDARY ═══════
                             ▼            │  pii-proto ONLY (wire types)
        ┌────────────────────────────────────────────────────────────────┐
        │ pii-server                                                     │
        │  conn.rs  hello gate → version skew = HARD REFUSAL (code 100)  │
        │  dispatch.rs  sys.* session.* run.* graph.* event.*            │
        │  runs.rs  registry: RunId → {header, CancelToken, status}      │
        │  bus.rs   JournalSink: RunEvent → redb(seq) → broadcast        │
        │  nodes/agent.rs (model loop + tools + compaction gate)         │
        │  nodes/tool.rs                                                 │
        └───┬───────────────┬──────────────┬──────────────┬──────────────┘
            │               │              │              │
      ┌─────▼─────┐  ┌──────▼─────┐  ┌─────▼─────┐  ┌─────▼──────┐
      │ pii-graph │  │ pii-store  │  │ pii-model │  │ pii-tools  │
      │ scheduler │  │ redb       │  │ Scripted  │  │ fs/edit/   │
      │ +RunSink  │  │ journal    │  │ Ollama    │  │ multi_edit │
      │ +loops    │  └─────┬──────┘  └───────────┘  │ bash+sandbox│
      └─────┬─────┘        │                        └─────┬──────┘
            └──────────────┴────────► pii-core (Ctx/CapSet/Budget) ──► pii-abi

  cargo tree -p pii-tui  ✗ pii-graph   ← THE GATE (E1, xtask deps-check)
```

Run lifecycle:

```
session.prompt ──> run_id (< 50 ms) ──> spawn_blocking
                                          │
   run_with_sink ──emit──> JournalSink ──append──> redb (seq n)
                                │
                                └──broadcast──> subscribers ──notification──> client
   reattach: event.subscribe{since_seq: n} → replay journal (seq > n) → live (drop seq ≤ n)
```

---

> **Note**: the Architecture above describes the FULL system, including
> artifacts owned by other parts. Build only what Scope lists.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `proto::parse_frame_classifies_request` | `{"jsonrpc":"2.0","id":1,"method":"sys.hello","params":{}}` | `Envelope::Request` with `Method::SysHello` |
| `proto::parse_frame_classifies_notification` | frame with `method`, no `id` | `Envelope::Notification` |
| `proto::parse_frame_classifies_response` | frame with `id` + `result` | `Envelope::Response` |
| `proto::unknown_method_is_method_not_found` | `"method":"sys.nope"` | `RpcError` code `-32601`, message names `sys.nope` |
| `proto::bad_jsonrpc_version_rejected` | `"jsonrpc":"1.0"` | `RpcError` code `-32600` |
| `proto::event_roundtrip` | each `Event` variant | serialize→parse equals the original; `kind` tag present |
| `server::hello_skew_is_hard_refusal` | hello with `protocol:"pii.v2"` | response code `100`, `data.expected == "pii.v1"`, connection closed, **no session created** |
| `server::method_before_hello_refused` | `session.create` as first frame | code `101`, connection closed |
| `server::peercred_rejects_foreign_uid` | `authorize_peer(0, 65534)` | `Err` code `101` |
| `server::peercred_live_refusal` | real connect from `setpriv --reuid=65534` to a live socket | server logs `refused uid 65534`; client reads a code-`101` frame then EOF |
| `server::prompt_returns_before_run_finishes` | scripted provider, first turn sleeps 2000 ms | `session.prompt` returns a `run_id` in < 50 ms; `run_finished` arrives later |
| `server::cancel_stops_run` | `run.cancel` during a 2 s node | `run_cancelled` event; terminal is `failure{error:"cancelled"}`; no further `node_started` |
| `server::reattach_since_seq_no_duplicates` | subscribe, drop at seq 3, resubscribe `since_seq:3` | every seq 4..=N delivered exactly once, strictly increasing, none ≤ 3 |
| `server::journal_is_the_only_writer` | 3 concurrent runs | all journals readable; `read_journal` seqs are gapless and per-run monotonic |
| `server::loopback_and_uds_agree` | same prompt, same fixture, both transports | ndjson identical after normalising `run_id` and `*_ms` |
| `graph::run_with_sink_emits_ordered_events` | 3-node linear graph | `Started, TickStarted, NodeStarted, NodeFinished, …, Finished` in that order; one `NodeFinished` per node |
| `graph::null_sink_preserves_phase1_behaviour` | every existing Phase 1 example | terminals identical to `run()` before this change |
| `graph::loop_reenters_and_reexecutes` | `verify-fix` with a fixture failing once then passing | `fix` and `test` each emit **2** `NodeFinished`; terminal `success` |
| `graph::loop_budget_exceeded_fails` | fixture that always fails, `loop_budget = 3` | terminal `failure` with `loop_budget exceeded for node 'fix'`; exactly 3 `fix` executions |
| `graph::node_caps_narrow_not_empty` | `emit` node in a graph granting `fs.write` to the workspace | write succeeds; the same graph without `fs.write` yields `PiiError::Denied` |
| `tools::edit_requires_unique_match` | `old_string` occurring twice | `Err(PiiError::Invalid)`, **file unchanged on disk** |
| `tools::multi_edit_is_all_or_nothing` | 2 edits, second not found | `Err`; file byte-identical to before |
| `tools::bash_rejects_cwd_outside_workspace` | `cwd = "/etc"` | `Err(PiiError::Denied)`, no process spawned |
| `tools::bash_denies_blocklisted_command` | `command = "sudo rm -rf /"` | `Err(PiiError::Denied)` before any spawn |
| `tools::bash_truncates_and_times_out` | command emitting 1 MiB; command sleeping 5 s with `timeout_ms:200` | `truncated:true`, ≤ 64 KiB; timeout returns non-zero `exit_code` within 1 s |
| `sandbox::probe_detects_unenforced` | `PII_SANDBOX_FORCE_UNSUPPORTED=1` | exit 4, stderr `sandbox: enforcement could not be verified`, nothing served |
| `sandbox::write_outside_workspace_denied` | `sandbox-exec … sh -c 'echo x > /etc/pii-probe'` | child exits non-zero; `/etc/pii-probe` does not exist |
| `sandbox::write_inside_workspace_allowed` | same, writing under the workspace | exit 0, file present |
| `model::scripted_returns_turns_in_order` | fixture with 2 turns | call 1 → turn 1, call 2 → turn 2, call 3 → `Err("scripted provider exhausted")` |
| `server::compaction_triggers_once_over_threshold` | history over `compact_at_tokens` | exactly one summarise call; system message preserved; last 4 messages preserved |
| `tui::renders_snapshot_without_engine` | recorded ndjson replayed into `AppState` | `TestBackend` buffer contains the assistant text and node names; `AppState` has no field not derived from a protocol message |
| `tui::reconnect_resubscribes_with_last_seq` | drop mid-stream | the re-sent `event.subscribe` carries `since_seq` equal to the last seq rendered |
| `xtask::deps_check_rejects_tui_graph_edge` | synthetic graph with `pii-tui → pii-graph` | checker returns a violation naming invariant 3 |
| `cli::oneshot_exit_codes` | scripted success / failure / no `PII_MODEL` | exit 0 / 1 / 5 respectively |

---


## Exit Criteria

- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo xtask deps-check`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && bash -c 'PII_RUNTIME_DIR=/projects/pi-improved/_tmp/phase2/nodaemon PII_MODEL=scripted:fixtures/phase2/fail.json ./target/debug/pii -p "boom" --json > _tmp/phase2/fail.ndjson; test $? -eq 1'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-server --test peercred -- --nocapture 2>&1 | grep -q 'refused uid 65534'`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -p pii-graph --test loops`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && ./target/debug/pii graph lint graphs/starter/react.toml && ./target/debug/pii graph lint graphs/starter/verify-fix.toml`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && bash -c 'PII_SANDBOX_FORCE_UNSUPPORTED=1 PII_RUNTIME_DIR=/projects/pi-improved/_tmp/phase2/rt ./target/debug/pii daemon --sandbox workspace --workspace /projects/pi-improved 2> _tmp/phase2/sandbox-refusal.txt; test $? -eq 4' && grep -q 'enforcement could not be verified' _tmp/phase2/sandbox-refusal.txt`

## Guardrails

- Do NOT add any dependency to `pii-tui` beyond `pii-proto`, `ratatui`, `crossterm`, `tokio`, `serde_json`, `thiserror` — not in `[dependencies]`, not in `[dev-dependencies]`, not transitively. If a TUI test needs a server, record its ndjson into a fixture file instead.
- Do NOT let `pii-proto` depend on `tokio` by default, and do NOT let it depend on `pii-core`, `pii-abi`, `axum` or `reqwest` at all. Duplicating a few wire structs is the intended cost.
- Do NOT put domain logic in `pii-tui`. No token counting, no status inference, no retry, no graph knowledge. If the TUI needs a number, the server sends it.
- Do NOT add a second execution path for `pii -p`. Loopback and UDS differ only in which `AsyncRead + AsyncWrite` reaches `serve_connection`. Any `if loopback { … } else { … }` in dispatch is a defect.
- Do NOT soften version skew into a warning, a downgrade, or a best-effort mode. Mismatch → code 100 → close.
- Do NOT run degraded when a sandbox profile was requested. If enforcement cannot be positively verified by the probe, exit 4 before serving.
- Do NOT use `unsafe`, and therefore do NOT use `pre_exec` for sandboxing. Confinement is applied process-wide in the re-exec'd `pii sandbox-exec` child and inherited across `execve`.
- Do NOT call `std::process::Command` anywhere except `ProcCap::run`, and do NOT call `std::fs` from a tool — go through `Ctx::fs()` so capability denial is enforced in one place.
- Do NOT write outside `_tmp/phase2/` for any artifact, socket, scratch repo or log. Never `/tmp`.
- Do NOT delete or `rm -rf` anything that existed before the run. `scripts/phase2/*.sh` must `mv` a stale scratch dir to `<name>.old-$(date +%s)` rather than removing it.
- Do NOT modify `_repos/`, `_docs/`, `AGENTS.md`, `HANDOFF.md`, or `.pidag/`.
- Do NOT run `git commit`, `git add`, `git push`, or create branches. Files only.
- Do NOT weaken an existing Phase 1 test to make a new one pass. `run()` must keep its exact current behaviour through `NullSink`; if a Phase 1 test breaks, the new code is wrong.
- Do NOT exceed 450 effective lines in any file, and do NOT silence a workspace lint with a blanket `#![allow]`. Per-item `#[allow]` with a `// reason:` comment only, and only in the two places this spec names (`ProcCap::run`, the CLI's stdout/stderr printing).
- Do NOT implement `pii-acp`, WebSocket/TLS transport, pairing tokens, postcard encoding, `pii-obs`, memory, or networking methods. They are out of scope and their absence is not a gap.
- **Do NOT create artifacts owned by other parts.** Only what Scope lists.
The Architecture section describes the full system; building all of it here
duplicates other parts' work and defeats the split.

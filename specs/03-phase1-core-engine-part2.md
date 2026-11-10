# Spec: Phase 1 - Core Engine — Part 2/4

**Parent-Spec**: `03-phase1-core-engine.md`
**Part**: 2 of 4
**Covers**: Exit criteria 1, 4, 12, 13, 16, 21
**Depends-On-Parts**: none — this part is self-contained

## Overview

**Project**: <repo>
**Phase**: 1 - Core engine (harness)

Phase 1 builds **the harness** as defined once and only once in pass-3 §4.1: `Ctx` + scheduler + journal + node registry + validator. Everything later in Pii (the coding harness, the IoT rule engine, the distiller) is a *graph* running on this one engine. If a second scheduler is ever written, the design has failed.

Phase 0 is DONE and committed: workspace skeleton, `pii-abi` (5 files, pure types), `pii-cli` (`pii --version`), `xtask` (`deps-check`, `lint-files`, `ci`), `quality-gate.sh`, workspace lints. Phase 1 adds three crates and grows two.

Source of truth (these documents ARE the contract; read them, do not re-derive):
- `_docs/06-research/001-pii-architecture-plan.md` §2.3 (PiiError), §2.4 (Ctx), §3.1 (graph model/Outcome), §3.2 (validate), §3.4 (redb tables, RunEvent, resume), §4.2 (cold start).
- `_docs/06-research/003-pii-architecture-plan-pass3.md` §4.2 (the two orki defects), §4.3 (the state machine — **the single most important section**), §4.4 (`Any` replaces `race`), §4.5 (`graph.batch.width`), §4.6 (V6/V9), §4.7 (the four-test diamond suite).

### Verified environment facts — read before writing any command

- Rust **1.95.0** at `/usr/local/cargo/bin`. **`cargo` is NOT on `PATH`** in gate shells. Every shell step begins `export PATH=/usr/local/cargo/bin:$PATH;`.
- crates.io reachable. Verified current versions: `redb 4.2.0`, `petgraph 0.8.3`, `toml 1.1.4`, `bitflags 2.13.1`, `parking_lot 0.12.5`, `blake3 1.8.7`, `dunce 1.0.5`, `tempfile 3.27.0`.
- **redb 4.2 API (verified on docs.rs — do not guess):** `Database::create(path)`, `Database::open(path)`, `db.begin_write() -> WriteTransaction`, `db.begin_read()` (**requires `use redb::ReadableDatabase`**), `txn.open_table(DEF)`, `table.insert(k, v)`, `table.get(k) -> Result<Option<AccessGuard<V>>>` (**requires `use redb::ReadableTable`**), `guard.value()`, `txn.commit()`. Tuple keys are supported: `impl<T0: Key, T1: Key> Key for (T0, T1)`. Import line to use verbatim: `use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};`
- `petgraph::algo::tarjan_scc` exists in 0.8.3.
- `toml 1.1.4` exposes `toml::Table`, `toml::Value` (`String|Integer|Float|Boolean|Datetime|Array|Table`), `toml::from_str`.
- `clippy.toml` **denies `std::path::Path::canonicalize`** (use `dunce::canonicalize`), **`std::process::Command::spawn`** (use `Command::output()`), **`std::time::SystemTime::now`** (use `std::time::Instant` deltas), **`std::env::var`**, and **`std::sync::Mutex`** (use `parking_lot::Mutex`).
- Workspace lints deny `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `todo`, `unimplemented`, `print_stdout`, `print_stderr`, and `missing_docs`. Test modules must open with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]`.

### Three scope decisions, stated explicitly rather than hidden

1. **The Phase-1 scheduler is fully synchronous.** No `tokio`, no `async-trait`, no `futures-core`. Concurrency comes from `std::thread::scope` over the parallel-safe partition of each ready batch. This is sufficient to prove every property pass-3 §4.7 demands (overlapping execution windows, race cancellation) and is deterministic, cheap to test, and has no runtime dependency. Async lands in Phase 2 with `pii-model`, where real I/O appears.
2. **No `llm`/`tool`/`agent`/`map`/`subgraph` node kinds.** Phase 1 ships four native kinds — `transform`, `route`, `emit`, `wait` — which is exactly what the four-test diamond suite, the cap-denial test, and the crash-resume test require. Pass-1 Phase-1 exit criterion 1 (`llm → tool → llm`) is therefore **deferred to Phase 2**, since `pii-model` and `pii-tools` are out of this phase's scope; it is replaced by `transform → wait → emit` end-to-end runs. `Outcome::Suspend` and `Terminal::Suspended` are implemented in the scheduler and journal and covered by a unit test with an in-crate test node, but no shipped node kind emits `Suspend` in Phase 1.
3. **Bounded cycles are validated but not executed.** `GraphSpec::validate()` implements rule V3 in full (Tarjan SCC + `loop_budget`) — that load-time safety property is the deliverable and it is unit-tested. The Phase-1 *scheduler* executes DAGs only: `pii graph run` on a spec containing a cycle returns `PiiError::Invalid("cyclic execution lands in Phase 2")`. Cyclic execution (edge re-arming, per-node generation counters) is a genuine design problem that would consume the whole implementation budget and is not needed by any Phase-1 test.

---

## Requirements

### Functional

#### F1 — `pii-abi` grows (still pure: serde, serde_json, thiserror, schemars — **no new dependencies**)

- **F1.1** `src/error.rs`: extend `PiiError` to the pass-1 §2.3 shape, minus the variants that need types Phase 1 does not have:
  ```rust
  pub enum PiiError {
      Invalid(String),                 // "invalid: {0}"
      Denied(String),                  // "capability denied: {0}"
      Cancelled,                       // "cancelled"
      Timeout { after_ms: u64 },       // "deadline exceeded after {after_ms}ms"
      BudgetExhausted(String),         // "budget exhausted: {0}"
      NotFound(String),                // "not found: {0}"
      Internal(String),                // "internal: {0}"
  }
  impl PiiError {
      pub fn is_retryable(&self) -> bool;  // Timeout only, in Phase 1
      pub fn is_terminal(&self)  -> bool;  // Denied | Invalid | BudgetExhausted | Cancelled
      pub fn is_cancelled(&self) -> bool;  // Cancelled
  }
  ```
  `is_retryable` and `is_terminal` must be mutually exclusive for every variant.
- **F1.2** `src/node.rs`: replace the Phase-0 placeholder `Outcome` with the pass-1 §3.1 form:
  ```rust
  pub struct NodeInput { pub input: serde_json::Value, pub payload: serde_json::Value,
                         pub nodes: serde_json::Value }
  pub enum Outcome {
      Next { port: PortName, value: serde_json::Value },
      Suspend { reason: SuspendReason, key: ResumeKey },
      Fail { error: String },
  }
  ```
  Keep the existing sync `NodeMeta` trait untouched.
- **F1.3** **New** `src/journal.rs` — the event-sourced types. They live in `pii-abi` (not `pii-store`, not `pii-graph`) because both of those crates need them and `pii-store` must never depend on `pii-graph`.
  ```rust
  pub enum RunStatus { Running, Suspended, Succeeded, Failed }
  pub struct RunHeader { pub run: RunId, pub graph: String, pub status: RunStatus,
                         pub seq: u64, pub dur_ms: u32 }
  pub enum SuspendReason { Approval { policy: String, prompt: String },
                           Timer { in_ms: u64 },
                           Event { topic: String },
                           HumanInput { channel: String, prompt: String } }
  pub struct ResumeKey(pub String);
  pub enum Terminal { Success { output: serde_json::Value },
                      Failure { error: String },
                      Suspended { key: ResumeKey } }
  pub enum RunEvent {
      Started      { graph: String, input_hash: String },
      TickStarted  { batch: Vec<NodeId>, width: u32 },
      NodeStarted  { node: NodeId, at_ms: u32 },
      NodeFinished { node: NodeId, port: PortName, value: serde_json::Value,
                     value_hash: String, at_ms: u32, dur_ms: u32 },
      NodeFailed   { node: NodeId, error: String, retryable: bool, attempt: u32 },
      NodeCancelled{ node: NodeId },
      NodeDead     { node: NodeId },
      Suspended    { node: NodeId, reason: SuspendReason, key: ResumeKey },
      Resumed      { key: ResumeKey, payload_hash: String },
      Finished     { terminal: Terminal, output_hash: String },
  }
  ```
  All types derive `Debug, Clone, PartialEq, Serialize, Deserialize`. **All time fields are milliseconds relative to run start, measured with `std::time::Instant`.** `std::time::SystemTime::now` must not appear anywhere in the workspace — that is what makes journal replay deterministic and it is enforced by `clippy.toml`.

#### F2 — `pii-core`: the capability context that actually enforces

New crate `crates/pii-core`. Deps: `pii-abi`, `serde`, `serde_json`, `thiserror`, `bitflags 2.13`, `parking_lot 0.12`, `dunce 1.0`. **Must not reach `redb`, `petgraph`, or `toml`.**

- **F2.1** `src/cap.rs`
  ```rust
  bitflags::bitflags! {
      #[derive(Debug, Clone, Copy, PartialEq, Eq)]
      pub struct CapKind: u32 {
          const FS_READ = 1; const FS_WRITE = 2; const NET = 4; const PROC = 8;
          const HW = 16; const SECRETS = 32; const MEMORY_WRITE = 64; const SPAWN = 128;
      }
  }
  /// Canonicalised allow/deny prefix sets.
  pub struct PathScope { allow: Vec<PathBuf>, deny: Vec<PathBuf> }
  impl PathScope {
      pub fn permits(&self, canonical: &Path) -> bool;  // deny wins; empty allow == permits nothing
      pub fn intersect(&self, other: &Self) -> Self;    // allow = paths in BOTH (prefix-wise);
                                                        // deny  = union of denies
  }
  pub struct StringScope(Vec<String>);   // prefix allowlist, used for net/proc/secrets in Phase 1
  pub struct CapSet { pub kinds: CapKind, pub fs_read: PathScope, pub fs_write: PathScope,
                      pub net_hosts: StringScope, pub proc_argv0: StringScope,
                      pub secrets: StringScope }
  impl CapSet {
      pub fn empty() -> Self;
      /// Attenuation ONLY. Never widening. kinds = self ∩ other; every scope intersected.
      pub fn intersect(&self, other: &Self) -> Self;
      pub fn parse_kind(s: &str) -> Result<CapKind, PiiError>;  // "fs_read"|"fs_write"|"net"|
                                                                // "proc"|"hw"|"secrets"|
                                                                // "memory_write"|"spawn"
  }
  ```
  `PathScope::intersect` rule: a path `p` is in the result's `allow` iff `self.permits(p) && other.permits(p)`. Implement as: `allow = { a ∈ self.allow ∪ other.allow : self.permits(a) && other.permits(a) }`, `deny = self.deny ∪ other.deny`.
- **F2.2** `src/cancel.rs` — `CancelToken { flag: Arc<AtomicBool>, parent: Option<Arc<CancelToken>> }`, with `new()`, `cancel()`, `is_cancelled()` (true if own flag or any ancestor flag is set), `child()`. `Clone` shares the same flag.
- **F2.3** `src/budget.rs`
  ```rust
  pub struct Budget { tokens: AtomicU64, tool_calls: AtomicU64, micro_usd: AtomicU64,
                      wall_ms: AtomicU64 }
  pub enum BudgetKind { Tokens, ToolCalls, MicroUsd, WallMs }
  pub enum BudgetShare { All, Fraction(u32 /* percent, 0..=100 */),
                         Absolute { tokens: u64, tool_calls: u64, micro_usd: u64, wall_ms: u64 } }
  impl Budget {
      pub fn unlimited() -> Self;   // u64::MAX in every field
      pub fn charge(&self, kind: BudgetKind, amount: u64) -> Result<u64, PiiError>;
      /// Atomically REMOVES the carved amount from self and returns a new Budget holding it.
      /// Never duplicates. Carving more than remains carves what remains.
      pub fn carve(&self, share: &BudgetShare) -> Budget;
      pub fn remaining(&self, kind: BudgetKind) -> u64;
  }
  ```
  `charge` returns `Err(PiiError::BudgetExhausted("<kind>"))` and leaves the counter at 0 when the amount exceeds what remains.
- **F2.4** `src/principal.rs` — `Principal { User, GraphNode { run: RunId, node: NodeId }, Plugin(String), Channel(String) }`; `TraceCtx { pub run: RunId, pub span: u64 }`.
- **F2.5** `src/observe.rs`
  ```rust
  pub enum ObsEvent { Run(pii_abi::RunEvent), Denial { principal: String, cap: String } }
  pub struct Metric { pub name: String, pub value: u64 }
  pub trait Observer: Send + Sync + 'static {
      fn event(&self, e: &ObsEvent);
      fn metric(&self, m: &Metric);
      fn flush(&self);
  }
  pub struct NullObserver;                        // all no-ops
  pub struct RecordingObserver { /* parking_lot::Mutex<Vec<..>> */ }
  impl RecordingObserver {
      pub fn metrics(&self) -> Vec<Metric>;
      pub fn max_metric(&self, name: &str) -> u64;
      pub fn events(&self) -> Vec<ObsEvent>;
  }
  pub struct MultiObserver(Vec<Arc<dyn Observer>>);
  ```
- **F2.6** `src/ctx.rs` — the enforcing context.
  ```rust
  pub struct Ctx { inner: Arc<CtxInner> }   // Clone is cheap; Ctx is Send + Sync
  struct CtxInner { cancel: CancelToken, started: Instant, deadline: Option<Instant>,
                    budget: Arc<Budget>, caps: Arc<CapSet>, principal: Principal,
                    trace: TraceCtx, obs: Arc<dyn Observer> }
  impl Ctx {
      pub fn root(caps: CapSet, budget: Budget, obs: Arc<dyn Observer>, trace: TraceCtx) -> Self;
      /// Accessors RETURN Result. The check happens HERE, not in a doc comment.
      pub fn fs(&self) -> Result<FsCap<'_>, PiiError>;    // Denied unless FS_READ|FS_WRITE
      pub fn net(&self) -> Result<NetCap<'_>, PiiError>;  // Phase 1: Denied unless NET; no methods
      pub fn proc(&self) -> Result<ProcCap<'_>, PiiError>;// Phase 1: Denied unless PROC; no methods
      /// Attenuation only: caps = self.caps ∩ narrow; budget CARVED from self, not duplicated;
      /// cancel is a child token; principal is replaced; span is incremented.
      pub fn child(&self, principal: Principal, narrow: &CapSet, share: BudgetShare) -> Ctx;
      pub fn cancelled(&self) -> bool;
      pub fn elapsed_ms(&self) -> u32;
      pub fn check(&self) -> Result<(), PiiError>;  // Cancelled, then Timeout{after_ms}
      pub fn observer(&self) -> &Arc<dyn Observer>;
      pub fn caps(&self) -> &CapSet;
      pub fn budget(&self) -> &Budget;
  }
  ```
- **F2.7** `src/fscap.rs` — there is no unchecked path.
  ```rust
  pub struct FsCap<'a> { ctx: &'a Ctx }
  impl FsCap<'_> {
      pub fn read(&self, p: &Path) -> Result<Vec<u8>, PiiError>;
      pub fn write(&self, p: &Path, bytes: &[u8]) -> Result<(), PiiError>;
      pub fn append(&self, p: &Path, bytes: &[u8]) -> Result<(), PiiError>;
  }
  ```
  Every method follows exactly this order, and the order is the security property:
  1. `ctx.check()?` (cancel/deadline).
  2. Require the matching `CapKind` (`FS_READ` for read, `FS_WRITE` for write/append) → `PiiError::Denied("fs_read")` / `Denied("fs_write")`.
  3. **Canonicalise with `dunce::canonicalize`.** If the target does not exist, canonicalise its *parent* directory and re-join the file name.
  4. **Re-check the scope on the canonical path** (`PathScope::permits`). The classic escape is `_tmp/link -> /etc`; a check on the pre-resolution path is theatre. On failure emit `ObsEvent::Denial` on the observer and return `PiiError::Denied(...)`.
  5. Only then perform the I/O.

  Doc comment on `FsCap` must state, verbatim in substance: *`Ctx` is the logical layer. It is not a security boundary against native code in the same process; kernel confinement arrives in Phase 2.*

#### F3 — `pii-store`: THE ONLY crate that knows redb

New crate `crates/pii-store`. Deps: `pii-abi`, `pii-core`, `serde`, `serde_json`, `thiserror`, `redb 4.2`, `blake3 1.8`. Dev-dep: `tempfile 3`.

- **F3.1** `src/tables.rs` — pass-1 §3.4, subset needed by Phase 1. Values are **`serde_json` bytes** (postcard is a later optimisation; it is not needed to make the transaction property hold and it adds an encoding failure mode a workhorse pass does not need).
  ```rust
  const META:          TableDefinition<&str, u64>        = TableDefinition::new("meta");
  const GRAPH_BODY:    TableDefinition<&str, &[u8]>      = TableDefinition::new("graph_body");
  const RUN:           TableDefinition<&str, &[u8]>      = TableDefinition::new("run");
  const RUN_JOURNAL:   TableDefinition<(&str, u64), &[u8]> = TableDefinition::new("run_journal");
  const RUN_SNAPSHOT:  TableDefinition<(&str, u64), &[u8]> = TableDefinition::new("run_snapshot");
  const RUN_BY_STATUS: TableDefinition<(&str, &str), ()> = TableDefinition::new("run_by_status");
  const RESUME_INDEX:  TableDefinition<&str, &str>       = TableDefinition::new("resume_index");
  ```
- **F3.2** `src/codec.rs` — `to_bytes<T: Serialize>`, `from_bytes<T: DeserializeOwned>` over `serde_json`, mapping errors to `PiiError::Internal`; `pub fn hash(bytes: &[u8]) -> String` = `blake3::hash(bytes).to_hex().to_string()`.
- **F3.3** `src/lib.rs` — `pub struct Store { db: redb::Database }` with exactly this API:
  ```rust
  impl Store {
      pub fn open(path: &Path) -> Result<Self, PiiError>;      // Database::create (creates or opens)
      pub fn next_run_id(&self) -> Result<RunId, PiiError>;    // META["run_counter"] += 1 in one
                                                               // write txn -> RunId("r{n:012}")
      pub fn put_graph(&self, spec_json: &[u8]) -> Result<String, PiiError>;  // key = blake3 hex
      pub fn get_graph(&self, hash: &str) -> Result<Option<Vec<u8>>, PiiError>;
      /// THE transaction that matters (pass-1 §3.4, pass-3 §4.8): appends the event at
      /// `header.seq`, writes the header, and repoints run_by_status - ALL in ONE
      /// redb write transaction, committed before returning.
      pub fn append_event(&self, header: &mut RunHeader, event: &RunEvent)
          -> Result<u64, PiiError>;
      pub fn get_run(&self, run: &RunId) -> Result<Option<RunHeader>, PiiError>;
      pub fn read_journal(&self, run: &RunId) -> Result<Vec<(u64, RunEvent)>, PiiError>;
      pub fn put_snapshot(&self, run: &RunId, seq: u64, bytes: &[u8]) -> Result<(), PiiError>;
      pub fn latest_snapshot(&self, run: &RunId) -> Result<Option<(u64, Vec<u8>)>, PiiError>;
      pub fn put_resume_key(&self, key: &ResumeKey, run: &RunId) -> Result<(), PiiError>;
      pub fn runs_by_status(&self, status: RunStatus) -> Result<Vec<RunId>, PiiError>;
  }
  ```
  `append_event` semantics: writes `RUN_JOURNAL[(run, header.seq)]`, removes `RUN_BY_STATUS[(old_status, run)]`, inserts `RUN_BY_STATUS[(new_status, run)]`, writes `RUN[run]`, then `header.seq += 1`, then commits. It returns the seq the event was written at.
- **F3.4** **Single-writer rule**: a `WriteTransaction` is opened and committed inside one method body. It is never stored in a struct, never returned, and never held across a node dispatch. Put this as a `//!` note at the top of `lib.rs`.

#### F4 — `pii-graph`: spec, validator, scheduler, journal, resume

New crate `crates/pii-graph`. Deps: `pii-abi`, `pii-core`, `pii-store`, `serde`, `serde_json`, `thiserror`, `petgraph 0.8`, `toml 1.1`, `parking_lot 0.12`, `blake3 1.8`. Dev-dep: `tempfile 3`.

- **F4.1** `src/spec.rs`
  ```rust
  pub enum EdgeKind { Data, Order }
  pub enum JoinPolicy { All, Any { cancel_losers: bool },
                        Quorum { k: u16, cancel_losers: bool }, AllSettled }
  pub struct Edge { pub from: NodeId, pub port: PortName, pub to: NodeId, pub kind: EdgeKind }
  pub struct RetryPolicy { pub max: u32 }                       // default 0
  pub struct GraphLimits { pub max_steps: u32, pub max_wall_ms: u64 }  // defaults 1000 / 600_000
  pub struct NodeSpec { pub id: NodeId, pub kind: String, pub join: JoinPolicy,
                        pub loop_budget: Option<u32>, pub retry: RetryPolicy,
                        pub caps: CapKind, pub cfg: serde_json::Value }
  pub struct GraphSpec { pub name: String, pub caps: CapSet, pub nodes: Vec<NodeSpec>,
                         pub edges: Vec<Edge>, pub entry: NodeId, pub limits: GraphLimits }
  impl GraphSpec {
      pub fn node(&self, id: &NodeId) -> Option<&NodeSpec>;
      pub fn inbound(&self, id: &NodeId, kind: EdgeKind) -> Vec<usize>;   // edge indices
      pub fn outbound(&self, id: &NodeId) -> Vec<usize>;
      pub fn to_json(&self) -> Result<Vec<u8>, PiiError>;
      pub fn from_json(bytes: &[u8]) -> Result<Self, PiiError>;
  }
  ```
  `JoinPolicy` parses from a string: `"all"` (default), `"any"`, `"any:cancel"`, `"quorum:K"`, `"quorum:K:cancel"`, `"allsettled"`. Anything else → `PiiError::Invalid`.
- **F4.2** `src/toml_load.rs` — `pub fn load_toml(src: &str) -> Result<GraphSpec, PiiError>`. Grammar:
  ```toml
  name  = "diamond"
  caps  = ["fs_write"]        # optional, default []
  entry = "entry"             # optional, default = first [[node]]
  [limits]
  max_steps   = 1000          # optional
  max_wall_ms = 600000        # optional

  [[node]]
  id   = "entry"
  kind = "transform"
  # reserved optional keys: join, loop_budget, retry = { max = 2 }, caps = ["fs_write"]
  # EVERY OTHER key in the table becomes part of `cfg`
  expr = "$.input"

  [[edge]]
  from = "entry"
  port = "ok"
  to   = "a"
  kind = "data"               # optional: "data" (default) | "order"
  ```
  Implementation: parse the document as `toml::Table`; for each `[[node]]` table pull the reserved keys out by name, then convert the remaining keys to `cfg` with an explicit `fn toml_to_json(v: &toml::Value) -> Result<serde_json::Value, PiiError>` that maps `String/Integer/Float/Boolean/Array/Table` and returns `Invalid("toml datetime unsupported")` for `Datetime`. Node `caps` default to the kind's `required_caps()`.
- **F4.3** `src/expr.rs` — the whole expression language, deliberately tiny.
  ```rust
  /// Context object is  { "input": <run input>, "payload": <join payload>,
  ///                      "nodes": { "<node id>": <value>, .. } }
  pub fn eval(ctx: &serde_json::Value, expr: &str) -> Result<serde_json::Value, PiiError>;
  ```
  Accepts `$.<key>(.<key>)*` only (object member access, no arrays, no functions). An unresolvable path returns `Ok(Value::Null)`; a malformed expression (not starting with `$.`) returns `PiiError::Invalid`.
  Helper: `pub fn eval_or_literal(ctx, v: &serde_json::Value) -> Result<Value, PiiError>` — if `v` is a string starting with `$.`, evaluate it; otherwise return it unchanged. Node configs use this uniformly.
- **F4.4** `src/node.rs` — the harness ABI.
  ```rust
  pub trait GraphNode: Send + Sync {
      fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError>;
  }
  pub trait NodeFactory: Send + Sync {
      fn kind(&self) -> &'static str;
      fn ports(&self, cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError>;
      fn required_caps(&self) -> CapKind;
      fn effects(&self) -> Effects;
      fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError>;
  }
  pub struct NodeRegistry { /* BTreeMap<&'static str, Arc<dyn NodeFactory>> */ }
  impl NodeRegistry {
      pub fn builtin() -> Self;   // transform, route, emit, wait
      pub fn factory(&self, kind: &str) -> Result<&Arc<dyn NodeFactory>, PiiError>;
  }
  ```
  `ports()` takes `cfg` because `route`'s ports are data-defined. `effects()` and `required_caps()` are static per kind so the validator and the scheduler can consult them without building.
- **F4.5** `src/nodes/*.rs` — the four built-in kinds.

  | kind | cfg keys | ports | effects | required caps | behaviour |
  |---|---|---|---|---|---|
  | `transform` | `expr` (string) **or** `value` (any) | `ok`, `err` | `Pure` | none | `Next{ok, eval_or_literal(...)}`; a malformed expr → `Next{err, {"error": ..}}` |
  | `route` | `on` (expr string), `rules` (array of `{eq?: any, port: string}`) | one per distinct `rules[].port`, in declaration order | `Pure` | none | evaluate `on`, take the first rule whose `eq` equals the value (a rule with no `eq` is the fallback and must be last); `Next{that port, evaluated value}`. No matching rule → `PiiError::Invalid` |
  | `emit` | `path` (expr-or-literal string), `mode` (`"write"`\|`"append"`, default `write`), `value` (expr-or-literal) | `ok`, `err` | `WritesFs` | `FS_WRITE` | serialise `value` to a string (JSON strings are written raw, without quotes), then `ctx.fs()?.write/append(path, bytes)`; on `Err` return it (do **not** swallow into `err`) so denials surface as `NodeFailed` |
  | `wait` | `timer_ms` (u64) | `fired`, `timeout` | `Pure` | none | sleep in 5 ms slices, checking `ctx.cancelled()` before each slice; on cancel return `Err(PiiError::Cancelled)`; otherwise `Next{fired, input.payload.clone()}` |

- **F4.6** `src/validate.rs` — `pub fn validate(spec: &GraphSpec, reg: &NodeRegistry) -> Result<Vec<String>, Vec<String>>` (`Ok(warnings)` / `Err(errors)`). Every message starts with its rule code so it is greppable.
  - **V0** structural: duplicate node id; edge referencing an unknown node id; unknown node `kind`; `entry` not a declared node. → error `V0: ...`
  - **V1** port existence: every `Data`/`Order` edge's `port` must be in `factory.ports(&node.cfg)?` of the source node. → error `V1: node '<id>' has no port '<port>'`
  - **V2** cap subset: `node.caps ∪ factory.required_caps()` ⊆ `spec.caps.kinds`. → error `V2: node '<id>' requires capability '<cap>' not granted by the graph`
  - **V3** bounded cycles: build a `petgraph::graph::DiGraph<(), ()>` over all edges, run `petgraph::algo::tarjan_scc`. Any SCC with more than one node, **or** a single node with a self-edge, that contains no node with `loop_budget.is_some()` → error `V3: cycle [a, b, ...] has no loop_budget`.
  - **V4** deferred (schema compatibility). Do not implement.
  - **V5** unreachable from `entry` → **warning** `V5: node '<id>' is unreachable from entry`.
  - **V6** statically unsatisfiable `All` join. This is the routed-diamond deadlock caught before it runs, and it needs the **transitive** formulation — the join in `entry → route → {a,b} → join` receives its edges from `a` and `b`, not from the router, so a rule that only looks one hop back cannot pass pass-3 Test C. Algorithm (skip entirely if V3 found a cycle):
    ```
    // constraints[e] : BTreeMap<NodeId /*router*/, BTreeSet<PortName>>
    // A node u is "branching" iff its outbound Data edges use >1 distinct port.
    for u in topological_order(spec):
        c_u = union_per_router(constraints[e] for e in inbound_data(u))   // empty for entry
        for e in outbound_data(u):
            constraints[e] = c_u
            if branching(u): constraints[e].insert(u, {e.port})           // overwrite, not union
    for j in nodes where j.join == All:
        for each pair (e1, e2) of inbound_data(j):
            if ∃ r present in BOTH constraints[e1] and constraints[e2]
               with constraints[e1][r] ∩ constraints[e2][r] == ∅:
                error "V6: join '<j>' can never be satisfied: '<e1.from>' and '<e2.from>' \
                       are on mutually exclusive ports of '<r>'"
    ```
    `union_per_router` unions the port sets for routers present in either map (a node reachable two ways is genuinely unconstrained on that router). Absence of a router from a map means unconstrained.
  - **V9** `Any`/`Quorum` require ≥ 2 inbound Data edges; `Quorum{k}` additionally requires `k ≥ 1` and `k ≤ inbound_data_count`. → error `V9: node '<id>' uses a multi-arm join with <n> inbound data edges`
- **F4.7** `src/sched/state.rs` — the state machine, **transcribed from pass-3 §4.3**.
  ```rust
  pub enum EdgeState { Pending, Fired { value: serde_json::Value }, Dead }
  pub enum Readiness { NotReady, Ready, DeadNode, UnsatisfiableJoin }
  pub struct RunState { pub edges: Vec<EdgeState>, pub executed: BTreeSet<NodeId>,
                        pub dead: BTreeSet<NodeId>, pub cancelled: BTreeSet<NodeId>,
                        pub failed: BTreeSet<NodeId> }
  pub fn readiness(spec: &GraphSpec, st: &RunState, n: &NodeId) -> Readiness;
  ```
  `readiness` is total. Cases, in this order:
  1. If any inbound **Order** edge is `Pending` → `NotReady`. (Order edges wait for settle.)
  2. **If the node has zero inbound Data edges → `Ready`.** This case must be written explicitly and first among the join arms. `all()` over an empty slice is `true`, so a literal transcription of pass-3's pseudocode reports the entry node as `DeadNode` and the graph never starts. This is the single most likely implementation bug in the whole phase.
  3. `All` → any `Pending` ⇒ `NotReady`; else all `Dead` ⇒ `DeadNode`; else any `Dead` ⇒ **`UnsatisfiableJoin`**; else `Ready`.
  4. `Any{..}` → any `Fired` ⇒ `Ready`; else all `Dead` ⇒ `DeadNode`; else `NotReady`.
  5. `Quorum{k,..}` → count(`Fired`) ≥ k ⇒ `Ready`; else count(`Fired`) + count(`Pending`) < k ⇒ `UnsatisfiableJoin`; else `NotReady`.
  6. `AllSettled` → any `Pending` ⇒ `NotReady`; else all `Dead` ⇒ `DeadNode`; else `Ready`.

  **Edge transitions on completion.** When node `n` completes on port `P`:
  ```
  for e in outbound(n):
      e.state = if e.kind == Order || e.port == P { Fired { value } } else { Dead }
  ```
  A node that failed or was cancelled marks **all** its outbound Data edges `Dead` and its outbound Order edges `Fired { Null }`.

  **Dead propagation**, run to a fixpoint before every readiness sweep:
  ```
  repeat until no change:
      for each unexecuted node n with >=1 inbound Data edge, all of them Dead:
          mark n dead; journal NodeDead; metric graph.node.dead += 1
          for e in outbound(n, Data):  e.state = Dead
          for e in outbound(n, Order): e.state = Fired { Null }   // Order edges SURVIVE dead
                                                                  // branches. That is the entire
                                                                  // point of Order.
  ```
- **F4.8** `src/sched/mod.rs` — the tick. **`.drain()`, never `.pop()`, never `.next()`** (orki defect 1).
  ```
  loop:
      if step_count > limits.max_steps      -> Failure("max_steps exceeded")
      if ctx.elapsed_ms() > limits.max_wall_ms -> Failure("max_wall_ms exceeded")
      propagate_dead_to_fixpoint()
      if any unexecuted node has readiness == UnsatisfiableJoin:
          metric graph.join.unsatisfiable += 1
          journal Finished{Failure}; return Failure(
              "join can never be satisfied: node '<id>'")
      batch = ALL unexecuted, non-dead nodes with readiness == Ready
      if batch.is_empty():  return terminal_determination()
      journal TickStarted { batch, width }; metric graph.batch.width = batch.len()
      (par, ser) = batch.partition(|n| factory(n).effects().is_parallel_safe())
      run_par(par);  for n in ser { run_one(n) }
  ```
  `terminal_determination()`: if at least one node executed and every unexecuted node is `Dead` or unreachable → `Terminal::Success { output }`, where **`output` is the `NodeFinished` value of the executed node with no outbound Data edge on the port it took, taking the latest by seq if several**. If some unexecuted node is still `NotReady` → `Terminal::Failure("no progress: node '<id>' never became ready")`. A node returning `Outcome::Fail`/`Err` that is not retryable and whose failure leaves the graph unable to reach a terminal → the run ends `Failure` with that node's error.

  **Never a hang.** Every one of these branches returns. There is no `loop` without a terminating arm.
- **F4.9** `src/sched/dispatch.rs` — concurrency and `cancel_losers`. The borrow split matters; write it exactly this way:
  ```rust
  struct Env<'a> { spec: &'a GraphSpec, reg: &'a NodeRegistry, built: &'a BTreeMap<NodeId, Box<dyn GraphNode>>,
                   ctx: &'a Ctx, tokens: &'a BTreeMap<NodeId, CancelToken>,
                   bb: &'a parking_lot::RwLock<Blackboard> }
  // Env is shared &-only with the worker threads. RunState is mutated ONLY by the
  // receiving loop, and is therefore never captured by a thread.
  let (tx, rx) = std::sync::mpsc::channel();
  std::thread::scope(|s| {
      for n in par { let tx = tx.clone(); s.spawn(move || { let _ = tx.send((n.clone(), dispatch(&env, &n))); }); }
      drop(tx);
      for (node, result) in rx {
          apply(&mut state, &node, result);      // journal + edge transitions
          cancel_losers(&env, &state);           // AFTER every single result
      }
  });
  for n in ser { let r = dispatch(&env, &n); apply(&mut state, &n, r); }
  ```
  `cancel_losers(env, state)`: for every node `j` whose join is `Any{cancel_losers:true}` or `Quorum{cancel_losers:true}` and whose `readiness` is now `Ready`, call `token.cancel()` on every node that feeds `j` through a still-`Pending` inbound Data edge. Because `CancelToken` is an `Arc<AtomicBool>`, this reaches the running thread.

  Per-node dispatch builds an attenuated child context:
  `let node_ctx = env.ctx.child(Principal::GraphNode{run, node}, &node_capset, BudgetShare::All);` with `node_ctx`'s cancel token being the one registered in `env.tokens` for that node. Journal `NodeStarted{at_ms}` **before** running (so a `MUTATES_EXTERNAL` node interrupted mid-flight is visible as "may have executed"), then run, then journal `NodeFinished`/`NodeFailed`/`NodeCancelled`/`Suspended`.

  **Retry**: on `Err(e)` where `e.is_retryable()` and `attempt < spec.retry.max`, journal `NodeFailed{retryable:true, attempt}` and re-run immediately. On `e.is_cancelled()`, journal `NodeCancelled` (**not** `NodeFailed`) and never retry.
- **F4.10** `src/blackboard.rs` — `pub struct Blackboard { pub nodes: BTreeMap<String, serde_json::Value>, pub input: serde_json::Value }`. Reads are memory-only. Writes go to the in-memory blackboard **and** append `NodeFinished` in the same redb write transaction that advances `RunHeader` (pass-3 §4.8). Snapshot (`Store::put_snapshot` with `serde_json` bytes of the `Blackboard`) every 32 journal events and unconditionally on `Suspend`. Do **not** make the blackboard per-key write-through; orki measured a 22× penalty for that and the granularity, not the durability, was the cause.

  `NodeInput` construction: `input` = the run input, `nodes` = the blackboard's node map, and **`payload` = a JSON object keyed by source node id holding the value of every `Fired` inbound Data edge**. A node with no inbound Data edges gets `payload = input.clone()`.
- **F4.11** `src/run.rs` — `pub fn run(store: &Store, spec: &GraphSpec, reg: &NodeRegistry, ctx: &Ctx, run: &RunId, input: Value) -> Result<Terminal, PiiError>` and `pub fn resume(store: &Store, reg: &NodeRegistry, ctx: &Ctx, run: &RunId) -> Result<Terminal, PiiError>`.

  **Resume** = load the `GraphSpec` from `GRAPH_BODY` by the hash in `RunHeader`, then **replay the journal from seq 0** to rebuild `RunState` and `Blackboard`:
  - `NodeFinished` → mark executed, write the value into the blackboard, apply the edge transitions for the taken port.
  - `NodeDead` → mark dead, propagate.
  - `NodeCancelled` / non-retryable `NodeFailed` → mark, kill outbound Data edges.
  - `NodeStarted` **without** a matching `NodeFinished` → that node is *not* marked executed and will be re-run. (Pass-1 §3.4: a run interrupted mid-node replays that one node.)

  Then continue the normal tick loop, appending to the same journal. **A completed node is never re-invoked**, because `NodeFinished` carries its value. Phase 1 replays the whole journal and does not read snapshots on resume; snapshots are still written and `Store::latest_snapshot` is still exercised by a test. Note this simplification in the module doc.

#### F5 — `pii-cli`: `pii graph {lint|run|resume|trace}`

`pii-cli` gains deps `pii-abi`, `pii-core`, `pii-graph`, `pii-store`, `serde_json`. It keeps its crate-level `#![allow(clippy::print_stdout, clippy::print_stderr)]`.

- **F5.1** argv dispatch in `main.rs` must branch on `args[1]` **before touching anything else** — no config load, no redb open, no allocation beyond argv (pass-1 §4.2). `--version`/`-V` returns immediately.
- **F5.2** Commands:
  ```
  pii --version
  pii graph lint   <file.toml>
  pii graph run    <file.toml> --db <path> [--run-id <id>] [--input <json>]
                               [--allow-write <dir>]... [--no-validate]
  pii graph resume <run_id> --db <path> [--allow-write <dir>]...
  pii graph trace  <run_id> --db <path> [QUERY]
  ```
  `QUERY` is exactly one of: (none) → ndjson dump of every journal event as `{"seq":<u64>,"event":<RunEvent>}`; `--count-finished <node>` → integer; `--node-status <node>` → one of `finished|failed|cancelled|dead|pending`; `--max-batch-width` → integer; `--terminal` → `success|failure|suspended|running`; `--output` → the terminal output JSON; `--overlapped <a>,<b>` → `yes|no`, computed as `a.at_ms < b.at_ms + b.dur_ms && b.at_ms < a.at_ms + a.dur_ms` from the two `NodeFinished` events.
- **F5.3** Exit codes: `0` success, `1` run ended `Failure`, `2` usage error, `3` validation error (`lint` failure), `4` run ended `Suspended`.
- **F5.4** Root capabilities for `run`/`resume`: `fs_read` allow = `[cwd]`; `fs_write` allow = `[cwd/_tmp]` plus every `--allow-write <dir>`; all other `CapKind` bits clear. The graph's declared `caps` are **intersected** with this root, never unioned. `--input` defaults to `{}`.
- **F5.5** `run`/`resume` install an ndjson observer on stdout emitting `{"metric":"<name>","value":<u64>}` per metric and `{"seq":<n>,"event":<RunEvent>}` per journal event, then a final `{"terminal":<Terminal>}` line. `lint` prints errors and warnings to **stderr**, one per line, each beginning with its `V<n>` code.
- **F5.6** `run` calls `validate()` and exits 3 on error unless `--no-validate` is passed. `--no-validate` exists so the runtime `UnsatisfiableJoin` path (pass-3 Test C) can be exercised; document it as such in the usage string.

#### F6 — `xtask` gains layering enforcement and the cold-start bench gate

- **F6.1** `xtask/src/deps.rs`: keep `check_abi_purity` as-is and add
  ```rust
  const LAYER_RULES: &[(&str, &[&str])] = &[
      ("pii-abi",   &["tokio","reqwest","redb","wasmtime","axum","hyper","rustls",
                      "petgraph","toml","parking_lot","blake3","dunce","bitflags"]),
      ("pii-core",  &["redb","petgraph","toml","reqwest","wasmtime","tokio"]),
      ("pii-store", &["petgraph","toml","reqwest","wasmtime","tokio"]),
  ];
  pub(crate) fn check_layering(metadata: &serde_json::Value) -> Result<(), Vec<String>>;
  ```
  Same BFS over `resolve.nodes` as `check_abi_purity`, one rule set per named crate. `deps-check` runs both and reports every violation. `ABI_ALLOWED_DIRECT` is **unchanged** — `pii-abi` gains no dependency in this phase.
- **F6.2** `xtask/src/bench.rs` + `cargo xtask bench-gate [--budget-ms <n>]` (default 20):
  - Requires `./target/release/pii` to exist; error out cleanly if not.
  - 10 warmup + 50 measured invocations of `./target/release/pii --version` via `std::process::Command::output()` (**never `.spawn()`** — `clippy.toml` denies it), timed with `Instant`.
  - p95 = the sorted sample at index `ceil(0.95 * n) - 1`.
  - Writes `_tmp/bench/version-startup.json`: `{"samples":50,"p50_ms":<f64>,"p95_ms":<f64>,"budget_ms":<n>}` (creating `_tmp/bench/`).
  - Prints `p95_ms=<x> budget_ms=<n>` and exits 1 when `p95_ms > budget_ms`.
- **F6.3** `cargo xtask ci` runs `deps-check`, `lint-files`, `bench-gate` (the last skipped with a printed note if the release binary is absent).
- **F6.4** Release profiles per pass-1 §4.2 are already present from Phase 0; do not change them.

#### F7 — Example graphs

Create `examples/` at the repo root with exactly these files, in valid TOML (multi-line tables — pass-1 §3.6 writes `id = "x"; kind = "y"` on one line, which is **not valid TOML**; do not copy that formatting).

`diamond.toml` (Test A) — `entry(transform, expr="$.input")` → `a(wait, timer_ms=150)` and `b(wait, timer_ms=150)` → `join(transform, join="all", expr="$.payload")`; edges `entry:ok→a`, `entry:ok→b`, `a:fired→join`, `b:fired→join`.

`routed-diamond.toml` (Test B) — `entry(transform, expr="$.input")` → `r(route, on="$.input.branch", rules=[{eq="a",port="a"},{port="b"}])` → `a(wait,80)` / `b(wait,80)` → `join(transform, join="allsettled", expr="$.payload")`.

`unsatisfiable.toml` (Test C) — byte-identical to `routed-diamond.toml` except `join` has `join = "all"`.

`race.toml` (Test D) — `fanout(transform, expr="$.input")` → `slow(wait, timer_ms=5000)` and `fast(wait, timer_ms=50)` → `use(transform, join="any:cancel", expr="$.payload")`.

`crash-resume.toml` — `caps=["fs_write"]`; `a(emit, path="$.input.log", mode="append", value="a\n")` → `b(wait, timer_ms=6000)` → `c(emit, path="$.input.log", mode="append", value="c\n")`.

`emit-path.toml` — `caps=["fs_write"]`; single node `w(emit, path="$.input.path", mode="write", value="x")`.

Lint negatives: `lint-bad-port.toml` (edge on port `nope` out of a `transform`) → V1; `lint-cap-escalation.toml` (graph `caps = []`, one `emit` node) → V2; `lint-unbounded-cycle.toml` (`a→b→a`, no `loop_budget`) → V3; `lint-one-armed-race.toml` (`x→y` with `y.join = "any"`) → V9.

### Non-Functional

- **N1** `./quality-gate.sh` (fmt --check, clippy `-D warnings`, test) passes on a clean checkout with no manual steps beyond the `PATH` export.
- **N2** No `unsafe` anywhere. Every new crate root carries `#![forbid(unsafe_code)]` and `[lints] workspace = true`.
- **N3** No source file exceeds **450 effective lines** (`cargo xtask lint-files`). Split into module folders (`sched/`, `nodes/`) rather than growing a file.
- **N4** `pii-abi` gains **no new dependency** and no new transitive reach. `pii-core` never reaches `redb`. Only `pii-store` knows redb. Machine-enforced by `xtask deps-check`.
- **N5** `pii --version` p95 < 20 ms in the release profile, gated in CI. `main` must not open redb, read config, or read the environment on that path.
- **N6** No `async`, no `.await`, no `tokio`, no `async-trait`, no `futures-core` anywhere in Phase 1.
- **N7** No `SystemTime::now` anywhere. All journal timestamps are `Instant`-derived millisecond offsets from run start, which is what makes replay deterministic.
- **N8** Every test that touches the filesystem writes under `_tmp/`. Rust unit tests use `tempfile::TempDir` rooted there (`tempfile::Builder::new().tempdir_in("_tmp")`). Nothing is written to `/tmp`.
- **N9** No node dispatch happens while a redb `WriteTransaction` is open.

---

## Scope (Part 2 of 4)

Create or modify **only** these artifacts:

- `crash-resume.toml`
- `diamond.toml`
- `quality-gate.sh`
- `race.toml`
- `routed-diamond.toml`

Anything else in the Architecture belongs to another part. Creating it
here duplicates that part's work and makes the split pointless.

Part 1 owns:
- `emit-path.toml`
- `lint-bad-port.toml`
- `lint-unbounded-cycle.toml`

Part 3 owns:
- `crash-resume.toml`
- `lint-one-armed-race.toml`
- `unsatisfiable.toml`

Part 4 owns:
- `lint-cap-escalation.toml`
- `routed-diamond.toml`
- `version-startup.json`

## Architecture (full system — context, not a build list)

```

├── Cargo.toml                    # + members pii-core, pii-store, pii-graph
│                                 # + workspace.dependencies: bitflags 2.13, parking_lot 0.12,
│                                 #   petgraph 0.8, redb 4.2, toml 1.1, blake3 1.8, dunce 1.0,
│                                 #   tempfile 3 (dev)
├── examples/                     # 9 .toml graphs (F7) - the behavioural fixtures
├── xtask/src/{deps,bench}.rs     # + check_layering, + bench-gate
└── crates/
    ├── pii-abi/     src/{lib,error,ids,effects,node,journal}.rs      [+1 file, NO new deps]
    ├── pii-core/    src/{lib,cap,cancel,budget,principal,observe,ctx,fscap}.rs
    ├── pii-store/   src/{lib,tables,codec}.rs
    ├── pii-graph/   src/{lib,spec,toml_load,expr,node,registry,blackboard,validate,run}.rs
    │                src/sched/{mod,state,dispatch}.rs
    │                src/nodes/{mod,transform,route,emit,wait}.rs
    │                tests/diamond.rs        # the four-test suite (pass-3 §4.7)
    └── pii-cli/     src/{main,graph}.rs

Dependency DAG (enforced by `cargo xtask deps-check`):

    pii-cli  ──► pii-graph, pii-store, pii-core, pii-abi
    pii-graph──► pii-store, pii-core, pii-abi, petgraph, toml, blake3, parking_lot
    pii-store──► pii-core, pii-abi, redb, blake3          ◄── THE ONLY redb EDGE
    pii-core ──► pii-abi, bitflags, parking_lot, dunce
    pii-abi  ──► serde, serde_json, thiserror, schemars    [HARD CEILING, unchanged]

THE HARNESS (pass-3 §4.1) - exactly five things, one scheduler, no exceptions:

  (1) Ctx            pii-core::ctx      caps ∩ / budget carve / cancel / trace
  (2) scheduler      pii-graph::sched   readiness -> drain-all -> effects partition
  (3) journal        pii-abi::journal + pii-store   event-sourced truth, resume
  (4) node registry  pii-graph::registry            kind -> NodeFactory (native only, Phase 1)
  (5) validator      pii-graph::validate            V0,V1,V2,V3,V5,V6,V9

One scheduler tick:

   ┌─ propagate_dead_to_fixpoint ────────────────────────────────────────────┐
   │  all inbound Data edges Dead  ⇒  node Dead                              │
   │      outbound Data  -> Dead                                             │
   │      outbound Order -> Fired   ◄── Order edges SURVIVE dead branches    │
   └─────────────────────────────────────────────────────────────────────────┘
                │
   any readiness == UnsatisfiableJoin ──► Terminal::Failure          (LOUD, ≤1 tick)
                │                          "join can never be satisfied"
                ▼
   batch = ALL nodes with readiness == Ready      ◄── .drain(), never .pop()  [orki bug 1]
                │                                     journal TickStarted{width}
                │                                     metric graph.batch.width
                ▼
   partition by Effects::is_parallel_safe()
        Pure | ReadsFs  ──► std::thread::scope + mpsc, results applied as they arrive
        │                        └─► after EACH result: cancel_losers()  [JoinPolicy::Any]
        WritesFs | MutatesExternal ──► sequential
                │
                ▼
   apply(outcome):  port P fires;  every other outbound Data edge -> Dead
                    NodeFinished appended in the SAME redb write txn that advances RunHeader
                    snapshot every 32 events, and unconditionally on Suspend

Readiness is TOTAL. The empty-inbound case is checked FIRST:
  a node with zero inbound Data edges is Ready.  (`all()` over an empty slice is `true`,
  so a literal transcription of the pass-3 pseudocode reports the entry node DeadNode.)
```

---

> **Note**: the Architecture above describes the FULL system, including
> artifacts owned by other parts. Build only what Scope lists.

## TDD Contract

| Test Name | Input | Expected Output |
|-----------|-------|-----------------|
| `pii_error_retryable_and_terminal_partition` | every `PiiError` variant | `is_retryable()` and `is_terminal()` are never both true; `Timeout` retryable; `Denied`/`Invalid`/`BudgetExhausted`/`Cancelled` terminal |
| `capset_intersect_never_widens` | `a = {FS_READ}`, `b = {FS_READ, FS_WRITE, NET}` | `a.intersect(&b).kinds == FS_READ` and `b.intersect(&a).kinds == FS_READ` |
| `ctx_child_intersects_caps` | root `Ctx` with `{FS_READ, FS_WRITE}`, `narrow = {FS_READ}` | child `caps().kinds == FS_READ`; `child.fs()` is `Ok`; child's `FsCap::write` → `Err(Denied("fs_write"))` |
| `ctx_child_cannot_widen_caps` | root with `{FS_READ}`, `narrow = {FS_READ, NET}` | child `caps().kinds == FS_READ`; `child.net()` → `Err(Denied("net"))` |
| `ctx_child_carves_budget` | parent budget `tokens = 1000`, `BudgetShare::Fraction(25)` | child `remaining(Tokens) == 250` **and** parent `remaining(Tokens) == 750` (carved, not duplicated) |
| `budget_charge_beyond_remaining_is_exhausted` | budget `tokens = 10`, `charge(Tokens, 11)` | `Err(BudgetExhausted("Tokens"))`, `remaining(Tokens) == 0` |
| `cancel_token_propagates_to_child` | parent token, `child()`, then `parent.cancel()` | `child.is_cancelled() == true`; cancelling the child leaves the parent uncancelled |
| `fscap_write_outside_scope_denied` | `fs_write` allow `[_tmp/ok]`, write to `_tmp/nope/f` | `Err(Denied(..))`, file not created |
| `fscap_symlink_escape_denied` | `_tmp/<tmpdir>/link -> /etc`, `fs_write` allow `[_tmp/<tmpdir>]`, write to `link/pii-escape` | `Err(Denied(..))`; `/etc/pii-escape` does not exist — the scope re-check happens **after** `dunce::canonicalize` |
| `recording_observer_reports_max_metric` | metrics `graph.batch.width` = 1, 2, 1 | `max_metric("graph.batch.width") == 2` |
| `append_event_advances_header_seq_atomically` | fresh `Store`, append 3 events | returned seqs `0,1,2`; `get_run().seq == 3`; `read_journal().len() == 3` and ordered |
| `run_by_status_index_is_repointed` | header `Running` → append with status `Succeeded` | `runs_by_status(Running)` empty, `runs_by_status(Succeeded)` contains the run |
| `next_run_id_is_monotonic` | `next_run_id()` twice | second > first lexicographically; both match `^r\d{12}$` |
| `snapshot_roundtrips` | `put_snapshot(run, 32, b"{}")` | `latest_snapshot()` returns `(32, b"{}")` |
| `graph_body_is_content_addressed` | `put_graph(bytes)` twice | same hash both times; `get_graph(hash)` returns the bytes |
| `toml_loader_parses_reserved_and_cfg_keys` | `examples/diamond.toml` source | 4 nodes; `entry == "entry"`; `join.join == JoinPolicy::All`; `a.cfg["timer_ms"] == 150` |
| `join_policy_parses_all_forms` | `"all"`,`"any"`,`"any:cancel"`,`"quorum:2"`,`"quorum:2:cancel"`,`"allsettled"`,`"bogus"` | the six policies, then `Err(Invalid)` |
| `expr_resolves_input_payload_and_nodes` | ctx `{"input":{"k":1},"payload":{"a":2},"nodes":{"a":2}}` | `$.input.k`→`1`; `$.payload.a`→`2`; `$.nodes.a`→`2`; `$.missing.x`→`Null`; `"nope"`→`Err(Invalid)` |
| `validate_accepts_plain_diamond` | `examples/diamond.toml` | `Ok(warnings)` with no errors |
| `validate_rejects_unknown_port` | `examples/lint-bad-port.toml` | `Err(e)`, some element starts with `V1` |
| `validate_rejects_cap_escalation` | `examples/lint-cap-escalation.toml` | `Err(e)`, some element starts with `V2` |
| `validate_rejects_unbounded_cycle` | `examples/lint-unbounded-cycle.toml` | `Err(e)`, some element starts with `V3` |
| `validate_accepts_cycle_with_loop_budget` | same graph with `loop_budget = 3` on `a` | no `V3` error |
| `validate_warns_unreachable_node` | diamond + an orphan node | `Ok(w)`, some warning starts with `V5` |
| `validate_rejects_statically_unsatisfiable_join` | `examples/unsatisfiable.toml` | `Err(e)`, some element starts with `V6` and names `join` |
| `validate_accepts_allsettled_after_route` | `examples/routed-diamond.toml` | `Ok(_)` — V6 applies to `All` only |
| `validate_rejects_one_armed_race` | `examples/lint-one-armed-race.toml` | `Err(e)`, some element starts with `V9` |
| `readiness_entry_node_with_no_inbound_is_ready` | node with 0 inbound edges, `JoinPolicy::All` | `Readiness::Ready` (**not** `DeadNode`) |
| `readiness_all_with_pending_is_not_ready` | inbound `[Pending, Fired]`, `All` | `NotReady` |
| `readiness_all_with_dead_and_fired_is_unsatisfiable` | inbound `[Fired, Dead]`, `All` | `UnsatisfiableJoin` |
| `readiness_all_dead_is_dead_node` | inbound `[Dead, Dead]`, `All` | `DeadNode` |
| `readiness_any_fires_on_first` | inbound `[Pending, Fired]`, `Any` | `Ready` |
| `readiness_quorum_unsatisfiable_when_remaining_insufficient` | inbound `[Dead, Dead, Pending]`, `Quorum{k:2}` | `UnsatisfiableJoin` |
| `readiness_allsettled_ignores_dead` | inbound `[Fired, Dead]`, `AllSettled` | `Ready` |
| `readiness_waits_for_pending_order_edge` | Data `[Fired]`, Order `[Pending]`, `All` | `NotReady` |
| `dead_propagation_kills_data_and_fires_order_edges` | node with all inbound Data `Dead`, one outbound Data + one outbound Order | node marked dead; outbound Data `Dead`; outbound **Order `Fired`** |
| `diamond_plain_runs_both_arms_concurrently` (**Test A**) | `examples/diamond.toml`, input `{}` | (i) `a` and `b` both have a `NodeFinished`; (ii) their `[at_ms, at_ms+dur_ms)` windows overlap; (iii) exactly one `NodeFinished` for `join`; (iv) `join.at_ms >= max(a.at_ms+a.dur_ms, b.at_ms+b.dur_ms)`; (v) some `TickStarted.width >= 2` **and** `RecordingObserver::max_metric("graph.batch.width") >= 2`; (vi) `Terminal::Success` |
| `diamond_routed_marks_branch_dead_and_joins_once` (**Test B**) | `examples/routed-diamond.toml`, input `{"branch":"a"}` | `b` has a `NodeDead` event and no `NodeFinished`; exactly one `NodeFinished` for `join`; `join`'s value has key `a` and **not** key `b`; `Terminal::Success`; the call returns (does not hang) |
| `diamond_unsatisfiable_join_fails_loudly` (**Test C**) | `examples/unsatisfiable.toml`, input `{"branch":"a"}`, validation bypassed | `Terminal::Failure` whose error contains `join can never be satisfied`; the failure is journalled; the run ends within one tick of `a` finishing; `validate()` on the same spec returns a `V6` error |
| `diamond_race_cancels_loser` (**Test D**) | `examples/race.toml`, input `{}` | exactly one `NodeFinished` for `use`, its value keyed by `fast` and not `slow`; `slow` has a **`NodeCancelled`** event (not `NodeFailed`); total wall time < 2000 ms although `slow.timer_ms == 5000` |
| `retry_reruns_only_retryable_errors` | in-crate test node failing `Timeout` twice then succeeding, `retry.max = 2` | 2 `NodeFailed{retryable:true}` then 1 `NodeFinished`; the same node returning `Invalid` with `retry.max = 2` produces exactly 1 `NodeFailed` and no retry |
| `suspend_journals_and_returns_suspended` | in-crate test node returning `Outcome::Suspend` | `RunEvent::Suspended` in the journal, a snapshot written at that seq, `Terminal::Suspended`, `RunHeader.status == Suspended` |
| `resume_replays_journal_without_reexecuting` | run a 3-node chain, truncate execution after node 1 by cancelling, then `resume()` | node 1 has exactly **one** `NodeFinished` across both journals; a counter inside node 1's impl shows it ran once; the run completes `Success` |
| `snapshot_written_every_32_events` | a graph producing > 32 journal events | `latest_snapshot()` is `Some` with `seq % 32 == 0` |
| `cyclic_graph_is_rejected_at_run_time` | a lint-clean graph with `loop_budget` on a cycle | `run()` → `Err(Invalid)` mentioning Phase 2 (the documented Phase-1 boundary) |
| `deps_check_rejects_redb_in_pii_core` | embedded `cargo metadata` fixture where `pii-core` reaches `redb` | `Err(v)`, some element names `pii-core` and `redb` |
| `deps_check_accepts_redb_in_pii_store` | fixture where only `pii-store` reaches `redb` | `Ok(())` |
| `bench_gate_p95_index_is_correct` | 50 sorted samples `1.0..=50.0` | p95 sample == `48.0` (index `ceil(0.95*50)-1 == 47`) |
| `version_line_matches_cargo_pkg_version` | existing Phase-0 test | unchanged, still passes |

---


## Exit Criteria

- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && ./quality-gate.sh`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && ! grep -rnE '\basync fn\b|\.await\b|async_trait|tokio::|futures_core' --include=*.rs crates xtask`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && ./target/debug/pii graph lint examples/diamond.toml && ./target/debug/pii graph lint examples/routed-diamond.toml && ./target/debug/pii graph lint examples/race.toml && ./target/debug/pii graph lint examples/crash-resume.toml`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && mkdir -p _tmp/p1 && D=_tmp/p1/dia-$$.redb && R=dia$$ && timeout 30 ./target/debug/pii graph run examples/diamond.toml --db "$D" --run-id "$R" --input '{}' >/dev/null && test "$(./target/debug/pii graph trace "$R" --db "$D" --max-batch-width)" -ge 2 && test "$(./target/debug/pii graph trace "$R" --db "$D" --count-finished join)" = 1 && test "$(./target/debug/pii graph trace "$R" --db "$D" --overlapped a,b)" = yes && test "$(./target/debug/pii graph trace "$R" --db "$D" --terminal)" = success`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && mkdir -p _tmp/p1 && D=_tmp/p1/race-$$.redb && R=race$$ && timeout 3 ./target/debug/pii graph run examples/race.toml --db "$D" --run-id "$R" --input '{}' >/dev/null && test "$(./target/debug/pii graph trace "$R" --db "$D" --node-status slow)" = cancelled && test "$(./target/debug/pii graph trace "$R" --db "$D" --count-finished use)" = 1 && ./target/debug/pii graph trace "$R" --db "$D" --output | grep -q fast && ! ./target/debug/pii graph trace "$R" --db "$D" --output | grep -q slow`
- [ ] `export PATH=/usr/local/cargo/bin:$PATH; cd /projects/pi-improved && cargo test -q -p pii-graph -- --list 2>&1 | grep -q 'readiness_entry_node_with_no_inbound_is_ready' && cargo test -q -p pii-graph readiness_ 2>&1 | grep -qv 'FAILED'`

## Guardrails

- Do NOT write `.pop()`, `.next()`, or `.first()` where the scheduler selects ready nodes. It **drains every ready node into one batch**. This is orki defect 1 (pass-3 §4.2) and it silently fabricated provenance in a sibling project while its tests stayed green.
- Do NOT let a node fire because *an* inbound edge fired. Firing is decided **only** by `readiness()`, which distinguishes "an edge fired" from "the join is satisfied". That is orki defect 2.
- Do NOT transcribe the pass-3 `readiness()` pseudocode literally without adding the empty-inbound guard first. `[].iter().all(..)` is `true`, so the entry node would be classified `DeadNode` and nothing would ever run.
- Do NOT let an unsatisfiable join hang, retry, or be silently skipped. It is `Terminal::Failure` with the exact string `join can never be satisfied`, journalled, within one tick. A broken join must be loud, immediate, and named.
- Do NOT let dead-branch propagation kill `Order` edges. Order edges become `Fired` when their source dies — running the cleanup after a failed branch is the entire reason `Order` exists.
- Do NOT add a `race` node kind. Racing is `join = "any"` / `"quorum:K"` on the joiner (pass-3 §4.4). Do NOT add a second scheduler, a "small loop for this special case", or a bespoke runner inside any node kind.
- Do NOT use `async fn`, `.await`, `async-trait`, `futures-core`, `tokio`, or any async runtime. Phase 1 is synchronous; concurrency is `std::thread::scope`.
- Do NOT add `redb` to any crate other than `pii-store`. Do NOT add `petgraph` or `toml` to `pii-core` or `pii-abi`. Do NOT add **any** dependency to `pii-abi` — its manifest keeps exactly `serde`, `serde_json`, `thiserror`, `schemars`, and `xtask`'s `ABI_ALLOWED_DIRECT` is not to be edited.
- Do NOT add a dependency outside this list: `bitflags 2.13`, `parking_lot 0.12`, `petgraph 0.8`, `redb 4.2`, `toml 1.1`, `blake3 1.8`, `dunce 1.0`, and `tempfile 3` (dev-only). No `anyhow`, no `clap`, no `uuid`, no `chrono`, no `walkdir`, no `rayon`, no `crossbeam`.
- Do NOT hold a redb `WriteTransaction` across a node dispatch, store one in a struct field, or return one from a method. Open and commit inside a single method body.
- Do NOT call `SystemTime::now`, `Path::canonicalize`, `Command::spawn`, `std::env::var`, or use `std::sync::Mutex` — `clippy.toml` denies all five and `-D warnings` will fail the gate. Use `Instant`, `dunce::canonicalize`, `Command::output()`, argv, and `parking_lot::Mutex`.
- Do NOT check the filesystem scope on a pre-resolution path. Canonicalise first, re-check second, open third. A scope check before symlink resolution is theatre and `fscap_symlink_escape_denied` exists to prove it.
- Do NOT let `Ctx::child` union capabilities or duplicate budget. Intersection and carving only — attenuation must hold by construction, not by a validation call someone can forget.
- Do NOT make the blackboard per-key write-through to redb. One batched journal append per node completion, snapshot every 32 events. orki's 22× penalty came from per-key granularity; "fixing" this later by adding per-key durability rediscovers it.
- Do NOT re-invoke a completed node on resume. `NodeFinished` carries the value; that is what makes resume free of duplicate spend and duplicate side effects. A node with `NodeStarted` and no `NodeFinished` **is** re-run — that is correct and intended.
- Do NOT use `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `dbg!`, `process::exit`, or slice indexing (`x[i]`) in non-test code. Return `Result`; `main` returns `ExitCode`; use `.get()`.
- Do NOT let any source file exceed 450 effective lines. Split into `sched/` and `nodes/` module folders before that happens.
- Do NOT write anything to `/tmp`. Test artifacts and databases go under `_tmp/`; unit tests use `tempfile::Builder::new().tempdir_in("_tmp")`.
- Do NOT run `rm -rf` or delete/overwrite anything that existed before this run. Do NOT touch `_repos/`, `_docs/`, `specs/`, `AGENTS.md`, `HANDOFF.md`, or `.pidag/`.
- Do NOT run `git init`, stage, or commit. File creation only.
- Do NOT implement `llm`, `tool`, `agent`, `map`, `subgraph`, or `gate` node kinds, WASM or worker backends, `pii-model`, `pii-tools`, `pii-memory`, BM25, vectors, or cyclic *execution*. They are later phases. `GraphSpec::validate()` still implements V3 in full.
- Do NOT write `id = "x"; kind = "y"` on one line in any `.toml` file. Pass-1 §3.6 uses that shorthand for readability; it is not valid TOML and `toml::from_str` will reject it.
- **Do NOT create artifacts owned by other parts.** Only what Scope lists.
The Architecture section describes the full system; building all of it here
duplicates other parts' work and defeats the split.

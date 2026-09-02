# Pii (Pi Improved) — Architecture Plan, Pass 2

> Pass 1 lives at `/projects/pi-improved/_docs/06-research/001-pii-architecture-plan.md`.
> This document **extends and challenges** it across 12 new directions. Where pass 1 and
> pass 2 disagree, pass 2 wins and says why. Everything pass 1 decided and pass 2 does not
> mention still stands.

---

## Context

Pass 1 established the substrate: a 14-crate Rust workspace, a transport-free kernel ABI
(`pii-abi`), an enforcing capability context (`Ctx`), a dynamic-DAG graph engine with an
event-sourced journal in redb, WASM components as the safe way to run machine-generated
code, UDS worker processes for hardware and untrusted work, and a hermes-shaped
self-improvement loop that promotes prose skills into executable graphs.

Pass 2 asks the questions pass 1 deferred or got wrong:

- How is any of this **debuggable** at 3 a.m. on day 400? (§2)
- The TUI is currently assumed to be part of the binary. It should be a **client**. What
  protocol? (§3)
- The daemon is described as a consumer of the network. It should be a **provider** —
  sockets, brokers, protocol handlers, and plugins that can request them. (§4, §11)
- redb + hand-rolled BM25 was pass 1's memory answer. There is a **proven Rust library**
  in this workspace already (`agent-memory-core`) and a strong published layering
  (Tencent's L0→L3). Pass 1's memory design is under-specified by comparison. (§5)
- `pidag`, `agent-memory`, `orki` exist. Which get **subsumed**, which get **embedded**,
  which get **wrapped**? (§6)
- Multiple sites. redb is single-writer *per process*, and now also per *site*. (§7)
- Pass 1's graph engine blocks. Hermes doesn't. **Callbacks, not loops.** (§8)
- 14 MB tool-calling models now run on a $10 MCU. The "tiering" story stops being a
  slide and becomes a wiring diagram. (§9)
- WASM plugins in pass 1 implement `Tool` or `GraphNode`. They should be able to implement
  **any kernel trait**, and talk to each other. (§10)

**Intended outcome of pass 2:** a plan concrete enough that Phase 1 can start without
another research round, with the newly-added surfaces (observability, client/server,
networking, layered memory, sync, MCU) each having a named crate, a named dependency set,
and a machine-checkable exit criterion.

---

## 0. Corrections to pass 1 (read these first)

Pass 1 contained five load-bearing corrections to the original brief. Pass 2 adds seven
corrections to **pass 1**, all verified this pass.

| # | Pass-1 claim | What is actually true |
|---|---|---|
| 1 | "wasmtime 47.x, WASI 0.2 (P2), component model" | **wasmtime is at 48.0.1** (2026-08-25). **WASI 0.3.0 shipped 2026-06-11** with native async (`async func`, `stream<T>`, `future<T>`), `wasi:io` **deleted**, and `wasi:sockets` promoted to stable. Wasmtime 46+ implements final 0.3. **But** `wasmtime-wasi 48.0.1`'s `p3` module is documented verbatim as *"Experimental, unstable and incomplete implementation of wasip3"*. → **Build on p2, design the WIT world so the p3 migration is mechanical, revisit at wasmtime ~52.** This is a real fork in the road pass 1 did not know existed (§10.1). |
| 2 | "Phase 0: there is no Rust toolchain in this environment" | Still true **in this container** — `rustc: command not found`, `~/.cargo/bin` empty, no rustup targets. The brief states 1.95.0 + probe-rs/cargo-embed/cargo-flash/cargo-uf2 are installed; that is the **host**, not this container. Any CI/agent workflow must not assume a toolchain inside the container. |
| 3 | "`tokio-serial` for the serial bridge" (§7 Phase 4) | **`tokio-serial` is unmaintained and openly seeking adoption.** Use **`serial2-tokio`** (active as of 2026-07) or `postcard-rpc`'s own transport. |
| 4 | "Vector search: i8 brute force, no ANN index, add HNSW only when p95 exceeds budget" | Defensible, but pass 1 did not know `agent-memory-core` already ships a `VectorBackend` trait with an **HNSW default** and a Qdrant alternative, plus tantivy BM25 and a petgraph co-occurrence GraphRAG layer. The right move is not to re-derive it (§5). |
| 5 | "Hand-roll BM25, ~600 LOC, the concrete cost of the redb choice" | Only if you ignore **tantivy**, which `agent-memory-core` already pairs with redb. ~600 LOC of hand-rolled scoring is a cost you do not have to pay. |
| 6 | "`pii-tui` is a crate in the binary; ACP is a separate mode" | The client/server split (§3) makes the TUI a **protocol client**, and ACP a **second client of the same protocol**, not a parallel stack. This changes the crate graph. |
| 7 | Observability appears as one trait (`Observer`) and one `TraceCtx` field with no design behind them | Observability is a **subsystem**, and the reference implementation (`zeroclaw-log`, 6,616 lines across 13 files) shows exactly how much design it takes. §2 specifies it. |

---

## 1. Rust best practices, design patterns, and enforcement

### 1.1 The principle

Pass 1 already stated the doctrine: *architecture enforced by lint, not by document*
(`grok-build`'s `clippy.toml` `disallowed-methods`). Pass 2 extends it to a full gate set.
The test for whether a rule belongs here is simple: **if a tired person at 1 a.m. can
violate it and CI stays green, it is not a rule, it is a wish.**

### 1.2 Design patterns — the ones that are actually decisions

**Traits-first, dyn-dispatched, one required method.** Already decided in pass 1 (§2.5)
and confirmed by `zeroclaw-api`: `ModelProvider` has 17 methods, one required. Keep it.

**Newtypes for every id, no exceptions.** `RunId`, `NodeId`, `DocId`, `SkillId`,
`PluginId`, `GraphHash`, `PeerId`, `SiteId`. All are `#[repr(transparent)]` newtypes over
`u128`/`[u8;32]` with `Display`/`FromStr`. The bug this prevents — passing a `RunId` where
a `DocId` is expected — is silent, and it is exactly the bug an LLM writes.

```rust
// pii-abi/src/ids.rs
macro_rules! id_newtype {
    ($name:ident, $inner:ty, $prefix:literal) => { /* Debug, Display, FromStr, serde, Ord */ };
}
id_newtype!(RunId, u128, "run");   // Display → "run_01J8Z..."  (ULID-ish, sortable)
```

**Sealed traits for anything the kernel must be able to reason about exhaustively.**
`Outcome`, `ContentBlock`, `NodeKind` and `PiiError` are *not* extensible by plugins —
plugins extend `Tool`/`Channel`/`GraphNode` (open) but not the routing vocabulary (closed).
Seal via the private-supertrait pattern. This is the same discipline as `zeroclaw-log`'s
closed `Action`/`EventCategory` enums with no `Other(String)` escape hatch — a deliberate
taxonomy that forces new concepts through review instead of letting them accrete as strings.

**Builders only where the argument count justifies it.** `Ctx`, `GraphSpec`, `PluginLimits`,
`ChatRequest`. Not for 2-field structs. `PluginLimits` in particular has **no `Default`**
(pass 1 §5.4) — ZeroClaw's reasoning is right and it generalises: *any struct where the
zero value is a security downgrade must not implement `Default`.* Add to that list:
`CapSet`, `SubgraphPolicy`, `NetGrant` (§4).

**Errors.** `thiserror` in every library crate; `anyhow` **only** in `pii-cli` and `xtask`.
`PiiError` (pass 1 §2.3) is the ABI error. Add one variant pass 1 missed, needed by §4:

```rust
#[error("no route to {host}:{port}")] NetUnreachable { host: String, port: u16 },
```

`is_retryable()`/`is_terminal()` stay the routing predicates.

**Async.** `#[async_trait]` for every `dyn`-dispatched trait (pass 1 §2.2, unchanged — AFIT
is still not `dyn`-compatible). **AFIT *is* correct for non-object-safe internal traits**
(e.g. `WireTx`/`WireRx` in the worker transport, §3) where there is exactly one impl per
build — no boxing, no proc macro. State the rule: *`async_trait` iff `dyn`; AFIT otherwise.*

**Interior mutability.** `Arc<Mutex<T>>` is a smell in a hot path. The house rules:
- Counters/budgets → `AtomicU64` (already in pass 1's `Budget`).
- Config/registries read hot, written cold → `arc_swap::ArcSwap`.
- Everything else → **message passing** (`tokio::sync::mpsc` to an owner task).
- `parking_lot::Mutex` only for short non-async critical sections. **Never hold any lock
  across `.await`** — enforced by `clippy::await_holding_lock` (deny).

**Ownership discipline.** `ChatRequest<'a>` borrows history (pass 1). Extend: any type
appearing in a per-turn hot path takes `&[T]`/`Cow<'a, T>`, not `Vec<T>`. `ToolSpec.schema`
is `Arc<serde_json::Value>` (from `zeroclaw-api`). `SmolStr` for all short identifiers.
`bytes::Bytes` for all binary payloads that cross a task boundary.

**`#![forbid(unsafe_code)]` in every crate except `pii-hw`** (which needs it for
`SCM_RIGHTS`/`pre_exec`) **and `pii-memory`** (SIMD dot-product intrinsics). Those two
carry a `# Safety` doc comment per `unsafe` block, checked by
`clippy::undocumented_unsafe_blocks` (deny).

### 1.3 The lint set — concrete

`[workspace.lints]` in the root `Cargo.toml`, inherited by every member with
`[lints] workspace = true`. This is the single place lints live.

```toml
[workspace.lints.rust]
unsafe_code                = "forbid"     # overridden to "allow" in pii-hw, pii-memory only
missing_docs               = "warn"       # "deny" for pii-abi — it is the published ABI
unreachable_pub            = "deny"
unused_qualifications      = "warn"
rust_2018_idioms           = { level = "deny", priority = -1 }

[workspace.lints.clippy]
# Baseline groups
pedantic  = { level = "warn", priority = -1 }
nursery   = { level = "warn", priority = -1 }
cargo     = { level = "warn", priority = -1 }

# --- HARD DENY: correctness and the failure modes this project actually has ---
unwrap_used              = "deny"   # allow in #[cfg(test)] via crate-level attr
expect_used              = "deny"
panic                    = "deny"   # production code does not panic; workers do not either
indexing_slicing         = "deny"   # LLM-written code loves a[i]
integer_arithmetic       = "allow"  # too noisy; rely on overflow-checks in release (below)
await_holding_lock       = "deny"
await_holding_refcell_ref= "deny"
undocumented_unsafe_blocks = "deny"
mem_forget               = "deny"
exit                     = "deny"   # only pii-cli::main may exit
print_stdout             = "deny"   # everything goes through pii-obs (§2)
print_stderr             = "deny"
dbg_macro                = "deny"
todo                     = "deny"
unimplemented            = "deny"
large_futures            = "deny"   # boxed futures in a daemon with 50MB RSS budget
large_enum_variant       = "deny"
missing_panics_doc       = "warn"
missing_errors_doc       = "warn"

# --- Explicit ALLOW: pedantic/nursery noise that fights the design ---
module_name_repetitions  = "allow"
must_use_candidate       = "allow"
missing_const_for_fn     = "allow"  # nursery, high false-positive rate
multiple_crate_versions  = "allow"  # cargo group; we quarantine deliberately (§1.5)
```

**`clippy.toml`** — the architecture rules. Pass 1 took four from `grok-build`; pass 2 adds
the ones the new directions require:

```toml
# --- from grok-build, verbatim reasoning ---
disallowed-methods = [
  { path = "reqwest::Client::new",            reason = "use pii_model::http::shared_client() — one TLS root store, one pool" },
  { path = "std::path::Path::canonicalize",   reason = "use dunce::canonicalize (Windows \\\\?\\ paths)" },
  { path = "dirs::home_dir",                  reason = "use pii_core::paths::home() — respects PII_HOME" },
  { path = "std::process::Command::spawn",    reason = "use Ctx::proc()? — an unenrolled child outlives its scope" },
  { path = "tokio::process::Command::spawn",  reason = "use Ctx::proc()?" },
  # --- new in pass 2 ---
  { path = "tokio::net::TcpListener::bind",   reason = "use NetService::listen() — grants, accounting, shutdown (§4)" },
  { path = "tokio::net::TcpStream::connect",  reason = "use Ctx::net()?.connect() — host allowlist is checked there" },
  { path = "tokio::net::UnixListener::bind",  reason = "use NetService::listen_uds() — socket mode 0600 is checked there" },
  { path = "std::time::SystemTime::now",      reason = "use Ctx::now() — deterministic in replay and in tests" },
  { path = "std::env::var",                   reason = "use pii_core::config — env reads must be enumerable for the daemon" },
]
disallowed-macros = [
  { path = "tracing::info",  reason = "use pii_obs::record! — closed Action taxonomy (§2)" },
  { path = "tracing::warn",  reason = "use pii_obs::record!" },
  { path = "tracing::error", reason = "use pii_obs::record!" },
  { path = "println",        reason = "use pii_obs::record! or, in pii-cli only, the UI writer" },
]
disallowed-types = [
  { path = "std::sync::Mutex", reason = "use parking_lot::Mutex (sync) or tokio::sync::Mutex (across await)" },
]
too-many-lines-threshold = 80      # clippy::too_many_lines, per FUNCTION
type-complexity-threshold = 250
```

> `zeroclaw-log` self-exempts from `disallowed-macros` (`[lints.clippy] disallowed_macros
> = "allow"`) because it *is* the plumbing. `pii-obs` does the same. That is the correct
> and only exemption.

**`rustfmt.toml`** — pinned so diffs are never formatting churn:

```toml
edition = "2024"
max_width = 100
imports_granularity = "Crate"     # nightly-only; run via `cargo +nightly fmt` in CI
group_imports = "StdExternalCrate"
newline_style = "Unix"
use_field_init_shorthand = true
```

**Release profile** (pass 1 §4.2, plus one addition):

```toml
[profile.release]
opt-level = "z"; lto = "fat"; codegen-units = 1; panic = "abort"; strip = true
overflow-checks = true            # ← NEW: cheap at "z", and `integer_arithmetic` is allowed
[profile.perf]
inherits = "release"; opt-level = 3; lto = "thin"; debug = 1; strip = false; panic = "unwind"
```

### 1.4 The 450-line file gate — how to enforce it

The user wants **450 lines per file**, not pass 1's 2,000. That is a much sharper knife and
it needs a correspondingly better tool, because a naive gate makes people write 449-line
files full of dead space.

**There is no clippy lint for file length.** `clippy::too_many_lines` is per-*function*
(set to 80 above). So this is an `xtask` gate. Implementation:

```
cargo xtask lint-files
```

- Walks `crates/*/src/**/*.rs` and `xtask/src/**/*.rs` (skip `target/`, skip generated
  `bindgen!` output, skip `tests/` fixtures).
- Counts **effective lines**: total minus blank, minus pure-comment, minus `#[cfg(test)]`
  module bodies. *Tests do not count against the budget* — otherwise the gate punishes
  testing, which is the exact wrong incentive.
- **Fails at > 450 effective lines. Warns at > 380** so the wall is visible before you hit it.
- Emits the fix, not just the complaint:
  `error: crates/pii-graph/src/scheduler.rs is 512 effective lines (limit 450).
   Split into crates/pii-graph/src/scheduler/{mod,ready_set,budget}.rs`
- Per-file opt-out requires a **committed reason**, in a checked-in `xtask/file-size-allow.toml`:
  ```toml
  [[allow]]
  file = "crates/pii-wasm/src/bindings.rs"
  lines = 4200
  reason = "wit-bindgen! generated; regenerated by `cargo xtask wit`"
  expires = "never"
  ```
  Any allow without `reason` is a CI failure. Any allow with a past `expires` is a CI failure.
  This is the pattern that keeps a gate honest: exceptions are legal, invisible exceptions
  are not.

**Also enforce the folder convention**: a module over 300 effective lines with more than
one `pub` item group *warns* with "consider `mod.rs` + submodules". Warning only — this is
taste, not correctness.

**Companion gate — module reachability** (pass 1 §8.3, from `pi_agent_rust/AGENTS.md:619`):
every `pub mod` must have ≥1 non-test call site. Keep it; it is what caught five dead
modules with green tests.

### 1.5 The tooling set — recommended, with the ones to skip

| Tool | Verdict | Where it runs |
|---|---|---|
| **`cargo fmt --check`** (nightly, for `imports_granularity`) | **Yes** | pre-commit + CI |
| **`cargo clippy --all-targets --all-features -- -D warnings`** | **Yes** | pre-commit + CI |
| **`cargo nextest run`** | **Yes** — process-per-test isolation matters when tests spawn workers, bind sockets, and open redb files; `cargo test`'s shared-process model produces flaky cross-talk here | CI + local |
| **`cargo deny check`** (`advisories`, `bans`, `licenses`, `sources`) | **Yes** — supersedes `cargo-audit` entirely (it embeds the same RustSec DB) and additionally enforces the license policy and the *duplicate-version bans* that make §1.6's quarantine rule checkable | CI, daily cron |
| **`cargo-audit`** | **Skip** — subsumed by `cargo deny check advisories` |
| **`cargo machete`** | **Yes** — stable toolchain, fast, catches unused deps | CI |
| **`cargo udeps`** | **Skip** — needs nightly and a full rebuild; `machete`'s false-negative rate is an acceptable trade for a solo project |
| **`cargo llvm-cov`** | **Yes** | CI nightly job |
| **`cargo tarpaulin`** | **Skip** — `llvm-cov` is source-based, faster, and more accurate on async code. Do not use both. |
| **`cargo hakari`** | **Skip for now.** It exists to unify features across a *large* workspace to cut rebuild times. At 14–18 crates with one binary the workspace-hack crate is pure ceremony. Revisit if a clean CI build exceeds ~8 min. |
| **`cargo about`** | **Yes**, at release only — generates the `THIRD-PARTY.md` a shipped static binary needs |
| **`cargo bloat` / `cargo size`** | **Yes**, in `xtask size-gate` — the 30–50 MB envelope is a contract |
| **`typos`** | **Yes** — 200 ms, catches doc/identifier typos |
| **`committed`/`cog`** | **Skip** — solo project, conventional-commit linting is overhead |

**Coverage target**: do **not** set a global percentage. Set it per-crate where it means
something: `pii-abi` and `pii-graph` ≥ 85% (the validator and scheduler are where a bug is
expensive and invisible); everything else reported but ungated. A global 80% gate on a
crate like `pii-tui` produces test theatre.

### 1.6 The CI pipeline — concrete

```yaml
# .github/workflows/ci.yml (or the local equivalent — this is a solo project, so it must
# also run as `cargo xtask ci` with no network and no GitHub)
jobs:
  fast:            # < 90s — this is the pre-commit hook too
    - cargo fmt --check
    - cargo xtask lint-files          # 450-line gate + reachability + allow-list validation
    - cargo xtask deps-check          # the §2.1 dependency DAG; must FAIL if pii-abi gains tokio
    - cargo clippy --workspace --all-targets -- -D warnings
    - typos

  test:            # < 8 min
    - cargo nextest run --workspace
    - cargo nextest run --workspace --no-default-features
    - cargo nextest run -p pii-graph --features proptest   # validator property tests

  contracts:       # the perf claims, as gates (pi_agent_rust's posture)
    - cargo xtask bench-gate          # `pii --version` p95 < 20 ms, 100 runs
    - cargo xtask size-gate           # stripped musl binary < budget
    - cargo xtask rss-gate            # daemon idle RSS < 50 MB after 120 s

  supply-chain:    # nightly cron, not per-PR
    - cargo deny check
    - cargo machete
    - cargo llvm-cov --workspace --lcov --output-path lcov.info
    - cargo xtask wit-check           # WIT world unchanged, or version bumped (§10)
```

**`cargo xtask ci` runs all of it locally with no network.** For a solo project that
alternates between a container without a toolchain and a host with one, the local runner is
the real gate and GitHub Actions is the backup. Everything above is a plain function in
`xtask`, and the workflow file just calls them.

**The two gates that matter most** (everything else is hygiene):
1. `deps-check` — provably fails when `pii-abi` gains a `tokio` dependency. Pass 1 is right
   that this is the single most valuable structural decision available, and it is only
   valuable if it is enforced.
2. `lint-files` — the 450-line wall. `zeroclaw-runtime/src/agent/loop_.rs` is 17,727 lines;
   it did not get there in one commit.

---

## 2. Observability (`pii-obs`)

### 2.1 What the reference implementation actually costs

`zeroclaw-log` is **6,616 lines across 13 files** (`writer.rs` 1942, `layer.rs` 882,
`reader.rs` 824, `event.rs` 668, `migrate.rs` 548, `subscriber.rs` 485,
`observer_bridge.rs` 465, …). That is the honest price of "structured logging that is
actually usable a year later". Budget for it, and steal the design rather than rediscovering
it.

### 2.2 The stack — decided

| Concern | Choice | Why not the alternative |
|---|---|---|
| Instrumentation API | **`record!` macro over a closed `Action` taxonomy** | Free-form `tracing::info!("thing happened")` is unqueryable at scale. zeroclaw bans it via `disallowed-macros` and it works. |
| Transport underneath | **`tracing` + `tracing-subscriber`, as an internal implementation detail only** | Reusing `tracing`'s span machinery gives context propagation for free; exposing it as API gives you back the free-form problem. |
| Durable output | **JSONL, one file per day, self-rotating** | A daemon's log must be greppable with `jq` on a machine with nothing installed. |
| Live output | **`tokio::sync::broadcast<Arc<LogEvent>>` → the event bus (§8)** | Tailing a file to get liveness is a hack; the bus is needed for §8 anyway. |
| Metrics | **`metrics` crate facade**, `metrics-exporter-prometheus` behind feature `metrics` (**on** by default — it is ~0 cost and the `/metrics` route is how you catch a leak) | Hand-rolled counters drift; a facade lets the exporter change without touching call sites. |
| Distributed tracing | **`tracing-opentelemetry` + `opentelemetry-otlp`, feature `otel`, OFF by default** | The OTel SDK is a large tree and needs a collector. A 50 MB-RSS daemon does not carry it unless asked. |
| Async task inspection | **`tokio-console`, feature `console`, debug builds only** | Requires `--cfg tokio_unstable`, which poisons the release build. Document the exact command. |
| Profiling | **`pprof` (Linux), feature `pprof`; `pii prof <secs>` writes a flamegraph SVG** | perf/flamegraph externally is fine too, but an in-daemon endpoint works on a headless box you can't attach to. |
| Panic capture | **crash record written before abort** | `panic = "abort"` in release means no unwinding, so the hook must write synchronously. `pidag` ships a whole `crash_writer` binary for this — that's the level of seriousness required. |

### 2.3 The record schema (`pii.log.v1`)

Deliberately close to zeroclaw's v2 (OTel severity + ECS event descriptor + attribution
block), with three changes:

```jsonc
{
  "id": "01J8Z...",                        // ULID, sortable, doubles as a cursor
  "@timestamp": "2026-09-02T14:03:11.221Z",// RFC3339 UTC ms — lexicographically sortable
  "severity_number": 9, "severity_text": "INFO",
  "event": { "category": "graph", "action": "node_finished", "outcome": "success" },
  "service": { "name": "pii", "version": "0.1.0", "site": "home" },   // ← site added (§7)
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",   // ← W3C, 16 bytes hex (change 1)
  "span_id":  "00f067aa0ba902b7",                   // ← W3C, 8 bytes hex
  "pii": {                                  // attribution — closed field list
    "run_id": "run_01J8Z...", "node_id": "implement", "node_kind": "agent",
    "graph": "coding.verify-fix", "model": "claude-...", "provider": "anthropic",
    "principal": "user", "duration_ms": 31002
  },
  "message": "node finished",
  "attributes": { "port": "done", "tokens_in": 18422, "cost_micro_usd": 41200 },
  "schema_version": 1
}
```

**Change 1 — real W3C trace ids, not opaque strings.** zeroclaw's `trace_id` is an
`Option<String>` populated in-process. Pii's is a genuine W3C `traceparent` that
**propagates across process boundaries**: into the worker RPC header, the outbound LLM
request's `traceparent` HTTP header, the MQTT v5 user property, the MCU postcard-rpc frame
header, and the WASM host-call frame. This is the thing that makes a graph run traceable
when it spans daemon + worker + MCU, and it is what a purely in-process attribution scheme
cannot give you.

**Change 2 — closed taxonomy, enforced.** `EventCategory` and `Action` are `#[non_exhaustive]`-free
closed Rust enums with **no `Other(String)`**. zeroclaw's discipline, kept exactly.
Categories: `Graph | Tool | Model | Memory | Channel | Net | Plugin | Worker | Mcu | Sync |
Skill | Store | System`.

**Change 3 — `ephemeral_attributes`.** `#[serde(skip)]` so it is *never* written to disk,
broadcast-only, for pairing codes/QR/one-time tokens. Copy zeroclaw's `EPHEMERAL_BROADCAST_MARKER`
rule verbatim: frames carrying ephemeral data are withheld from **unauthenticated**
subscribers. This is a fail-closed secret-handling rule that costs nothing and prevents a
whole bug class.

**Attribution field list is a single const**, read by writer, reader, query validator, TUI
and web client:

```rust
// pii-obs/src/attribution.rs
pub const ATTRIBUTION_FIELDS: &[&str] = &[
    "run_id","parent_run_id","node_id","node_kind","graph","tool","plugin","model",
    "provider","channel","worker","mcu","peer","principal","session","skill","site",
];
```
Adding a field is a one-line change. zeroclaw proves this scales; the alternative (each
consumer knowing its own field list) drifts within weeks.

### 2.4 `TraceCtx` — how it threads through `Ctx`

```rust
// pii-core/src/trace.rs
#[derive(Clone, Copy, Debug)]
pub struct TraceCtx {
    pub trace_id: TraceId,          // [u8;16] — stable for an entire run tree
    pub span_id:  SpanId,           // [u8;8]  — this node / tool call / rpc
    pub parent:   Option<SpanId>,
    pub flags:    TraceFlags,       // sampled bit
}
impl TraceCtx {
    pub fn root(run: RunId) -> Self;                  // trace_id derived from RunId
    pub fn child(&self) -> Self;                      // new span_id, parent = self.span_id
    pub fn traceparent(&self) -> String;              // "00-<trace>-<span>-01"
    pub fn parse_traceparent(s: &str) -> Option<Self>;
}
```

`Ctx` already carries `trace: TraceCtx` (pass 1 §2.4). The rule:

> **Every `Ctx::child()` mints a new span. Every boundary crossing serialises `traceparent`.**

Boundaries and their carrier:

| Boundary | Carrier |
|---|---|
| Graph node dispatch | `Ctx::child()` — in-process span |
| Tool call | `Ctx::child()`, span name = tool name |
| LLM HTTP request | `traceparent` request header |
| Worker UDS JSON-RPC | `"_trace"` member on the JSON-RPC envelope |
| MCU postcard-rpc | fixed 25-byte header prefix on the frame |
| WASM host call | host reads it from the store's `PluginState`, not from the guest |
| MQTT publish | MQTT v5 user property `traceparent` |
| Control-protocol notification | `trace` field on the notification envelope |
| Sync push | per-record, so a remote trajectory keeps its trace |

**Explicit non-goal**: the trace is *not* the journal. The journal (pass 1 §3.4) is durable,
replayable, and authoritative — it is what `pii graph trace <run_id>` renders and what
resume reads. Traces are sampled, lossy, and for latency debugging. **Do not duplicate node
timings into both.** The journal records `dur_ms`; the trace records the span tree. If OTel
is off, you lose nothing that matters.

### 2.5 Emission path and cost

```
record!(INFO, Event::new(Category::Graph, Action::NodeFinished)
                 .outcome(Outcome::Success)
                 .attrs(json!({ "port": "done", "tokens_in": n })),
        "node finished")
   │
   ├─► tracing::event! (target "pii_log_event")     ← transport only
   │        │
   │        └─► CaptureLayer::on_event
   │               ├─ walk span scope leaf→root, merge attribution (inner wins)
   │               └─ build LogEvent
   │                     ├─► bounded sync_channel(1024) ──► writer std::thread ──► JSONL + fsync policy
   │                     ├─► broadcast::Sender<Arc<LogEvent>>  ──► event bus (§8) ──► clients
   │                     └─► Observer bridge ──► metrics / OTel exporters
```

Three rules taken from zeroclaw's writer, all load-bearing:

1. **Dedicated OS thread + bounded channel, drop-on-full with a counter.** Never block the
   hot path on disk. The dropped count is itself a metric (`pii_log_dropped_total`) — a
   silently lossy logger is worse than no logger.
2. **Attribution merges leaf→root, inner wins.** A `tool_call` deep in the stack gets stamped
   with `run_id`/`graph`/`model` without the call site passing them.
3. **Schema migration at startup, streamed.** `migrate.rs` rewrites v1→v2 JSONL row by row
   without slurping the file. Have the mechanism from day one even at `schema_version: 1`.

**Cost gate:** `xtask bench-gate` gets a `record!` microbench. Budget: **< 250 ns** per
record with a non-full queue, **< 40 ns** when the level is filtered out. If observability
costs more than that on a reactive IoT path targeting p95 < 100 ms end-to-end (pass 1 Phase 4),
it will get switched off, and then it doesn't exist.

### 2.6 The `Observer` bridge — fixed

zeroclaw's `observer_bridge::project` re-parses `event.action` **strings** and does
string-keyed lookups into a `serde_json::Value` bag (`.get("input_tokens")`) to rebuild a
typed `ObserverEvent`. That is a second, informal, statically-unchecked schema on top of the
"typed" one, and it silently drops unknown actions. Pii's fix is trivial: **`record!` takes
the typed event; the bridge receives the same typed value it was built from.** The JSON bag
is a *rendering* of the typed event, not its source.

```rust
// pii-abi/src/observe.rs  (sync — hot path, buffer internally)
pub trait Observer: Send + Sync + 'static {
    fn on_event(&self, e: &Event);           // the TYPED event, not a reparsed Value
    fn on_metric(&self, m: &Metric);
    fn flush(&self) {}
    fn name(&self) -> &str;
}
```
Built-in observers: `JsonlObserver` (the writer), `BroadcastObserver` (the bus),
`PrometheusObserver`, `OtelObserver` (feature), `NullObserver`.

### 2.7 Making a long-running daemon debuggable

Everything below is a client of the control protocol (§3) — **no client reads the daemon's
files directly.** That is what makes the same feature work locally and over the network.

| Need | Command | Mechanism |
|---|---|---|
| Live tail | `pii logs -f [--run R] [--node N] [--level warn] [--grep RE]` | `log.subscribe{filter}` → notifications |
| History | `pii logs --since 2h --category graph` | `log.query{cursor,filter}` → paginated byte-offset cursor over JSONL (zeroclaw's `reader.rs` model) |
| One run, end to end | `pii graph trace <run_id> [--format tree\|json\|mermaid]` | journal read, not logs |
| What's running now | `pii top` | `sys.status` + `run.list` — live runs, budgets, RSS, worker health, listener table |
| Attach to a run | `pii run attach <run_id>` | `event.subscribe{run}` — same stream the TUI uses |
| Async task stalls | `tokio-console` | debug build, feature `console` |
| CPU | `pii prof 30 > flame.svg` | `sys.profile{secs}` (admin-authenticated) |
| Crash | `~/.pii/crashes/<ts>.json` | panic hook writes synchronously before abort |
| Web | `/api/logs`, `/api/events` (SSE) | gateway proxies the same two RPCs |

**One thing zeroclaw lacks that Pii gets free**: because the tail is a control-protocol
subscription and not a gateway-only SSE bolt-on, `pii logs -f` works with no gateway, no
HTTP, and no port open.

---

## 3. Client–server architecture

### 3.1 The decision

> **The daemon is the server and owns everything: redb, the graph scheduler, providers,
> memory, channels, listeners, workers, plugins. Every UI — TUI, web, ACP, CLI one-shots —
> is a thin client of one protocol: JSON-RPC 2.0 over ndjson, defined once in `pii-proto`,
> carried over UDS locally and WebSocket remotely.**

This was already forced by pass 1 §6.1 (redb is single-process, so the daemon must own the
DB and the control channel is needed in Phase 2). Pass 2 makes it the *primary* architecture
rather than a consequence.

### 3.2 Why not the alternatives

| Option | Verdict |
|---|---|
| **(b) MCP as the control protocol** | **No.** MCP models *tools, resources, prompts* — an agent reaching outward. It has no first-class session, no assistant-delta stream, no approval round-trip, no graph run, no listener table. Using it here means tunnelling Pii's real API through `tools/call` and losing every type. **But Pii should speak MCP on both flanks**: `pii mcp` exposes Pii's tools *to* other agents, and the MCP client consumes external tool servers. Integration surface, not control plane. |
| **(d) ACP as the control protocol** | **No, as the primary.** ACP is editor↔agent (Zed, JetBrains), JSON-RPC 2.0 over stdio, scoped to coding sessions. It has no concept of a daemon, an IoT automation, a device table, or a sync peer. **Adopt it as an adapter**: `pii acp` is a bridge that maps ACP `session/*` and `session/update` onto `pii-proto`'s session API — exactly `grok-build`'s posture of wrapping the external `agent-client-protocol` crate rather than reimplementing, including its two hard-won stdio fixes (dedicated OS thread for stdin; cancel-safe `LineBufferedRead`). |
| **(c) postcard/bincode custom binary** | **Not as the primary.** A browser can't speak it without a WASM decoder, `tcpdump`/`socat` debugging dies, and version skew becomes silent corruption instead of a parse error. **But**: since the wire types are defined once in `pii-proto` with `serde`, supporting postcard is nearly free — offer it as a **negotiated encoding** for the local TUI over UDS, where both ends are the same build and the LLM-delta stream is high-frequency. Negotiated in `hello`, never assumed. |
| **(a) JSON-RPC 2.0 over UDS/WS** | **Yes.** Human-readable, one framing for stdio/UDS/WS, notifications give the §8 event model for free, `jq`-debuggable, and it is the same protocol as the worker channel (pass 1 §4.3) so there is **one RPC implementation in the codebase, not two**. |

### 3.3 `pii-proto` — the crate

The single most important thing zeroclaw gets wrong here is that **there is no protocol
crate**: every route defines its own local structs, WS frames are ad-hoc
`serde_json::json!({"type": ...})` with no shared enum, and OpenAPI is generated *after the
fact* from those ad-hoc structs. There is no compile-time guarantee the dashboard and server
agree. Pii inverts it.

```
crates/pii-proto/           # deps: serde, serde_json, schemars, thiserror.  NOTHING ELSE.
├── src/lib.rs              # PROTOCOL_VERSION, Envelope, Request, Response, Notification
├── src/session.rs          # session.* method params + results
├── src/run.rs              # run.*   graph.*
├── src/memory.rs           # memory.* skill.*
├── src/net.rs              # net.*   plugin.*   mcu.*
├── src/log.rs              # log.*   sys.*
├── src/event.rs            # the Event enum — server→client notifications (§8)
└── schema/                 # CHECKED-IN generated JSON Schema + .d.ts  (xtask proto-gen)
```

`pii-proto` depends on neither `tokio` nor `axum` nor `pii-core`. It is pure wire types, so
a web client, a Python script, and the TUI all get the same contract. `xtask proto-gen`
regenerates `schema/` and CI **fails if it differs from what's committed** — the schema is
the artifact, not a byproduct.

### 3.4 Transport and handshake

```
local   : unix://$XDG_RUNTIME_DIR/pii/<instance>/control.sock   (mode 0600, dir 0700)
remote  : wss://host:7420/v1/ws     subprotocol "pii.v1"        (TLS required, always)
embedded: stdio                     (for `pii acp`, and for tests)
```

Framing is **newline-delimited JSON** on UDS and stdio; **one WS text frame per message** on
WebSocket. Identical bytes either way — a client that works over UDS works over WS by
swapping the reader.

```jsonc
// → client
{"jsonrpc":"2.0","id":1,"method":"hello","params":{
  "protocol":"pii.v1","client":{"name":"pii-tui","version":"0.1.0","build":"a41f..."},
  "encodings":["json","postcard"], "features":["approvals","graph-view","log-tail"]}}
// ← server
{"jsonrpc":"2.0","id":1,"result":{
  "protocol":"pii.v1","server":{"version":"0.1.0","build":"a41f...","site":"home"},
  "encoding":"postcard", "capabilities":["sessions","graphs","memory","net","mcu","sync"],
  "limits":{"max_frame_bytes":1048576,"max_subscriptions":32}}}
```

**Version skew is a hard refusal, not a warning** — same rule as the worker `hello`
(pass 1 §4.3). A client one minor version ahead gets a clear error, not undefined behaviour.

**Auth.**
- **UDS**: `SO_PEERCRED` — the connecting uid must equal the daemon's uid. No token, no
  pairing, no config. This is the right local-security answer and zeroclaw does not have it
  (it is TCP-only, with no `UnixListener` anywhere).
- **Remote**: TLS + bearer token issued by `pii pair` (zeroclaw's pairing model is good:
  `POST /pair` with a one-shot code → token; token accepted via `Authorization` header,
  `Sec-WebSocket-Protocol: bearer.<token>`, or `?token=` — **and Pii drops the query-param
  form**, since it lands in proxy logs).
- Two roles: `operator` (everything) and `viewer` (read + subscribe, no mutation, no
  secrets, no `sys.*`). Approvals require `operator`.

### 3.5 The method surface

```
sys.hello  sys.status  sys.shutdown  sys.reload  sys.profile  sys.version
session.create {cwd, model, tools, graph?} -> {session_id}
session.prompt {session_id, content[]} -> {run_id}          # returns IMMEDIATELY (§8)
session.interrupt {session_id}
session.state  {session_id} -> {messages[], tokens, cost, status}   # full snapshot
session.list | session.close
run.start  {graph, input, caps?} -> {run_id}                # returns IMMEDIATELY
run.get    {run_id} -> RunHeader
run.list   {status?, since?}
run.cancel {run_id}
run.resume {run_id}
run.await  {run_id, timeout} -> Terminal                    # sugar over subscribe; never used internally
run.trace  {run_id} -> JournalView                          # the pii graph trace payload
graph.apply|get|list|lint|diff|history
approve.list | approve.resolve {approval_id, decision, note}
memory.search|put|get|forget|consolidate
skill.list|open|write|learn|curate|promote
net.listeners | net.listen | net.close | net.dial | net.tap
plugin.list|install|grant|revoke|reload
mcu.list|sync|infer|tune
sync.peers|add|pull|push|status
log.query {cursor, filter} -> {records[], next_cursor}
log.subscribe {filter} -> {sub_id}          # notifications follow
event.subscribe {filter:{run?,session?,kinds[],since_seq?}} -> {sub_id}
event.unsubscribe {sub_id}
```

### 3.6 The client/server line — what a client may and may not do

| Client may | Only the daemon may |
|---|---|
| Send prompts, cancel runs | Open the redb files |
| Subscribe to events; render deltas | Execute graph nodes |
| Read a full `session.state` / `run.trace` snapshot | Call model providers (holds the API keys) |
| Approve/deny a suspended gate | Own listeners and outbound sockets |
| Browse/search logs and memory | Spawn/supervise workers and MCUs |
| View and lint graphs; apply a graph | Instantiate WASM plugins |
| List peers, trigger a sync | Perform the sync, decide redaction |
| Negotiate encoding, reconnect, resubscribe | Hold any secret |

**The rule that makes a web client a drop-in replacement for the TUI:**

> **A client holds no domain state.** It holds a rendering of a snapshot plus a stream of
> deltas. Reconnect = re-`hello`, re-subscribe with `since_seq`, re-fetch the snapshot.
> A client that computes anything the server also computes is a bug.

This is why the TUI stops being a crate that links the engine and becomes `pii-tui`
depending on **only** `pii-proto` + `ratatui` + `tokio`. It is the cheapest possible proof
that the boundary is real: if `pii-tui` can compile without `pii-graph` in its dependency
tree, the split is honest — and `xtask deps-check` asserts exactly that.

### 3.7 Standalone (no daemon) mode

`pii -p "task"` with no daemon running must still work, and must not require starting one.
Solution: **an in-process loopback transport.** The CLI spins up the server object in the
same process and connects a `tokio::io::duplex` pair to it. Same protocol, same code path,
zero sockets, zero extra latency. Pass 1's "own redb at a session path, merge on next daemon
start" (§6.1) stays as the storage answer; §3.7 is the *protocol* answer, and it means there
is exactly one execution path in the codebase instead of two.

---

## 4. Networking — the daemon owns sockets, plugins request them (`pii-net`)

### 4.1 The model

> **No component other than `NetService` ever calls `bind()` or `connect()`.** Native tools,
> graph nodes, and WASM plugins all request network access through it, against a persisted
> grant. This is enforced by `clippy.toml` `disallowed-methods` on
> `TcpListener::bind` / `TcpStream::connect` / `UnixListener::bind` (§1.3).

One policy point means one place where allowlists, byte budgets, connection leases,
accounting, TLS configuration, graceful shutdown and tracing live. Scattering `bind()` calls
across 14 crates means none of those exist consistently.

### 4.2 `NetService`

```rust
// pii-net/src/lib.rs
pub struct NetService { /* listener registry, conn registry, resolver, tls configs */ }

pub enum ListenSpec {
    Tcp  { addr: SocketAddr, tls: Option<TlsId> },
    Udp  { addr: SocketAddr },
    Uds  { path: PathBuf, mode: u32 },              // mode CHECKED, refuses > 0600
    Serial { dev: PathBuf, baud: u32 },
    Ble  { service_uuid: Uuid },                    // feature `ble`
}

pub enum Protocol {
    Http,          // axum router supplied by the caller
    WebSocket,
    Mqtt,          // rumqttd, embedded broker
    Lines,         // \n-delimited frames
    PostcardRpc,   // MCU / worker (§9)
    Raw,           // caller frames it
    Plugin(PluginId),   // a WASM component implements ProtocolHandler (§10)
}

impl NetService {
    pub async fn listen(&self, ctx: &Ctx, spec: ListenSpec, proto: Protocol,
                        grant: &NetGrant) -> Result<ListenerId, PiiError>;
    pub async fn close(&self, id: ListenerId) -> Result<(), PiiError>;
    pub fn listeners(&self) -> Vec<ListenerInfo>;     // → `pii net ls`, `net.listeners`

    /// Outbound. The ONLY way out of the process.
    pub async fn connect(&self, ctx: &Ctx, target: &Target, grant: &NetGrant)
        -> Result<Conn, PiiError>;
    pub async fn request(&self, ctx: &Ctx, req: HttpRequest) -> Result<HttpResponse, PiiError>;
}
```

### 4.3 Egress discipline — copy zeroclaw almost verbatim

`zeroclaw-plugins/src/{wasi_http.rs,egress.rs}` contains the single best-engineered piece of
security code in the reference set. Adopt it wholesale for **all** Pii egress (not just
plugin egress — the LLM client and the MCP client go through it too):

1. **Grant ≠ reach.** A manifest permission only unlocks a *linker surface*. A separate,
   host-owned `authorize()` runs **per request** against the live policy. A store with no
   egress service denies synchronously before anything is spawned
   (`wasi_http.rs:198-208`).
2. **Resolve DNS exactly once, then pin the `SocketAddr` set, then dial only pinned
   addresses — never re-resolve.** This closes DNS-rebinding/TOCTOU. Use `hickory-resolver`
   so resolution is explicit and controllable rather than hidden inside `reqwest`.
3. **Validate address class after resolution**: reject private / link-local / loopback /
   cloud-metadata (169.254.169.254, fd00:ec2::254) / NAT64-mapped-private, and reject
   **mixed answers** (a hostname resolving to one public and one private address is an
   attack, not a config).
4. **One deadline spans authorize + dial + TLS + handshake.** Not per-stage budgets that
   stack into minutes.
5. **A process-wide connection registry keyed by canonical principal**, so per-principal
   connection leases can't be multiplied by cloning the service (zeroclaw hit exactly this
   bug and fixed it).
6. **Bundled webpki roots for plugin egress**, deliberately not the OS trust store.

```rust
pub struct NetGrant {                 // NO Default impl — zero value is a downgrade
    pub listen:  Vec<ListenScope>,    // ("tcp", 1883..=1883) | ("uds", "$RUNTIME/pii/*")
    pub connect: HostScope,           // host+port allowlist, wildcard-aware
    pub allow_private: bool,          // default false
    pub max_conns: u32,
    pub max_bytes_out: u64,
    pub max_bytes_in: u64,
    pub expires_at: Option<SystemTime>,
    pub version_range: VersionReq,    // pi_agent_rust's PersistedDecision shape
}
// redb:  net_grant : (Principal, GrantKey) -> Postcard<NetGrant>
```

### 4.4 How a WASM plugin gets network access

**Decision: do NOT expose `wasi:sockets` to guests.** Reasons, in order:

1. You inherit `wasi:sockets`' whole address/resource model and lose the ability to attach
   per-plugin byte budgets, connection leases, and the resolve-once-pin discipline — the
   policy would have to be re-implemented *inside* the WASI host impl.
2. zeroclaw reached the same conclusion independently: raw TCP/TLS/STARTTLS is modelled as a
   `SocketClient`-gated **host-mediated** `EgressTransport`, and *no raw guest socket handle
   is ever handed to a plugin*.
3. `wasmtime-wasi`'s p3 sockets impl is currently "experimental, unstable and incomplete".

Instead, a Pii-owned WIT interface, host-mediated, with the host holding the fd:

```wit
interface net {
  use types.{err};
  resource conn {
    write: func(data: list<u8>) -> result<u32, err>;
    read:  func(max: u32)       -> result<list<u8>, err>;   // p3: becomes stream<u8>
    close: func();
  }
  resource listener {
    accept: func() -> result<conn, err>;                     // p3: becomes stream<conn>
    close:  func();
  }
  /// Both are grant-checked; both may return err::denied.
  connect: func(target: string, opts: dial-opts) -> result<conn, err>;
  listen:  func(spec: listen-spec) -> result<listener, err>;
}
```

**And the more important direction — the inverted one.** Most plugins should not *ask for* a
port; they should **register a protocol handler on a port the daemon already owns**:

```wit
interface protocol-handler {                    // the guest EXPORTS this
  on-open:    func(peer: peer-info) -> result<_, err>;
  on-message: func(peer: peer-info, data: list<u8>) -> result<list<u8>, err>;
  on-close:   func(peer: peer-info);
}
world pii-protocol-plugin { import host; import net; export plugin; export protocol-handler; }
```

The host owns the listener, accepts, frames, applies limits, and calls the plugin with
bytes. The plugin never touches an fd. This is the safe, cheap, and far more common case —
"add a new protocol to Pii" without granting anyone a socket. It is also the shape that
lets a plugin be hot-reloaded without dropping connections.

**Note on WASI HTTP**: `wasi:http/outgoing-handler` *is* wired for plugins granted `net`,
but with wasmtime's default send hook **replaced** by Pii's `authorize()`. Never call
`default_hooks()` — zeroclaw's in-code comment says enabling it would attach ungoverned
egress and open SSRF, and they are right. And like zeroclaw, **linking is the enforcement
point**: if `net` is not granted, `wasi:http` is not added to the `Linker` and the component
fails to instantiate with a clear message, rather than failing deep inside a call.

### 4.5 Integration with the graph engine

Two mechanisms, and the distinction matters:

**Listeners are event sources, not nodes.** A listener is bound once, at daemon
configuration time, and publishes onto the bus:

```
net.<listener_id>.open      { peer }
net.<listener_id>.message   { peer, payload }
net.<listener_id>.close     { peer }
```

A reactive graph subscribes with `wait{Event{topic:"net.doorbell.message"}}` — pass 1's
`starter/sense-decide-act` idiom, unchanged. The graph engine never learns what a socket is.

**Outbound is a node kind.** New built-in `net` node (11th kind), `Effects::NET`:

```toml
[[node]]
id = "notify"; kind = "net"; op = "request"
method = "POST"; url = "https://hooks.example/x"; body = "$.nodes.summary.value"
# ports: ok | err | timeout
```

**The rule that prevents a whole class of bugs:**

> **A graph never owns a socket across a `Suspend`.** Connections are owned by `NetService`
> and referenced from the blackboard by `ConnId`. A suspended run may outlive the process;
> a file descriptor may not. On resume, a stale `ConnId` yields `PiiError::NotFound` on its
> `err` port rather than a dangling handle.

---

## 5. Layered memory (`pii-memory`)

### 5.1 The decision

> **Embed `agent-memory-core` as a native crate dependency. Do not reimplement it, do not
> wrap it as a WASM plugin. Build the L0–L3 layering *on top* of it as consolidation
> graphs.**

Pass 1 budgeted ~600 LOC of hand-rolled BM25 as "the concrete cost of the redb choice".
That cost does not need paying: `agent-memory-core` is redb-based **and** pairs it with
tantivy, and it already implements exactly the 3-lane RRF pass 1 §6.3 specified.

What it provides (per its own architecture notes):

```
agent-memory-core/src/
├── store/redb_store.rs   # redb, ACID, 8-table schema
├── vector/               # VectorBackend trait: HnswBackend (default) | QdrantBackend
├── fulltext.rs           # tantivy, real BM25 — no external infra
├── graph.rs              # petgraph co-occurrence graph + GraphRAG query expansion
├── embedder.rs           # Embedder trait: FastEmbedEmbedder | MockEmbedder
├── quantize.rs           # int8 scalar quantization (HnswBackend only)
├── search.rs             # Reciprocal Rank Fusion + temporal decay
├── consolidate.rs        # importance decay, duplicate merge, archive/evict
└── memory.rs             # AgentMemory facade
```

Measured: 3-lane RRF (BM25 + vector + graph) + temporal decay gave **+11% NDCG** over
vector-only. That is a real number from a real corpus, and it retires pass 1's open question.

**Two backends behind pass 1's `Memory` trait**, which is what makes this safe:

```rust
pub enum MemoryBackend {
    Embedded(agent_memory_core::AgentMemory),   // ~/.pii/memory.redb  — default
    Remote  (AgentMemoryRestClient),            // the existing server at :7420 (§7)
    Null,
}
```

`Remote` matters more than it looks: it is the multi-site read path (§7) and it lets a
low-RAM node (a Pi in the garage) use the household memory without carrying tantivy + HNSW.

**Two redb files, one process.** `~/.pii/pii.redb` (journal, graphs, skills, grants) and
`~/.pii/memory.redb` (agent-memory-core's 8 tables). redb's lock is per *file*, so this is
legal, and it isolates a corrupted vector index from the run journal. **Version-align redb
across both** — `pidag` is on `redb 1.x`; whatever `agent-memory-core` uses becomes the
workspace pin, and `cargo deny bans` enforces one version.

### 5.2 The L0–L3 layering

Tencent's DB-Agent-Memory model (Tulving 1972 episodic/semantic split), adapted. The key
property to preserve: **a clear path from abstraction back to ground-truth evidence.**

| Layer | What it is in Pii | Where it lives | Built by | Trigger |
|---|---|---|---|---|
| **L0 — raw** | **The run journal.** `RunEvent` already records every message, tool call, port and cost. | `pii.redb: run_journal` | the engine, free | every node |
| **L1 — atoms** | Facts, preferences, decisions. `Doc{kind: Atom, evidence: (RunId, seq)}` | `memory.redb` | one `llm` node with a structured output schema | every 5 assistant turns, and at session end |
| **L2 — scenes** | Clusters of related atoms with a label | `memory.redb` | **computed**, then labelled | ≥N new atoms in a concept community |
| **L3 — persona** | One document: who the user is, how they work, standing constraints | `memory.redb`, pinned | one `llm` call | every 50 new atoms |

**L0 is the journal — do not duplicate raw conversation into the memory store.** This is a
genuine simplification over Tencent's design, available to Pii only because it is
event-sourced already. It also means L1 extraction reads *structured* input (node/port/cost
traces) rather than a transcript, which is cheaper and higher-signal — the same trick pass 1
applied to distillation (§6.5).

**L2 is computed, not generated.** `agent-memory-core` already maintains a petgraph
co-occurrence graph. A scene is a **community in that graph** (label propagation or
Louvain), not an LLM's opinion about which atoms belong together. One cheap LLM call
*labels* the community. Letting a model decide membership is how you get non-deterministic,
unreproducible, drifting L2 — and you pay tokens for it every pass.

**Retrieval order, per LLM turn:**

```
L3 persona           always resident, ~500 tokens, no query           (hermes' index pattern)
+ L1/L2 via RRF      3 lanes: tantivy BM25 | HNSW vector | graph expansion
                     → RRF k=60 → temporal decay → top-k             (~1200 tokens)
+ L0 drill-down      ONLY on explicit request ("why do you think that?") → journal citation
```

Retrieval happens in `starter/react`'s **prompt-assembly step, not as a separate graph
node** — you want it on every turn, and a node per turn doubles the journal for no
information gain. A `memory` node kind exists for explicit ops (`search|put|consolidate`)
that a graph author wants to place deliberately.

This design is directly motivated by **Context Rot** (Chroma 2025): 18 frontier models lose
30–50% accuracy as context grows, degradation starting around 50 k tokens in a 200 k window.
The conclusion is not "get a bigger window", it is "put less in it". ~1,700 tokens of
retrieved context beats 50 k of transcript.

### 5.3 Embeddings — the constraint pass 1 never confronted

`fastembed` uses ONNX Runtime (`ort`), a **C dependency**. That directly fights the
"single static musl binary, no C deps in the default build" target (pass 1 §0).

```rust
pub trait Embedder: Send + Sync {
    fn dims(&self) -> usize;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, PiiError>;
}
```

| Impl | Build | Recommendation |
|---|---|---|
| `OllamaEmbedder` (HTTP → `/api/embeddings`, e.g. `nomic-embed-text`) | **musl-clean, no C** | **Default.** You are already running Ollama for the local 9B. |
| `FastEmbedEmbedder` | glibc only, `ort` C dep | Feature `fastembed`, glibc builds |
| `CandleEmbedder` (pure-Rust BGE-small / MiniLM) | musl-clean, +~40 MB of model | Feature `candle`; the offline answer if Ollama isn't running |
| `MockEmbedder` | — | tests |

**Degrade honestly:** if no embedder is reachable, retrieval falls back to BM25 + graph
(2 lanes instead of 3) and **says so in a log record and in `sys.status`**. It does *not*
silently return worse results — which is precisely the bug `agent-memory-core`'s own
2026-07 audit found (HNSW was memory-only and never persisted to the `VECTORS` table, so
after every restart retrieval silently degraded to BM25-only). That specific failure becomes
a **test**: write docs, restart, assert vector-lane recall is unchanged.

### 5.4 Consolidation, decay, and one hard budget rule

Adopt `agent-memory-core`'s `consolidate()` (importance decay, duplicate merge, archive,
evict) as-is, run as a scheduled graph (`starter/consolidate`) rather than a background
thread — so it is journaled, budgeted, cancellable and traceable like everything else.

**Temporal decay**: `score *= exp(-λ · age_days)` with per-kind λ. Atoms decay fastest,
scenes slower, skills slowest, persona not at all. Framing worth keeping: *forgetting is not
the bug, it is the compression step* (Ebbinghaus 1885). **Never delete — archive.** Same
rule as hermes' skill lifecycle, and for the same reason.

**The hard rule, learned from a real incident:**

> **No O(n²) candidate-generation pass may dispatch an LLM call per candidate without a hard
> candidate cap AND an explicit `Budget`.**

`agent-memory-core`'s `ConflictDetector` used a loose heuristic (shared_concepts ≥ 1 XOR
negation-anywhere) and produced **3,800 conflict candidate pairs** on a modest workspace,
each triggering an LLM validation call — a multi-million-token burn that had to be killed
mid-run. The fix tightened it to `MIN_SHARED_CONCEPTS=2`, `MIN_CONCEPT_JACCARD=0.15`,
`NEGATION_WINDOW=8` (negation must be within 8 tokens of the shared concept, not anywhere in
the doc) and `num_predict` 1024→256: **3,800 → 18 candidates, ~844× token reduction.**

This is exactly what pass 1's `Budget` / `BudgetExhausted` machinery exists for, and it was
never wired to memory. Wire it: `starter/consolidate` declares `max_llm_calls` and
`max_tokens` in `GraphLimits`, and the run terminates with `Terminal::BudgetBreach` (§6.2)
rather than quietly spending. Also port the tightened constants — they are hard-won.

### 5.5 Vector backend — final answer

**Embedded `hnsw_rs` with int8 quantization** (agent-memory-core's default). Not Qdrant (a
second process defeats the single-binary claim; keep the `QdrantBackend` feature for a
future large deployment). Not sqlite-vec (wrong database). Not `usearch` (C++ dep, same musl
problem as fastembed).

**Correction to pass 1 §6.3**: brute-force i8 scan was a sound default *given no index
existed*. One does. Keep brute force as the small-corpus path — below ~10 k docs the HNSW
build cost dominates and a SIMD scan wins — and select automatically on corpus size, with
the threshold in config and the choice reported in `sys.status`.

**Keep pass 1's RRF over weighted normalisation.** `agent-memory-core` independently landed
on RRF too. Confirmed twice, no calibration constants, one line.

---

## 6. agent-memory, pidag, orki — subsume, embed, or steal

### 6.1 What "a Pii plugin" actually is — four tiers

Pass 1 implied one answer (WASM component). There are four, and picking the wrong tier is
how a project acquires a plugin system nobody uses.

| Tier | What | Use when | Cost |
|---|---|---|---|
| **T1 — native crate** | Compiled in, feature-gated, implements a kernel trait | Needs sockets, hardware, tantivy/HNSW, or is hot-path | Binary size, and it is *your* code forever |
| **T2 — WASM component** | WASI 0.2 component implementing a kernel trait (§10) | Untrusted, third-party, or **machine-generated** | Sandbox overhead, WIT ABI churn |
| **T3 — subgraph** | A named `GraphSpec`. No code at all. | Composition of existing nodes | ~zero. **This should be the default answer.** |
| **T4 — external process** | MCP server, A2A agent, UDS worker, MCU | It already exists and speaks a protocol | Process supervision, IPC latency |

**T3 is the one people forget.** Most of what users will call "a plugin" — a morning
briefing, a build-and-test loop, a thermostat rule — is a graph, not code. Say so in the
docs, or you will get a plugin registry full of things that should have been three nodes.

### 6.2 `pidag` → **subsumed**, but steal four things

`pidag` is a single crate (~22.8 k LOC, `src/scheduler/execute.rs` 3,012 lines,
`src/core/dag.rs` 1,192, `src/store/redb_store.rs` 1,185), redb-backed with **8 tables**
(`runs, nodes, events, artifacts, node_timings, event_seq, meta, budget`), append-only event
log projected into current state, checkpoint/resume, MCP server over `rmcp`, and an A2A
*worker* (client-only; the A2A server side was deliberately deleted).

Its mechanics are a **strict subset** of pass 1's graph engine: static DAG, `depends_on`,
retries, model fallback chains, bounded concurrency, subgraph inlining, `for_each` fan-out,
budget ceilings, event-sourced resume. Pass 1's engine adds bounded cycles, data-routed
ports, `Suspend`, capability contexts, and isolation policies. **Pii's engine subsumes
pidag. Do not run pidag inside Pii; do not port pidag's scheduler.**

Four things pass 1's engine is missing that pidag has and that are genuinely good:

**1. `verify` / `verify_pre` as a delta-based effect check.** The best idea in pidag, and it
is missing from pass 1 entirely. The insight: *the dominant LLM failure mode is claiming
success while changing nothing.* pidag measures a **delta across the node** (hash before,
hash after), not a state snapshot — and it learned the hard way that snapshotting produces
false positives on resume when the tree is already dirty.

```rust
pub struct NodeSpec { /* ... */ pub verify: Option<Verify>, pub verify_pre: Option<Verify> }
pub enum Verify {
    Delta  { paths: Vec<PathGlob> },     // blake3 before/after; fail if unchanged
    Shell  { cmd: String, expect: ExitCode },
    Schema { schema: Arc<Value> },       // output must validate
    Critic { model: ModelRef, rubric: String },   // model-as-judge — see below
}
```
A failed `verify` routes to the node's `err` port with `PiiError::Invalid("no effect")`. This
also feeds the trajectory's `outcome.verifier` block (pass 1 §6.6) with a **programmatic**
signal, which is exactly what that field was specified to need.

**2. `after` edges — ordering without dependency.** pidag distinguishes `depends_on`
(blocking, propagates upstream failure) from `after` (ordering-only: wait for terminal
state, run regardless of outcome). Pass 1's single edge type conflates these, which makes
"always run the cleanup/validator node" inexpressible. Add:

```rust
pub struct Edge { pub from: NodeId, pub port: PortName, pub to: NodeId, pub kind: EdgeKind }
pub enum EdgeKind { Data, Order }   // Order: fires on ANY terminal port, passes no value
```

**3. Budget in observable units, with a distinct terminal state.** pidag counts
`max_model_calls` and `max_tokens` — deliberately **not dollars**, because it drives
free-tier and self-hosted models where no price table exists. It reserves *before* dispatch
(first-breach-wins, sticky), records usage after, allows in-flight dispatches to finish
(documented overshoot ≤ concurrency), and exits with **status 3** distinct from 1, because
the recovery action differs: raise the ceiling vs fix the node.

Pass 1 has `PiiError::BudgetExhausted` but folds breach into failure. Split it:
`Terminal::{Success, Failure, BudgetBreach, Cancelled, Expired}`, `pii graph run` exits 3 on
breach, and `RunHeader.breach: Option<BudgetBreach>`. Keep USD as an *optional* additional
ceiling for priced providers; make calls+tokens the primary.

**4. `Quorum` / `Critic` — ensemble adjudication.** A node that dispatches no worker and
adjudicates over sibling outputs. Add as a `route` variant. **Flag it honestly as
unproven**: pidag's own `docs/ARCHITECTURE.md` says the critic path has not been validated
against a live model. Ship it behind a config flag with a measured comparison, not as a
default.

**One thing pidag has that Pii should explicitly *not* copy**: the thesis that "the
orchestrator never reasons at runtime". It is a good discipline for a *batch spec-execution*
tool and it is why pidag works. But Pii's whole premise is an interactive agent whose
`agent` node reasons continuously. The correct translation is narrower and worth stating:
**the scheduler makes no judgements** — routing is data (`Outcome::Next{port}`) and guards
are side-effect-free expressions. Pass 1 already has this. Keep it.

**Migration**: `cargo xtask pidag-import <dag.toml> > graph.toml`. pidag keeps running
untouched until Pii clears Phase 2 exit; then it retires. It is not a dependency, not a
plugin, and not a subgraph — it is a predecessor.

### 6.3 `agent-memory` → **T1, embedded** (see §5)

Not a WASM plugin: it needs tantivy, HNSW, redb and (optionally) ONNX — none of which belong
inside a sandbox, and all of which are trusted first-party code. Not a T4 external process
by default either, because a second process holding a second redb file defeats the
single-binary claim. **But keep the `Remote` backend** — the existing server at
`10.10.10.217:7420` (REST + MCP, `GET /v1/memory/<topic>/search?q=&k=`,
`POST /v1/memory/<topic>/insights`, `GET /v1/memory/_topics`, plus a web UI) is both a
migration path and the multi-site read path (§7).

Note its API shape is deliberately REST + MCP rather than gRPC, "because LLM tool-calling
requires text descriptions" — a good reason, and it means Pii can expose the same surface
through `pii mcp` with no adaptation.

### 6.4 `orki` → **adopt two patterns, one measurement, and one warning**

`orki` is a standalone 7-crate Cargo workspace (`orki-core, orki, orki-api, orki-mcp,
orki-daemon, orki-cli, orki-messaging`), ~29.6 k LOC, v2.0.0-alpha, JSON pipeline schema
v2.0. **Do not adopt the implementation.** Adopt:

**Pattern 1 — racing.** N alternatives dispatched concurrently; first success wins; the rest
are cancelled. Trivial on pass 1's scheduler (`Ctx::cancel` already propagates). Add as an
11th/12th node kind:

```rust
Race { arms: Vec<NodeId>, win: WinCondition }   // FirstOk | FirstOkOrBestOf(n) | Quorum(k)
// ports: ok (with `winner` in the value), all_failed
```
This is the concrete mechanism for "try the local 9B and the frontier model in parallel when
latency matters more than cost", and it composes with §9's tiering.

**Pattern 2 — role routing.** orki's `HybridAgentPool` routes `agent_role: llm|agent` →
`PiAgentPool` and `shell|http|docker|mcp` → `SimpleAgentPool`. In Pii this is already just
`NodeKind`. **Nothing to build** — worth noting only because it confirms the node-kind
taxonomy is the right factoring.

**Measurement 1 — blackboard durability costs 22×.** orki benchmarked a write-through
durable blackboard (atomic temp+rename JSON per key) at **2,310 ops/sec at 32-way fanout**
versus **50,279 ops/sec** for an in-memory context. Pass 1 specified an in-memory blackboard
with a snapshot every 32 events; this is the evidence that the decision is right. **Do not
make the blackboard write-through.**

**Warning 1 — the diamond bug.** orki's `graph_flow` executor has a confirmed defect with
two parts:
1. **single-task fanout** — the scheduler dispatches only one ready task per iteration even
   when several are ready, silently starving fan-out siblings;
2. **any-predecessor convergence** — a task with multiple `depends_on` predecessors is
   scheduled when **any** one completes, not when **all** do.

Impact: a `setup → {architect, builder} → fusion` pipeline silently skipped `builder` on
every run *and fabricated provenance*. The workaround was to force a linear chain.

> **Therefore: Phase 1 exit criteria gain a diamond test.** `entry → {a, b} → join`; assert
> (i) `a` and `b` both executed, (ii) they overlapped in time (real parallelism, not
> serialised), (iii) `join` executed **exactly once**, (iv) `join` started after both
> finished. Four assertions, ~40 lines, and they catch a bug that shipped in a sibling
> project and produced *confidently wrong output*. This is the cheapest high-value test in
> the whole plan.

**Three more ops lessons from orki's hardening specs, all free to adopt:**
- `kill_on_drop(true)` on every spawned child — tokio's `Child` does **not** kill on drop,
  and orki orphaned processes because of it. Add to the `Ctx::proc()` wrapper so it cannot
  be forgotten (`Command::spawn` is already banned by clippy, §1.3).
- 29 production `unwrap()`s had to be removed. Already handled: `unwrap_used = "deny"` (§1.3).
- **Cheap workhorse models repeatedly wrote fake tests** that hid real bugs — including a
  fabricated `ExecutionResult` hardcoding `success: true`. This is why §1.6 sets no global
  coverage gate and why every phase exit criterion in §13 is **behavioural** ("kill the
  process, assert no duplicate node execution") rather than "tests pass".

### 6.5 Summary table

| Tool | Verdict | Action |
|---|---|---|
| `agent-memory-core` | **Embed (T1)** | Workspace dependency; `Memory` trait wraps it; keep `Remote` backend |
| the agent-memory server | **Keep as T4** | `MemoryBackend::Remote`; also the sync target (§7) |
| `pidag` | **Subsume** | Steal `verify`/`after`/budget-terminal/quorum; write an importer; retire it after Phase 2 |
| `orki` | **Do not adopt** | Steal `race`; adopt the blackboard measurement; **write the diamond test** |

---

## 7. Multi-site sync (`pii-sync`)

### 7.1 What syncs

| Data | Syncs? | Conflict class | Mechanism |
|---|---|---|---|
| **Trajectories / run journals** | **Yes — highest priority** | **None possible** | Append-only, keyed `(SiteId, RunId)`. This is the 9B training corpus and it is the *easiest* thing to sync. Ship it first. |
| Memory docs (L1/L2) | Yes | Additive | Dedupe by content hash; LWW on `importance`/`last_used` (they are hints) |
| L3 persona | Yes | Real | One doc per site; merge is an explicit `starter/merge-persona` graph, not automatic |
| Skill bodies | Yes | **Real** | Append-only `skill_ver` + `needs_merge` flag (§7.3) |
| Graph definitions | Yes | **None for bodies** | Content-addressed by `GraphHash` — bodies are immutable and can never conflict. Only the `name → hash` pointer can, and that is a 3-way choice the user makes. |
| Plugin manifests + wasm blobs | Yes | None | Content-addressed by hash |
| Config | **No** | — | Site-local. Ports, paths, hardware differ by definition. |
| Secrets / API keys | **Never** | — | Site-local, and this is not negotiable |
| Grants (capability, net, plugin) | **No** | — | A grant given at home is not a grant at the office |
| Live run state, suspended runs, timers | **No** | — | A suspended run belongs to its site's process |
| Logs | Optional, opt-in | None | Append-only; usually you want them local |
| Device/listener/MCU state | **No** | — | Physical, site-bound |

### 7.2 The mechanism — a sync log, not a distributed database

> **Do not CRDT the whole store.** Every record carries a `SiteId` and a Hybrid Logical
> Clock. redb stays single-writer *per site*. Sync is **pull-based import/export of an
> append-only oplog** — the same model as `git fetch`, and roughly 300 LOC.

```
# pii.redb
sync_log    : (SiteId, seq u64)   -> Postcard<SyncOp>   # append-only, local writes only
sync_cursor : (PeerId, SiteId)    -> u64                # how far we've pulled from each peer
sync_peer   : PeerId              -> Postcard<PeerRec>  # url, token, pubkey, last_seen, build
```

`SyncOp` is `{ hlc, site, kind: Upsert|Archive|Pointer, table, key, value_hash, value }`.
Pulling is one RPC on the existing control protocol (§3):

```
sync.pull { since: [(site_a, 4021), (site_b, 118)], limit } -> { ops[], next: [...] }
```

**Why not full CRDT (automerge / Loro / diamond-types):** the overwhelming majority of Pii's
data is append-only or content-addressed, where a CRDT buys nothing and costs a large
dependency, a new storage format, and unbounded operation history. Loro is genuinely
excellent (Fugue algorithm, 2–5× smaller encoding than Yjs/Automerge, MIT, shallow
snapshots, bring-your-own transport) — but it is the right tool for **concurrently edited
rich text**, which in Pii is exactly one thing: skill bodies.

### 7.3 Skill conflicts — the only genuinely hard case

Pass 1 already made skills append-only versioned (`skill_ver : (SkillId, ver) -> Skill`).
Two options:

- **(b) Versioned with a conflict flag** — concurrent edits produce *two versions* plus
  `needs_merge`, surfaced in `pii skill ls` and the TUI. The user (or a `starter/merge-skill`
  graph) resolves it. **Recommended.** A skill is edited by one human on one machine ~99% of
  the time; a CRDT is a large permanent dependency for a rare event.
- **(a) Loro** — `LoroText` for the body, `LoroMap` for frontmatter. Genuinely better UX for
  the rare case. **Only if (b) proves annoying in practice**, and the migration is
  mechanical because the body is just a string.

### 7.4 Transport, trust, and the failure mode to design against

- Same `pii-proto` (§3) over TLS. `pii sync add <url>` runs the pairing flow; peers are
  **mutually** authenticated (each stores the other's pubkey).
- **Not transitive.** No gossip, no eventual convergence across a mesh in v1. Star or
  explicit pairwise. Gossip is where "simple sync" becomes a distributed systems project.
- **Redaction runs before export, on the sender**, never on the receiver. Pass 1 §6.6's
  rule generalises: a trajectory leaves a machine already scrubbed, with
  `redaction.policy` recorded.
- `pii sync status` shows, per peer: `site`, `pii` version, **build sha**, `pii-proto`
  version, cursor positions, last error, lag.

**The failure mode to design against, from this workspace's own history:** a three-machine
topology (lnx build host / container / mac) where a Containerfile `COPY`ed from a stale
snapshot missing a whole `src/` directory — so rebuilds silently compiled old code and
reported success. The generalisation:

> **`sync.pull` refuses ops from a peer whose `pii-proto` version is incompatible, and warns
> loudly when the peer's build sha differs from the local one.** Silent version skew that
> "works" is worse than a hard refusal.

Also worth copying: the `_sync-state` pattern — a small git-tracked repo recording what has
been synced between machines. Pii's `sync_cursor` table is the machine-readable version;
exporting it to a git-tracked file gives a human-auditable one for free.

### 7.5 The single-writer question, answered

Pass 1 flagged "redb single-writer across sites" as a risk. It is not one, because **sites
never share a database file.** Each site owns its redb exclusively; sync moves *operations*
between them. The single-writer constraint is a per-process, per-file property and it is
untouched by replication. The real constraints are:

1. **`SiteId` must be in every syncable key or record.** Retrofitting it is a migration, so
   put it in from Phase 1 even though sync ships in Phase 6. This is the one thing that must
   be done early.
2. **HLC on every write.** Also cheap, also painful to retrofit.

---

## 8. Async task model — callbacks, not blocking loops

### 8.1 The three streams, and the rule that separates them

```
1. RunEvent      durable   redb `run_journal`    replayable, authoritative, survives crash
2. Event         live      tokio::broadcast      superset: + deltas, progress, logs, net
3. Notification  filtered  control protocol      what a subscribed client actually sees
```

> **The rule: anything that must survive a crash goes in the journal; anything only useful
> live goes on the bus. Nothing goes in both.**

LLM token deltas, tool stdout chunks, and progress percentages are bus-only — journaling
them would multiply the journal by 100× for zero replay value (the final content is already
in `NodeFinished`). Node completions, suspensions, approvals, and terminals are journaled.
This single rule resolves what zeroclaw never decided (it streams everything, untyped, over
WS) and what hermes never built (cron streams nothing at all).

### 8.2 Nothing blocks

> **`session.prompt` and `run.start` return a `RunId` immediately.** There is no blocking
> call anywhere in the control protocol. `run.await` exists as ergonomic sugar for
> subscribe-until-terminal with a timeout, and **it is never used internally.**

Hermes' cron is the anti-pattern to avoid: a 60-second-poll daemon thread takes a
cross-process `fcntl` lock, dispatches to a `ThreadPoolExecutor`, and the job-runner thread
then **blocks on `future.result()`** while a *separate* watchdog thread polls
`agent.get_activity_summary()` to enforce an idle timeout. `run_conversation` is invoked
with no `stream_callback`, so a cron job produces **no live progress at all** — one final
message on completion. Job control is conversational ("stop reminder X") against a
file-backed job store rather than a typed API.

### 8.3 The job registry

```rust
pub struct RunHandle {
    pub run_id: RunId,
    cancel:   CancelToken,
    progress: broadcast::Receiver<Arc<Event>>,
    done:     watch::Receiver<Option<Terminal>>,
}
pub struct RunRegistry { /* DashMap<RunId, RunHandle> */ }
```
Exposed as `run.list | run.get | run.cancel | run.attach`, and as
`pii run ls|cancel|attach <id>`. This is the typed handle hermes lacks; `execution_id`
(hermes' `cron/executions.py`, with `recover_interrupted_executions`) is the closest
analogue and it is file-based.

### 8.4 The event taxonomy

Adopt hermes' callback *menu* — it is a good, complete list, learned from production — but
typed, and unified into one enum instead of eleven separately-registered callbacks
(`stream_callback`, `thinking_callback`, `step_callback`, `reasoning_callback`,
`tool_progress_callback`, `tool_start_callback`, `tool_complete_callback`,
`interim_assistant_callback`, `status_callback`, `notice_callback`, `event_callback`).

```rust
// pii-proto/src/event.rs   — server → client notifications
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    RunStarted   { run: RunId, graph: SmolStr, seq: u64 },
    NodeStarted  { run: RunId, node: NodeId, kind: SmolStr, seq: u64 },
    NodeProgress { run: RunId, node: NodeId, pct: Option<f32>, msg: SmolStr },   // bus-only
    LlmDelta     { run: RunId, node: NodeId, text: String },                     // bus-only
    LlmThinking  { run: RunId, node: NodeId, text: String },                     // bus-only
    ToolStarted  { run: RunId, node: NodeId, tool: SmolStr, args_digest: Hash },
    ToolProgress { run: RunId, node: NodeId, chunk: String },                    // bus-only
    ToolFinished { run: RunId, node: NodeId, ok: bool, truncation: Option<Truncation> },
    NodeFinished { run: RunId, node: NodeId, port: PortName, dur_ms: u32,
                   tokens: Option<TokenUsage>, cost_micro_usd: u64, seq: u64 },
    ApprovalRequested { run: RunId, approval: ApprovalId, prompt: String,
                        policy: SmolStr, expires_at: Option<SystemTime> },
    Suspended    { run: RunId, node: NodeId, reason: SuspendReason, seq: u64 },
    Resumed      { run: RunId, key: ResumeKey, seq: u64 },
    RunFinished  { run: RunId, terminal: Terminal, seq: u64 },
    Log          { record: LogRecord },
    NetMessage   { listener: ListenerId, peer: SmolStr, len: u32 },
    McuToolCall  { mcu: McuId, tool: SmolStr, args: Value, confidence: f32 },
    PluginEvent  { plugin: PluginId, topic: SmolStr, payload: Value },
    Lagged       { sub: SubId, dropped: u64 },       // ← never drop silently
}
```

Two disciplines from hermes worth keeping verbatim:
- **Best-effort delivery; a failing subscriber never affects the emitter.** Hermes swallows
  and debug-logs callback exceptions. Pii's equivalent: a slow client gets `Lagged`, not
  backpressure onto the graph.
- **`thinking` is a status line, and empty string means "clear it".** Small, and it is what
  makes a TUI status line not flicker.

### 8.5 Backpressure, honestly

- Per-subscription bounded queue (default 1,024).
- On overflow: **drop oldest, increment a counter, emit `Lagged{dropped}`**. Never block the
  graph on a slow client; never grow unbounded; **never drop silently**. This is exactly
  zeroclaw's log-writer discipline (bounded `sync_channel(1024)`, drop with a warn) applied
  to subscriptions.
- The journal is unaffected — a client that misses events can always replay durable ones.

### 8.6 Reconnect and replay

```
event.subscribe { filter, since_seq: 4021 }
   → server replays journaled events with seq > 4021 from run_journal
   → then switches to live
   → and sends Lagged{dropped: 0} or a `live_only_gap: true` marker if bus-only
     events were missed in between (they cannot be replayed, and the client is told so)
```
Being explicit that live-only events are unrecoverable is better than pretending the stream
is complete.

### 8.7 Cron and reactive triggers are graphs, not a subsystem

Pass 1 already has `timer_wheel : (fire_at_ms, RunId) -> ResumeKey` and
`resume_index : ResumeKey -> RunId`. Therefore:

> **A cron job is a graph whose entry node is `wait{Timer{cron}}`. A reactive automation is
> a graph whose entry node is `wait{Event{topic}}`. There is no scheduler subsystem.**

Firing = a single redb write transaction that appends `Resumed` to the journal and pushes
the run onto the ready set. Cross-process safety comes from redb's write lock — no `fcntl`
file lock, no 60-second poll, no thread pool, no separate job store. And because it is a
graph, a cron job **streams progress like everything else**, which is the specific thing
hermes cannot do.

Timer accuracy: a single `tokio::time::sleep_until` on the earliest entry in `timer_wheel`,
rearmed on every insert. Not a poll loop.

### 8.8 Approvals — the shape that pays for all of this

```
gate node → Outcome::Suspend{ Approval{policy, prompt} }
  → journal: Suspended{ key: ResumeKey::Approval(id) }
  → bus/clients: Event::ApprovalRequested
  → ANY client answers: approve.resolve{ id, decision, note }
  → journal: Resumed{ key, payload_hash }  → scheduler continues at the gate's port
```

The daemon can be restarted between the request and the answer. The answer can arrive from
the TUI, the web dashboard, or a Telegram reply — because the `ResumeKey` is in redb, not in
a future. **A 3-day approval and a 200 ms tool call are the same shape.** That was pass 1's
claim; §8 is the delivery mechanism that makes it true.

---

## 9. Tiny models on MCUs (`pii-mcu`)

### 9.1 The facts, stated plainly before the design

**Needle 2** (Cactus Compute, Apache 2.0): 45 M parameters, 14 MB binary, ~28 MB peak
session RAM. It is a **tool-calling dispatcher with grammar-constrained decoding**, not a
chatbot — you predefine tools as JSON Schema, prompt in natural language, and it emits a
schema-valid call. 98.3% correct function name on Mobile Actions, beating models 5–7× larger.
Architecture tricks: n-gram hashed lookup tables replacing part of the matrix multiply,
Hadamard transform replacing MLP layers, and **trained at 2-bit from the start** rather than
quantised afterwards.

**The concrete ESP32 port** (`github.com/andrisgauracs/needle-2-esp32`) — and this is where
the marketing meets the oscilloscope:

| Fact | Value | Consequence for Pii |
|---|---|---|
| Board | ESP32-S3 **N16R8** — 16 MB flash, **8 MB octal PSRAM** | 2 MB or quad-PSRAM variants **do not work**. Not portable across ESP32 variants. |
| Model | `needle2.cact`, 13.7 MB at flash offset `0x210000`, mmap'd | ~3 min to flash; the model is not something you swap per task |
| Speed @240 MHz | **534 ms/token = 1.87 tok/s**; ~25 s per request, ~47 s with reasoning; ~51 s one-time boot prime | **Never on a latency path.** Compute-bound, not memory-bound. |
| Context | **256 tokens** | The binding constraint. Fewer tools = more room = better accuracy. |
| KV cache | 3.5 MB | |
| Tool schema | JSON Schema, `string`(+`enum`) / `integer` / `number` / `boolean` only; **nested objects and arrays rejected at compile time**; `minimum`/`maximum` honoured | Pii's `ToolSpec`s must be *downcast* to this subset |
| Output guarantee | Grammar-constrained: fields "guaranteed present, correctly typed, within declared bounds — no validation needed" | Parsing is infallible. This is the real value. |
| Confidence | Reported per call, observed 0.46–0.98, **measures groundedness, not correctness**; unit conversions score lower | Usable as an escalation signal, **not** as a correctness proof |
| I/O in the demo | Serial console over the UART bridge; no Wi-Fi, no MQTT | Pii must supply the transport |

**Hardware reality check for this user:** the primary board in `/projects/nrf52-lab` is a
Pro Micro **nRF52840** — 1 MB flash, 256 KB RAM, Cortex-M4F, embassy + `defmt` + probe-rs.
**Needle 2 cannot run on it**, not close. Using Needle 2 requires buying an ESP32-S3-N16R8
(~$10). That is a decision, not a detail (§14, decision 6). The nRF52840 remains excellent
as a **sensor/actuator edge node** speaking `postcard-rpc` over USB or BLE — it just is not
where the model lives.

The 28.9 M-parameter TinyStories model running on an ESP32-S3 at 9.5 tok/s (Per-Layer
Embeddings, 25 M params in flash, ~560 KB in SRAM, 14.9 MB after 4-bit quantisation) is the
useful counter-example: **5× faster and completely useless here**, because it cannot make
tool calls. Needle 2 is right *because of its shape*, not its size.

### 9.2 What an MCU model is actually for

Be honest about the value, because 25 seconds per request kills the obvious pitch:

- **Offline resilience.** A wall panel or voice button keeps working when the daemon is
  down, the network is out, or the house is being rewired.
- **Privacy.** The utterance never leaves the room.
- **Zero marginal cost.** Unlimited invocations, no tokens, no rate limit.
- **It is the smallest tier of the tiering story**, and it makes the escalation ladder
  *physical* rather than a config file.

It is **not**: faster, cheaper-than-local-9B in latency terms, or capable of anything
requiring more than 256 tokens of context.

### 9.3 Transport — `postcard-rpc`

**`postcard-rpc` 0.12.1** (June 2026, actively maintained): `no_std` + serde, `WireTx`/`WireRx`
traits implementable over USB/serial/TCP/I²C/UART, an **Endpoint** model (request→response)
and a **Topic** model (one-way pub/sub), schema + URI hashing for compile-time-checked
message identity, and a `host_client` module for the PC side. It is purpose-built for exactly
this host↔MCU relationship.

```rust
// pii-abi/src/mcu.rs  — the message set, shared between firmware (no_std) and daemon
endpoint!(GetStatus,   (),             McuStatus,   "pii/mcu/status");
endpoint!(SetToolTable, ToolTable,     Ack,         "pii/mcu/tools/set");
endpoint!(Infer,        InferReq,      InferResp,   "pii/mcu/infer");
topic!   (ToolCallOut,  ToolCallMsg,                "pii/mcu/toolcall");   // MCU → host
topic!   (SensorOut,    SensorMsg,                  "pii/mcu/sensor");     // MCU → host
topic!   (LogOut,       McuLog,                     "pii/mcu/log");        // MCU → host
```

Physical layers, in preference order:
1. **USB / UART serial** via **`serial2-tokio`** — *not* `tokio-serial`, which is
   unmaintained and openly seeking adoption. This is the demo board's native path.
2. **TCP over Wi-Fi** for mains-powered nodes (`postcard-rpc` over a TCP `WireTx`/`WireRx`).
3. **BLE** via `btleplug` (host central, actively maintained) for battery nodes — including
   the nRF52840 lab boards.
4. **MQTT** as the interop fallback: topic `pii/mcu/<id>/toolcall`, same payload. Use this
   when the device already speaks MQTT (Zigbee2MQTT, ESPHome, Tasmota) and you do not
   control the firmware.

### 9.4 Integration — an MCU is a worker of kind `mcu`

Pass 1's `pii-worker` assumed UDS + JSON-RPC. Generalise its transport:

```rust
pub trait WireTransport: Send + Sync {          // AFIT — one impl per build, not dyn
    async fn send(&self, frame: &[u8]) -> Result<(), PiiError>;
    async fn recv(&self) -> Result<Bytes, PiiError>;
}
// impls: UdsJsonRpc (local workers), PostcardSerial, PostcardTcp, PostcardBle, MqttBridge
```

Everything else in pass 1 §4.3 is unchanged and applies directly: `hello` with an **ABI
version and build sha** (mismatch = hard refusal), 5 s health poll, 3 consecutive misses →
reconnect, exponential backoff 100 ms → 30 s, circuit-break to `Degraded` after 5 failures
in 60 s, in-flight calls fail with `PiiError::WorkerDied` (retryable). The MCU appears in
`pii top` and `sys.status` next to every other worker.

**Tool-table synchronisation** — the interesting part:

```
pii mcu sync <id>
  1. take Pii's ToolSpec set for that MCU's declared scope
  2. DOWNCAST to the Needle subset: flatten one level, drop non-scalar params,
     drop tools that cannot be expressed → report exactly what was dropped, and why
  3. rank by journal usage (§9.6) and truncate to fit the 256-token context
  4. emit tools/<id>.json, regenerate the C TOOL_TABLE, rebuild, flash
     — tool table only; the 13.7 MB model partition is untouched
```

Step 2 must be **loud**. A silently-dropped tool is a capability the user thinks they have.

### 9.5 The escalation ladder, concretely

An MCU's `ToolCallOut` becomes a bus event (`mcu.<id>.toolcall`, §4.5), consumed by a
reactive graph:

```toml
name = "mcu.dispatch"
[[node]] id="listen"; kind="wait"; event="mcu.panel.toolcall"
[[node]] id="triage"; kind="route"; on="$.nodes.listen.value"
rules = [
  { and = [{ gte = { path="confidence", v=0.80 } }, { in = { path="tool", set="@local_safe" } }], port="direct" },
  { gte = { path="confidence", v=0.50 }, port="local9b" },
  { port="frontier" },
]
[[node]] id="direct";   kind="tool"                       # 0 LLM tokens
[[node]] id="local9b";  kind="llm"; model="tier:small"    # Ollama, ~1 s
[[node]] id="frontier"; kind="agent"; model="tier:large"
```

| Tier | Model | Latency | Cost | Job |
|---|---|---|---|---|
| **Nano** | Needle 2, 45 M, on the MCU | ~25 s | 0 | Intent → schema-valid tool call. Offline. No reasoning. |
| **Small** | local 9 B via Ollama | ~1 s | 0 | Routing, running promoted graphs, tool-arg repair, classification |
| **Large** | frontier API | 2–30 s | $ | Novel reasoning, multi-file code, planning, distillation |

`ModelTier` (ported from `yantrikclaw/src/tools/tier.rs`) gains a fourth variant **below**
`Tiny`:

```rust
ModelTier::Nano => ModelCapabilityProfile {
    max_tools_per_prompt: 8,
    tool_call_mode: ToolCallMode::GrammarConstrained,   // NEW — output is guaranteed valid
    max_agent_steps: 1,                                  // it is a dispatcher, not a loop
    repair_attempts: 0,                                  // repair is the next tier's job
    confidence_threshold: 0.80,
}
```

**Offline degradation** is a firmware behaviour, not a Pii one: if the MCU cannot reach the
daemon, it executes its own `local_safe_set` directly (LED, relay, servo — the demo's
`ACT <name> k=v` handler pattern) and buffers a `ToolCallOut` for replay on reconnect.

### 9.6 The self-improvement loop at the smallest tier

This is the part that makes MCUs more than a gadget:

1. Every MCU dispatch is journaled with `confidence`, the tier that finally answered, and
   whether the outcome was verified (§6.2's `Verify`) — pass 1's `escalation` trajectory
   block, populated for free.
2. `pii mcu tune <id>` reads the journal and:
   - **prunes the tool table** to the tools actually used (the 256-token context is the
     binding constraint, so pruning directly buys accuracy);
   - **fits the confidence thresholds** — find the threshold that maximises
     (direct-executions) subject to (verified-error-rate < ε), per tool;
   - reports what it would change, and applies only with `--apply`.
3. Over time this is a measurable, LLM-free optimisation of a real system, and it produces
   labelled escalation data — which is the single highest-value signal for the 9 B fine-tune
   (pass 1 §6.6).

**Phase-6 exit criterion:** ≥100 MCU dispatches journaled; `pii mcu tune` demonstrably raises
the direct-execution rate at constant verified-error rate; and with the daemon stopped, the
MCU still actuates its local safe set and replays the buffered calls on reconnect.

---

## 10. WASM modules as first-class citizens (`pii-wasm`)

### 10.1 p2 now, p3 by design — the decision

**WASI 0.3.0 shipped 2026-06-11**: native async in the Component Model (`async func`,
`stream<T>`, `future<T>`), `wasi:io` **removed entirely** (absorbed into the Canonical ABI),
and `wasi:cli / http / sockets / filesystem / clocks / random` all promoted to stable.
Wasmtime 46+ implements final 0.3 and runs 0.2 and 0.3 components from the same binary,
dispatching per component. Current wasmtime is **48.0.1**.

**But** `wasmtime-wasi 48.0.1`'s `p3` module is documented verbatim as *"Experimental,
unstable and incomplete implementation of wasip3 version of WASI."* The **spec** is stable;
the **host implementation** is not.

> **Decision: build on p2. Write Pii's own WIT interfaces so that migrating to p3 is
> mechanical. Re-evaluate at wasmtime ~52 or when the `p3` module drops the "experimental"
> label, whichever is first.**

Concretely, "mechanical" means: **do not use `wasi:io` types in Pii's own interfaces.** Where
p3 will give a `stream<T>`, use an explicit Pii-owned resource with `read(max) -> list<u8>`
today (§4.4). Then the p3 port is a WIT edit plus a host adapter, not a redesign of every
plugin.

This is the biggest single correction to pass 1, which planned around wasmtime 47 / WASI 0.2
without knowing 0.3 existed.

### 10.2 The world structure — one common interface, one world per role

The claim "WASM modules are first-class citizens" means precisely: **any kernel trait can be
implemented by a component, and the host has exactly one code path per trait.** zeroclaw
proves this works for `Channel`, `Tool`, and `Memory` (`wit/v0/{channel,tool,memory}.wit`;
`WasmChannel` at `wasm_channel.rs:381-886` is a macro-generated adapter over
`bindings.zeroclaw_plugin_channel().call_*()`). Pii generalises the set.

```wit
package pii:plugin@0.1.0;

/// Shared vocabulary. Every world uses this.
interface types {
  type json = string;
  flags effects { pure, reads-fs, writes-fs, net, spawns-proc, mutates-external, long-running }
  flags roles   { tool, channel, memory, node, protocol-handler, transform, observer }
  variant err { denied(string), invalid(string), timeout, budget, upstream(string), internal(string) }
  record span { trace-id: string, span-id: string }        // W3C traceparent halves (§2.4)
}

/// EVERY plugin exports this. It is what makes the registry uniform.
interface plugin {
  use types.{json, roles, err};
  info:       func() -> plugin-info;            // name, version, abi
  declared-roles: func() -> roles;              // which of the role interfaces are exported
  configure:  func(cfg: json) -> result<_, err>;
  health:     func() -> bool;
  shutdown:   func();
}

/// EVERY plugin imports this.
interface host {
  use types.{json, err, span};
  log:        func(level: level, event: json);
  now-ms:     func() -> u64;
  current-span: func() -> span;
  config:     func() -> result<json, string>;         // public config; x-secret stripped
  secret:     func(name: string) -> result<string, string>;
  call-tool:  func(name: string, args: json) -> result<json, err>;   // ← inter-plugin, §10.3
  emit:       func(topic: string, payload: json);                    // ← inter-plugin, §10.3
  subscribe:  func(pattern: string) -> result<u32, err>;             // ← inter-plugin, §10.3
  poll-event: func(sub: u32) -> option<tuple<string, json>>;
}

// --- role interfaces: each mirrors ONE Rust kernel trait, 1:1 ---
interface tool     { list: func() -> list<tool-spec>;
                     call: func(name: string, args: json) -> result<json, err>; }
interface channel  { caps: func() -> channel-caps;  send: func(msg: outbound) -> result<msg-ref, err>;
                     poll: func() -> option<inbound>;  /* ~10 caps-gated optional fns */ }
interface memory   { put: func(doc: json) -> result<string, err>;
                     search: func(q: json) -> result<json, err>;
                     forget: func(sel: json) -> result<u32, err>; }
interface node     { ports: func() -> list<string>;
                     required-caps: func() -> u32;  effects: func() -> effects;
                     run: func(input: json) -> result<tuple<string, json>, err>; }   // (port, value)
interface protocol-handler { on-open: func(p: peer) -> result<_, err>;
                     on-message: func(p: peer, data: list<u8>) -> result<list<u8>, err>;
                     on-close: func(p: peer); }
interface transform{ apply: func(input: json) -> result<json, err>; }   // pure, no host imports
interface observer { on-event: func(e: json); }

world pii-plugin {                    // ONE world. Roles are opt-in exports.
  import host; import net;            // net only linked if `net` is granted (§4.4)
  export plugin;                      // mandatory
  export tool; export channel; export memory; export node;
  export protocol-handler; export transform; export observer;
}
```

**One world with optional exports, not seven worlds.** zeroclaw has `channel-plugin`,
`tool-plugin`, `memory-plugin` as separate worlds, which means a component cannot be both a
tool and a channel, and the host needs three load paths. Pii uses `declared-roles()` — the
same trick as zeroclaw's `get-channel-capabilities()` flags, applied one level up. The host
loads once, reads the role flags, and registers the component into the corresponding
registries. **A single component can be a tool *and* a graph node *and* a protocol handler.**

**Capability flags shared between WIT and Rust.** `effects`, `roles`, `channel-caps` are one
definition, generated into both. `WasmTool: Tool` is then a pure adapter with no capability
translation layer — pass 1 §5.2's goal, now with the mechanism.

### 10.3 Inter-plugin communication — the genuinely new part

zeroclaw has **none**: no pub/sub, no shared mailbox, no plugin-addressable bus. Its only
shared constructs are host-owned (a per-channel `InboundQueue`; a process-wide
`ConnectionRegistry` for egress budgets). Pii needs this and must design it fresh.

Three mechanisms, in **preference order**:

**1. Graph composition (preferred).** Two plugins are two nodes in a subgraph. They do not
call each other at all — the engine routes data between them. This is observable, resumable,
budgeted, and journaled for free. *Most "plugin A needs plugin B" requests are really this.*

**2. The bus — `host.emit` / `host.subscribe` / `host.poll-event`.** Host-mediated pub/sub
with topic ACLs in the grant (`emit: ["sensor.*"]`, `subscribe: ["cmd.lights.*"]`).
Fire-and-forget, no reply, no ordering guarantee beyond per-topic FIFO. Covers protocol
adapters, sensor normalisers, and anything reactive. Polling (rather than a guest callback)
because it works identically under p2 and p3.

**3. `host.call-tool(name, args)`.** A plugin invokes any registered tool — native or WASM —
by name. It does not know, and must not know, which. Guards:
- budget-checked against the calling `Ctx`;
- **depth-limited** (`max_plugin_depth = 4`) and **cycle-detected** via the `TraceCtx` span
  chain (§2.4), which is a second, free use of the trace infrastructure;
- counted against `MAX_HOST_CALLS_PER_FRAME = 1_000` (zeroclaw's number, no reason to differ);
- the callee's caps are the **intersection** of caller and callee grants — attenuation only,
  same invariant as `Ctx::child()`.

**Explicitly rejected:** shared linear memory between components (unsandboxable), and direct
component-to-component imports (the host loses all accounting, and version skew becomes a
link error at instantiation time in production).

### 10.4 Registry, manifest, and grants

Borrow zeroclaw's `PluginManifest` shape wholesale — it is well designed:

```toml
name = "zigbee-normalise"; version = "0.3.1"
roles = ["node", "transform"]
permissions = ["config-read"]                    # requested, not granted
[config_schema]                                  # JSON Schema Draft 2020-12
# properties marked  x-secret = true  are stripped from host.config() and served
# only through host.secret() — schema-level separation, not a second config surface
signature = "..."; publisher_key = "..."         # Ed25519 over the canonical manifest
```

```
plugin        : PluginId         -> Postcard<PluginRecord>   # name, ver, wasm_hash, manifest, roles
plugin_grant  : (PluginId, Cap)  -> Postcard<Decision>       # allow, scope, expires_at, version_range
```

**"Grant ≠ reach" is the load-bearing rule** (§4.3). The manifest declares *intent*; a
separate operator decision decides what is *admitted*; and enforcement happens at **link
time** — an ungranted capability means the interface is never added to the `Linker`, so the
component fails to instantiate with a clear message rather than failing deep inside a call.
`version_range` on the grant (from `pi_agent_rust/src/permissions.rs:29`) means a grant does
not silently carry across an upgrade that widens the manifest.

### 10.5 Engine configuration — correcting both pass 1 and zeroclaw

zeroclaw runs a single process-wide `OnceLock<Engine>` with **only** `consume_fuel(true)`:
no epoch interruption anywhere in the repo, and **no pooling allocator** (default on-demand
allocation). It compensates with `store.fuel_async_yield_interval(Some(100_000))` so that an
outer tokio deadline can actually preempt a compute-bound guest — without that, fuel is only
observed at exhaustion. Pass 1 called for epoch-primary + pooling and did not mention the
yield-interval problem.

**Final configuration:**

```rust
let mut cfg = Config::new();
cfg.wasm_component_model(true);
cfg.async_support(true);
cfg.epoch_interruption(true);                       // PRIMARY: background ticker, cheap
cfg.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));  // ON — pass 1 is right
cfg.consume_fuel(false);                            // opt-in per plugin, metering only

// per store:
store.set_epoch_deadline(limits.epoch_ticks);
store.epoch_deadline_async_yield_and_update(limits.epoch_ticks);  // ← the piece pass 1 missed
store.limiter(|_| StoreLimits{ memory: max_mem, tables: max_tables, instances: max_inst });
tokio::time::timeout(limits.call_timeout, call).await?;           // wall clock, always
```

- **Epoch primary**: a periodic check is materially cheaper than per-instruction fuel
  accounting for compute-bound guests.
- **`epoch_deadline_async_yield_and_update` is mandatory**, and it plays exactly the role
  zeroclaw's `fuel_async_yield_interval` plays: without it the outer `tokio::time::timeout`
  cannot preempt a tight guest loop. This is the single most likely bug in a
  first implementation.
- **Pooling on.** Component instantiation dominates a plugin call; pooling turns it into a
  µs-scale operation. zeroclaw not using it is a known gap, not a design choice.
- **Warm store for stateful roles** (`channel`, `protocol-handler`, `observer` — zeroclaw
  keeps a warm `(Store, Bindings)` pair for channels); **fresh instance per call for
  stateless roles** (`tool`, `transform`, `node` — zeroclaw does this for tools, and with
  pooling it is cheap). Discard and rebuild on trap or timeout — zeroclaw's
  `reinstantiate`/`unavailable_instance_error` recovery path is correct.
- **No `Default` on `PluginLimits`** — limits always come from the config registry so no
  code path can construct an unsandboxed store by accident.

One more zeroclaw pattern worth copying: **`listen()` is a host-driven poll loop with
exponential backoff**, not a guest push, and a trapping poll marks the plugin unhealthy. It
keeps the host in control of scheduling and it works identically under p2 and p3.

---

## 11. Pii as a swiss-army knife on networking (synthesis)

### 11.1 The claim, and its honest form

Pii can **serve and connect across many protocols from one static binary, with WASM plugins
able to add new protocol handlers without a rebuild.** The honest caveat, stated up front:

> **Every protocol you enable is attack surface and resident memory. Default: everything
> off.** Listeners are declared in config, each with an explicit `NetGrant`. `pii net ls`
> shows exactly what is bound, by whom, under what grant, and how many bytes have moved.
> The RSS gate (§1.6 `rss-gate`) is measured with the *configured* listener set, not an empty one.

### 11.2 The protocol surface

| Role | Crate | Notes |
|---|---|---|
| HTTP server + REST + dashboard | **axum 0.8** | Also the WS upgrade point for remote clients (§3) |
| WebSocket hub | axum's `ws` | Subprotocol `pii.v1`; same messages as UDS |
| **MQTT broker (embedded)** | **rumqttd** | Explicitly "lightweight and embeddable... use it as a library in your code". This is what replaces Mosquitto and, with it, a chunk of Home Assistant. |
| MQTT client | rumqttc | For an existing broker |
| TCP / UDP / UDS | tokio | Raw and `Lines`-framed |
| TLS | rustls + webpki-roots | Never OpenSSL (musl) |
| DNS | **hickory-resolver** | Explicit resolution is required by the resolve-once-pin rule (§4.3); do not let `reqwest` hide it |
| Serial | **serial2-tokio** | `tokio-serial` is unmaintained |
| BLE (central) | btleplug | Feature `ble`; nRF52840 lab boards, sensors |
| MCU RPC | postcard-rpc 0.12 | §9 |
| Outbound HTTP | reqwest 0.12 + rustls, behind `NetService::request` | One shared client (clippy-enforced) |
| MCP client / server | rmcp 2.x, **own crate** | Quarantines its `reqwest` requirement (pass 1 §1.4) |
| ACP | agent-client-protocol 0.10.x, own crate | §3.2 |
| **New protocols** | **WASM `protocol-handler`** | §4.4 — host owns the socket, plugin sees framed bytes |

### 11.3 What this replaces

- **Mosquitto + Home Assistant's automation engine** → embedded `rumqttd` + reactive graphs
  (`starter/sense-decide-act`, zero LLM nodes on the hot path, pass 1 Phase 4 exit criterion 2).
- **A separate webhook receiver** → an `Http` listener publishing `net.*.message` events.
- **A separate cron daemon** → `wait{Timer}` entry nodes (§8.7).
- **socat / a bespoke serial bridge** → a `Serial` listener with a `Lines` or `PostcardRpc`
  protocol.
- **A reverse proxy for one service** → a `Raw` TCP listener with a `net` node dialling out.

### 11.4 The unifying shape

```
                          ┌──────────────── NetService (pii-net) ────────────────┐
   inbound                │  listener registry  │  conn registry  │  resolver    │
   ─────────────────────► │  grants · leases · byte budgets · TLS · tracing      │
   TCP UDP UDS Serial BLE └───────┬──────────────────────────┬───────────────────┘
                                  │ frames                   │ dial (authorize →
                    ┌─────────────▼──────────────┐           │  resolve once →
                    │ Protocol handler           │           │  pin addrs → dial)
                    │  Http│Ws│Mqtt│Lines│       │           ▼
                    │  PostcardRpc│Raw│Plugin(id)│      outbound: LLM APIs, MCP,
                    └─────────────┬──────────────┘      webhooks, sync peers
                                  │ publish
                                  ▼
                        event bus  net.<listener>.*
                                  │
                    ┌─────────────▼──────────────┐
                    │ graph engine               │  wait{Event} → route → tool/emit
                    │ (reactive rules, agents)   │  `net` node kind for outbound
                    └────────────────────────────┘
```

**One `NetService`. One egress policy. One grant model. One trace.** Native tools, graph
nodes, WASM plugins, and MCU links all go through it. That — not the length of the protocol
list — is what makes the swiss-army-knife claim defensible rather than a pile of features.

---

## 12. Updated workspace layout

### 12.1 The full target — 18 crates

Pass 1 had 14. The six new directions add five crates and split the TUI out; two pass-1
crates merge. **Every crate has a one-line reason; a crate without one gets merged.**

```
pii/
├── Cargo.toml            # workspace: [workspace.lints], [profile.release], [profile.perf]
├── clippy.toml           # disallowed-{methods,macros,types} — architecture by lint (§1.3)
├── rustfmt.toml
├── rust-toolchain.toml   # pinned; edition 2024
├── deny.toml             # cargo-deny: advisories, bans (one redb, one reqwest), licenses
├── wit/                  # pii:plugin@0.1.0 — the plugin ABI (§10.2)
├── xtask/                # ci, deps-check, lint-files, size-gate, bench-gate, rss-gate,
│                         # wit-check, proto-gen, pidag-import
└── crates/
    ├── pii-abi/       # ABI STABILITY. Traits + types + ids + errors. serde, thiserror,
    │                  #   async-trait, bitflags, futures-core, smol_str. NOTHING ELSE.
    ├── pii-proto/     # WIRE CONTRACT (§3.3). Control-protocol types + generated schema.
    │                  #   serde + schemars. Feature `client` adds the tokio transport.
    ├── pii-core/      # Ctx, CapSet, Budget, CancelToken, TraceCtx, paths, config.
    ├── pii-obs/       # record!, LogEvent, writer thread, broadcast, Observer impls.
    │                  #   features: metrics (default), otel, console, pprof.
    ├── pii-store/     # THE ONLY CRATE THAT KNOWS redb (pii.redb). Tables, codecs,
    │                  #   migrations, SiteId + HLC stamping.
    ├── pii-graph/     # Spec, validator, scheduler, journal, snapshot, resume, node kinds.
    ├── pii-model/     # ModelProvider impls + SSE. QUARANTINES reqwest/rustls.
    ├── pii-tools/     # Native tools: fs, edit, bash, glob, grep, http.
    ├── pii-memory/    # Wraps agent-memory-core (memory.redb) + Embedder + L0–L3 policy.
    ├── pii-net/       # NetService, egress, grants, protocol handlers.
    │                  #   QUARANTINES axum / rumqttd / hickory-resolver.
    ├── pii-worker/    # WireTransport, supervisor, UDS JSON-RPC half, MCU transports (§9.4).
    ├── pii-hw/        # PLATFORM-ONLY. rppal/nusb/serial2/btleplug. Linux, opt-in,
    │                  #   worker-side. The only crate besides pii-memory allowing unsafe.
    ├── pii-wasm/      # wasmtime host. QUARANTINES the wasmtime tree. Feature `wasm`.
    ├── pii-channels/  # cli, mqtt, telegram, webhook, email.
    ├── pii-sync/      # Sync log, cursors, peers, redaction-on-export (§7).
    ├── pii-server/    # THE DAEMON. Assembles everything; serves pii-proto; run registry;
    │                  #   event bus; cron wheel; supervisor.
    ├── pii-tui/       # ratatui CLIENT. Depends on pii-proto[client] + ratatui ONLY.
    ├── pii-acp/       # QUARANTINES agent-client-protocol. ACP ↔ pii-proto bridge. Client.
    ├── pii-mcp/       # QUARANTINES rmcp (its reqwest 0.13 requirement). Client + server.
    └── pii-cli/       # The binary. argv → mode dispatch. Thin.
```

Merged away from pass 1: `pii-client` folded into `pii-proto` behind a feature (keeping
`pii-proto` tokio-free by default); `pii-mcu` folded into `pii-worker` (an MCU *is* a worker
with a different transport — §9.4).

### 12.2 The dependency DAG — enforced by `xtask deps-check`

```
pii-cli ────────────────────► everything

pii-tui, pii-acp ──────────► pii-proto[client]                    ◄── AND NOTHING ELSE
pii-server ────────────────► pii-graph, pii-net, pii-memory, pii-channels,
                             pii-worker, pii-sync, pii-wasm?, pii-mcp?, pii-proto
pii-channels, pii-worker,
pii-sync, pii-wasm, pii-mcp ► pii-graph, pii-abi, pii-core, pii-obs
pii-graph ─────────────────► pii-abi, pii-core, pii-obs, pii-store
pii-memory, pii-model,
pii-tools, pii-net ────────► pii-abi, pii-core, pii-obs [, pii-store]
pii-store ─────────────────► pii-abi, pii-core
pii-obs   ─────────────────► pii-abi
pii-core  ─────────────────► pii-abi
pii-proto ─────────────────► serde, serde_json, schemars, thiserror   (+ tokio iff `client`)
pii-abi   ─────────────────► serde, thiserror, async-trait, futures-core, bitflags, smol_str
```

**Four hard invariants, each a CI assertion that must provably fail when violated:**

1. `pii-abi` **must never** depend on `tokio`, `reqwest`, `redb`, `wasmtime`, or `axum`.
2. `pii-proto` **must never** depend on `pii-core` or anything below it — the wire contract
   cannot know about the implementation, or a web client inherits the daemon's tree.
3. **`pii-tui` must not have `pii-graph` in its dependency tree.** This is the machine-checkable
   proof that the client/server split (§3.6) is real and not aspirational.
4. Exactly one version each of `redb`, `reqwest`, `wasmtime`, `serde` in `Cargo.lock`
   (`cargo deny bans`) — with a documented exception for `pii-mcp`'s reqwest.

### 12.3 Do not start with 18 crates

Pass 1 already flagged crate proliferation as a risk (`grok-build`: 154 members, a
15,416-line lockfile, no in-tree evidence of payoff). 18 is a *target*, not a starting point.

**Start with seven**, and split only when a stated reason arrives:

```
pii-abi   pii-proto   pii-core   pii-obs   pii-store   pii-graph   pii-cli
```

`pii-obs` is in the initial seven deliberately: observability retrofitted is observability
that never happens, and `record!` call sites are cheapest to write when the code is written.
Everything else splits out of `pii-cli` as it grows, each split justified by one of:
ABI stability, dependency quarantine, platform gating, or a genuine reuse boundary.

---

## 13. Critical review — where this over-engineers

I am the one proposing this, so this section is where I argue against it.

| Over-engineering risk | Severity | Cut / mitigation |
|---|---|---|
| **Observability before there is anything to observe.** 6,616 lines in zeroclaw's equivalent. A `record!` macro, a closed taxonomy, an attribution merger, a writer thread, a rotation policy, a reader with cursors, and a migration path — before the first graph runs. | **High** | Ship `pii-obs` in **two stages**. Stage 1 (Phase 1): `record!`, the typed `Event`, the closed taxonomy, one JSONL writer on a thread, `tracing-subscriber` fmt for dev. ~600 lines. Stage 2 (Phase 3): rotation, reader/cursors, broadcast, Prometheus, migration. **The taxonomy and the macro must be first** — everything else is additive; those two are not. |
| **The control protocol before there is a second client.** Building `pii-proto` + schema generation + negotiation when the TUI is the only consumer. | **Medium** | Real, but the alternative is worse: pass 1 §6.1 already forces a UDS control channel in Phase 2 because redb is single-process. You are building it anyway. The *marginal* cost of doing it as a typed protocol crate rather than ad-hoc structs is ~2 days, and zeroclaw's stringly-typed WS shows the price of skipping it. **Do it. Skip the postcard encoding and the web client until asked.** |
| **`NetService` as a full abstraction before there are three protocols.** Listener registry, grants, leases, byte budgets, protocol-handler plugins — for what is initially one axum server. | **Medium-High** | Build the **egress** half in Phase 3 (it is needed for LLM calls and it is where the security is) and the **listener registry** with only `Http` + `Mqtt` handlers. `Serial`, `Ble`, `Raw`, and `Plugin(id)` handlers are Phase 4+ and each must be justified by a device that exists. |
| **L0–L3 layering before there are 1,000 memories.** L2 community detection and L3 persona rebuilds are pointless on a corpus of 40 documents. | **Medium** | Phase 3 ships **L0 (journal, free) + L1 (atoms) + retrieval** only. L2/L3 land in Phase 5 gated on corpus size, and the trigger thresholds (5 turns / 50 memories) are config. |
| **CRDTs for sync.** A large dependency and a new storage format for a problem that is 90% append-only. | **High if adopted** | **Do not adopt.** §7.2's sync log is ~300 LOC. Loro only for skill bodies, only if conflict-flagging proves annoying. |
| **MCU integration.** A hardware purchase, a firmware toolchain, a C engine, a 25-second inference path, for a capability the daemon already has. | **Medium** | It is genuinely optional and genuinely last. Phase 6. Its real value is the *escalation dataset* and offline resilience, not throughput. If the ESP32-S3 never gets bought, nothing else in the plan changes — that is the test of correct layering. |
| **WASM plugin system before a third-party plugin exists.** wasmtime is a large tree; `bindgen!` breaks on majors; and p2→p3 migration is pending. | **High** | Unchanged from pass 1: `pii-wasm` is feature-gated, nothing in Phases 1–4 links it, and Phase 5 is the earliest. **The first WASM component Pii runs should be one Pii itself generated** (a distilled skill step) — that is the use case that justifies the sandbox, and it is the one that cannot be served any other way. |
| **18 crates.** | **Medium** | §12.3: start at seven. |
| **Quorum / Critic nodes.** pidag's own docs say the critic path is unvalidated against a live model. | **Low but real** | Ship behind a flag with a measured A/B. Do not put it in a starter graph. |
| **Racing.** Doubles cost for a latency win. | **Low** | Node kind exists; no starter graph uses it by default. |

### 13.1 The MVP cut — the smallest thing that is still Pii

Unchanged in spirit from pass 1, with observability added because it is not optional:

```
pii-abi + pii-core + pii-obs(stage 1) + pii-store + pii-graph + pii-cli
+ one provider (Anthropic) + five tools (read, write, bash, glob, grep)
+ `pii -p` headless with ndjson
```

No TUI, no daemon, no WASM, no workers, no channels, no memory beyond the journal, graphs
from TOML files with no dynamic ref indirection. **~5–6 weeks.** It proves or disproves the
one claim everything else rests on: *that graph + journal + `Ctx` is the right substrate.*
If that is wrong, everything after it is wasted, and it is far cheaper to find out in week 6
than in week 26.

---

## 14. Updated phased roadmap

Pass 1 was 6 phases / 15–19 weeks. The twelve new directions add real work. **Honest revised
total: 24–30 weeks solo.** The phases are reordered so that each one ends with something
usable, and every exit criterion is machine-checkable.

### Phase 0 — Prerequisites (1 week, was ½)

`rust-toolchain.toml` (edition 2024; MSRV floor 1.96 given the references), workspace
skeleton with the **seven** starting crates, `[workspace.lints]`, `clippy.toml`,
`rustfmt.toml`, `deny.toml`, and `xtask` with `ci / deps-check / lint-files / size-gate /
bench-gate / rss-gate / proto-gen`.

**Exit:** `cargo xtask ci` green on an empty workspace; `deps-check` **provably fails** when
`pii-abi` is made to depend on `tokio`; `lint-files` **provably fails** on a 451-effective-line
file and **passes** on the same file with 200 lines of `#[cfg(test)]`.

> Note: `rustc` is **not installed in this container** (verified: `command not found`,
> `~/.cargo/bin` empty). Phase 0 runs on the host toolchain. Do not write CI or agent
> workflows that assume a toolchain inside the container.

### Phase 1 — Core engine + observability stage 1 (4–5 weeks, was 3–4)

`pii-abi`, `pii-core` (`Ctx`, `CapSet`, `Budget`, `TraceCtx`), `pii-obs` stage 1,
`pii-store` (tables + migrations + `SiteId`/HLC from day one), `pii-graph` (spec, validator,
scheduler, journal, snapshot, resume, 12 node kinds incl. `net`/`race`, `verify`/`verify_pre`,
`EdgeKind::Order`), `pii-model` (Anthropic + Ollama), `pii-tools` (5 tools), `pii-cli`.

**Exit:**
1. `pii graph run examples/hello.toml` executes `llm → tool → llm`.
2. `kill -9` mid-run; `pii graph resume <id>` completes **without re-invoking any completed
   node** (assert: zero duplicate `NodeFinished` for the same `(node, seq)`).
3. **The diamond test** (§6.4): `entry → {a,b} → join` — both ran, they *overlapped in time*,
   `join` ran exactly once, after both. Four assertions.
4. `pii graph lint` rejects, with three distinct error variants: an undeclared port; a cycle
   with no `loop_budget`; a node whose `required_caps` exceed the graph's `caps`.
5. A tool denied by `Ctx` returns `PiiError::Denied` and the denial is in the journal.
6. `verify: Delta` fails a node that reports success but changed nothing.
7. A `max_model_calls` breach yields `Terminal::BudgetBreach` and exit code **3**, distinct
   from a node failure's exit 1.
8. `record!` microbench < 250 ns; `pii --version` p95 < 20 ms; both gated in CI.

### Phase 2 — Client/server split + coding harness (4–5 weeks, was 3–4)

`pii-proto` + generated schema, `pii-server` (control protocol over UDS + the in-process
loopback transport, run registry, event bus), `pii-tui` as a pure client, `pii -p` over
loopback, `pii-acp`, edit/multi-edit tools, bash mediation + `nono` sandbox profiles,
context compaction as a graph node.

**Exit:**
1. **`cargo tree -p pii-tui` does not contain `pii-graph`.** Asserted in CI. This is the
   whole point of the phase.
2. Pii makes a real multi-file change in its own repo, driven by `starter/verify-fix`, and
   `cargo test --workspace` passes.
3. `pii -p "..."` runs with **no daemon**, over the loopback transport, emitting parseable
   ndjson and exiting non-zero on failure.
4. Start `pii daemon`; the same `pii -p` now executes daemon-side over UDS with **identical**
   ndjson output. One execution path, proven.
5. `session.prompt` returns a `run_id` in **< 50 ms** regardless of run duration (§8.2).
6. Kill and restart the TUI mid-run; reattaching with `since_seq` replays journaled events
   and resumes live streaming with no duplicates.
7. A non-owner uid connecting to the UDS socket is refused (`SO_PEERCRED`).
8. `pii acp` completes a turn from Zed.
9. Sandbox self-verification: with the `workspace` profile, a write to `/etc` fails at the
   **kernel** level, and Pii refuses to start if it cannot verify enforcement.

### Phase 3 — Daemon, networking, memory (4–5 weeks, was 3–4)

`pii-net` (egress with the resolve-once-pin discipline; `Http` + `Mqtt` listeners; grants),
`pii-channels` (cli, webhook, telegram), `pii-worker` (UDS protocol, supervisor, hot reload),
`pii-memory` (agent-memory-core embedded + `OllamaEmbedder` + L0/L1 + retrieval), `pii-obs`
stage 2 (rotation, reader, broadcast, `/metrics`), gateway (axum: REST + WS + minimal
dashboard), `pii-mcp`.

**Exit:**
1. 72 h continuous uptime, RSS **max** (not mean) < 50 MB, sampled every 60 s, with the
   configured listener set active.
2. Telegram message → graph run → reply, with the full journal in `pii graph trace`.
3. `kill -9` a worker: restarted within 2 s, in-flight RPCs fail `WorkerDied`, the graph
   retries and completes. Hot reload replaces the binary with zero dropped requests.
4. **Egress test**: a host resolving to one public and one private address is refused; a
   DNS answer that changes between authorize and dial does **not** change the dialled
   address (assert against a controlled resolver).
5. Hybrid retrieval beats BM25-alone on a 20-query hand-labelled set (recall@10).
6. **The persistence test**: write 1,000 docs, restart the daemon, assert vector-lane recall
   is unchanged (the `agent-memory-core` HNSW-not-persisted bug class, §5.3).
7. `starter/consolidate` on a 5,000-doc corpus completes within its declared `max_llm_calls`
   or terminates `BudgetBreach` — it never runs unbounded (the ConflictDetector incident, §5.4).
8. `pii logs -f` streams live with **no gateway running** (control protocol only).

### Phase 4 — IoT / reactive (2–3 weeks, unchanged)

Embedded `rumqttd` broker + `rumqttc` client, `pii-hw` worker (GPIO/I²C/SPI/USB via
`rppal`/`nusb`/`serial2`, `SCM_RIGHTS` fd handoff), reactive rule graphs, retained device
state in redb.

**Exit:** (pass 1's four, kept verbatim) sensor → rule → actuator p95 < 100 ms over 1 h;
zero `llm` node executions on the reactive path (asserted from the journal); `kill -9` the
hardware worker mid-automation and resume within 2 s with no lost events; RSS with MQTT +
20 devices + 10 automations still < 50 MB.

### Phase 5 — WASM plugins + self-improvement (4–5 weeks, was 3–4)

`pii-wasm` (p2 host, one `pii-plugin` world, role flags, pooling + epoch + async-yield,
manifest + grants, the bus and `call-tool` inter-plugin paths), skills store + file mirror,
`starter/distill`, read-before-write enforcement, curator lifecycle, graph promotion with
shadow-running, trajectory export with redaction, L2/L3 memory layers, local 9 B routing with
MCQ encoding.

**Exit:**
1. A WASM component declaring `roles = ["tool","node"]` is registered into **both**
   registries from **one** load, and is callable as both.
2. A plugin with `net` denied **fails to instantiate** (link-time), not at call time.
3. Plugin A `emit`s a topic; plugin B `subscribe`d to it receives it; a `call-tool` cycle of
   depth 5 is refused with a clear error.
4. A guest tight loop is preempted by the wall-clock timeout (proves
   `epoch_deadline_async_yield_and_update` is wired) and the store is rebuilt.
5. ≥10 skills distilled from real usage measurably reduce median tokens per task by ≥25% on
   a held-out repeat-task set.
6. ≥1 prose skill promoted to a `GraphSpec` executing with **zero LLM tokens**, after ≥95%
   shadow-run agreement over 5 invocations.
7. ≥1,000 trajectories exported; an independent scanner (`gitleaks`) over the JSONL finds
   **zero** secrets.

### Phase 6 — Multi-site sync + MCU (3–4 weeks, new)

`pii-sync` (sync log, cursors, peers, redaction-on-export, `pii sync` CLI), `pii-worker` MCU
transports (`postcard-rpc` over serial/TCP/BLE), tool-table downcast + flashing, confidence
routing, `pii mcu tune`.

**Exit:**
1. Two Pii instances on separate hosts; trajectories written on A appear on B after
   `pii sync pull`, byte-identical, with redaction applied **on A**.
2. A peer with an incompatible `pii-proto` version is **refused**, and a peer with a
   different build sha produces a loud warning in `pii sync status`.
3. Concurrent edits to the same skill on both sites produce two versions and a `needs_merge`
   flag — **no data is lost and nothing is silently overwritten**.
4. Config and secrets are provably absent from the sync log (assert on table membership).
5. An ESP32-S3 running Needle 2 dispatches ≥100 journaled tool calls; `pii mcu tune` raises
   the direct-execution rate at constant verified-error rate.
6. With the daemon stopped, the MCU actuates its local safe set and replays buffered calls
   on reconnect.

### 14.1 Recommended first milestone

**Phases 0–2, ~9–11 weeks**, ending at a coding agent that replaces the current one *and*
has an honest client/server boundary. That boundary is the thing that is expensive to
retrofit, and it is what makes the web client, the assistant daemon, and multi-site all
possible later without a rewrite.

The alternative — the §13.1 MVP cut at ~5–6 weeks — is the right choice if there is any
doubt about the graph substrate itself.

---

## 15. New open decisions

Pass 1 left three (redb vs SQLite → **redb**, still right and now reinforced by
`agent-memory-core` and `pidag` both being redb-based; first milestone; drop
`pii worker --graph` as a default → **yes, still**). Pass 2 adds eight.

| # | Decision | Options | My recommendation |
|---|---|---|---|
| **1** | **Memory backend** | (a) embed `agent-memory-core` as a workspace dep; (b) talk to the existing server at `:7420` over REST; (c) reimplement per pass 1 §6.2/6.3 | **(a) embed, with (b) available as `MemoryBackend::Remote`.** (c) costs ~600 LOC of BM25 and re-derives a +11%-NDCG result you already have. Blocker to check first: does `agent-memory-core`'s redb version match the workspace pin? |
| **2** | **450-line gate semantics** | (a) raw lines; (b) **effective** lines (minus blank/comment/`#[cfg(test)]`) | **(b).** Counting tests against the budget punishes testing. |
| **3** | **WASI 0.2 vs 0.3** | (a) p2 now, mechanical port later; (b) p3 now on the experimental host impl | **(a).** The spec is stable; `wasmtime-wasi`'s `p3` is "experimental, unstable and incomplete". Avoid `wasi:io` types in Pii's own WIT so the port stays mechanical. |
| **4** | **Protocol encoding** | (a) JSON only; (b) JSON + negotiated postcard for local UDS | **(a) for Phase 2, (b) if delta streaming shows up in a profile.** Do not pre-optimise the wire before the TUI exists. |
| **5** | **Skill sync conflicts** | (a) versioned + `needs_merge` flag; (b) Loro CRDT | **(a).** A CRDT is a permanent dependency for a rare event. Migration to (b) is mechanical if (a) annoys. |
| **6** | **Buy an ESP32-S3-N16R8?** | (a) yes, ~$10, unlocks §9; (b) no, nRF52840-only | **(a) — but not before Phase 5.** The nRF52840 (1 MB flash / 256 KB RAM) **cannot** run Needle 2; it stays as a `postcard-rpc` sensor/actuator node. Nothing else in the plan depends on this. |
| **7** | **pidag's fate** | (a) retire after Pii Phase 2; (b) keep both indefinitely | **(a), with an importer.** Keeping two DAG engines means two bug surfaces and two mental models. Steal `verify`/`after`/budget-terminal/quorum first. |
| **8** | **Embeddings** | (a) `OllamaEmbedder` (musl-clean, needs Ollama); (b) `fastembed` (C dep, glibc); (c) `candle` (pure Rust, +40 MB model) | **(a) default, (b) behind a feature for glibc, (c) if offline-without-Ollama becomes a real requirement.** The static-musl goal makes (b) unsuitable as the default. |

---

## 16. Risk assessment

### 16.1 Highest risks, ranked

| # | Risk | Likelihood | Impact | Early warning | Mitigation |
|---|---|---|---|---|---|
| **1** | **Scope.** Pass 2 grew the plan from ~17 to ~27 weeks. A solo project that never ships Phase 1 has produced nothing. | **High** | **Fatal** | Phase 1 slipping past week 7 | §13.1's MVP cut is the pressure valve. Phases 3–6 are each independently droppable; **Phases 0–2 are not.** |
| **2** | **`wasmtime` churn.** 47 → 48 in the time between pass 1 and pass 2. `bindgen!` output changes on majors; p2→p3 is coming. | High | Medium | A `cargo update` breaking `pii-wasm` | Exact-pin. Feature-gate. Nothing in Phases 1–4 links it. `xtask wit-check` catches WIT drift. |
| **3** | **Observability overhead on the reactive path.** Phase 4 targets p95 < 100 ms; a naive `record!` on every node could eat a meaningful slice. | Medium | Medium | The `record!` microbench regressing | Gate it (< 250 ns / < 40 ns filtered). Bounded queue, drop-on-full with a counter. Level-filter *before* building the event. |
| **4** | **`agent-memory-core` coupling.** It is external to this repo (lives on another machine's checkout), and Pii would take a hard dependency on a crate with its own redb version and its own release cadence. | **Medium-High** | High | A redb version conflict at first `cargo build` | Vendor it as a **workspace path dependency or a pinned git rev**, never a floating one. `cargo deny bans` enforces a single redb. The `Memory` trait keeps a hand-rolled fallback possible. **Verify the redb version before committing to decision 1.** |
| **5** | **The client/server boundary leaking.** The temptation to let `pii-tui` "just import one type" from `pii-graph` is enormous and it destroys the whole design. | **High** | High | Any PR adding a dep to `pii-tui` | The CI assertion (`cargo tree -p pii-tui` must not contain `pii-graph`) is non-negotiable and must exist from the first commit of Phase 2. |
| **6** | **redb single-writer.** All writes serialise; a write transaction held across `.await` deadlocks the daemon. | Medium | **High** | Latency spikes correlated with journal writes | `clippy::await_holding_lock` is deny; `pii-store` is the only writer; batch journal appends; **never** hold a txn across an await. Add a debug-build assertion that a write txn's lifetime contains no yield point. |
| **7** | **Egress policy bugs.** The resolve-once-pin logic is subtle and a mistake is an SSRF. | Medium | **High** | — | Port zeroclaw's tests too, not just its code: the deny-by-default test, the mixed-answer test, the metadata-endpoint test. Treat this as security code, review it as such. |
| **8** | **Sync data loss.** A naive merge silently overwrites a skill edited on the other machine. | Medium | High | — | Append-only everywhere; `needs_merge` rather than auto-resolve; **no destructive operation in the sync path at all** — the maximum action is "create a second version". |
| **9** | **musl + hardware crates.** `rppal`/`probe-rs`/`nusb`/`btleplug` may not build against musl. | Medium | Low | Phase 4 build failure | Already mitigated by pass 1's design: `pii-hw` is a **separate worker process**, so it can ship as a separate glibc build. A genuine second-order benefit of the worker split. |
| **10** | **MCU integration cost.** A C engine, ESP-IDF, a 3-minute flash cycle, and a board that must be bought. | Medium | Low | — | Phase 6, fully optional, zero coupling. If it never happens the plan is unaffected — which is the test that §9 is correctly layered. |
| **11** | **Cheap models writing fake tests.** Directly observed in orki's history: a fabricated `ExecutionResult` hardcoding `success: true` that hid a real bug. | **High** (this project delegates implementation) | High | Green tests, broken behaviour | Every phase exit criterion in §14 is **behavioural** — kill a process, assert timing overlap, assert an exit code, assert a dependency-tree absence. No criterion is "tests pass". Ask of every test: *would this fail if the feature were removed?* |
| **12** | **The trajectory corpus leaking secrets.** It is exactly the artifact that ends up in a fine-tune. | Medium | **Severe** | — | Redaction is mandatory, pre-write, policy-versioned, and runs **on the sender** before sync. `gitleaks` over the corpus is a Phase 5 exit criterion. Retrofitting means discarding the corpus. |

### 16.2 Risks pass 1 flagged that pass 2 downgrades

- **"redb single-writer across sites"** — not a risk. Sites never share a file (§7.5). The
  real cost is `SiteId` + HLC on every record, which must go in at Phase 1.
- **"No FTS in redb, budget 600 LOC of BM25"** — not a cost, given `agent-memory-core` +
  tantivy (§5.1).
- **"Vector index tuning/corruption problems"** — real, but now a *known* failure mode with
  a specific test (§5.3, Phase 3 exit 6) rather than an unknown.

### 16.3 Risks pass 2 adds

- **Two databases in one process** (`pii.redb` + `memory.redb`). Legal, but transactions
  cannot span them: a memory write and a journal write are **not** atomic together. Design
  around it — memory writes are idempotent and derived from the journal, so a crash between
  them re-derives rather than corrupts. **State this invariant in `pii-memory`'s docs.**
- **The protocol becomes a compatibility surface** the moment a second client exists.
  `PROTOCOL_VERSION` bumps and the checked-in schema are the discipline; `sys.hello` refuses
  skew rather than guessing.
- **W3C trace propagation across MQTT/serial/postcard** is more plumbing than it looks. If
  Phase 1 runs long, ship `TraceCtx` in-process only and add the carriers in Phase 3 — the
  struct and the `Ctx` field must exist from day one, but the carriers can lag.

---

## 17. Verification of this plan's own claims

No code exists, so verification means the claims themselves.

**Verified this pass, re-checkable:**

| Claim | How to re-check |
|---|---|
| wasmtime 48.0.1; `wasmtime-wasi` has `p2` and `p3`; `p3` is "Experimental, unstable and incomplete" | `docs.rs/wasmtime-wasi/latest` |
| WASI 0.3.0 released 2026-06-11; `wasi:io` removed; wasmtime 46+ implements final 0.3 | `wasi.dev/releases/wasi-p3` |
| Needle 2: 45 M params, 14 MB, ESP32-S3 **N16R8** only, 13.7 MB @ `0x210000`, 534 ms/token, 256-token context, JSON-Schema tools with no nesting, grammar-constrained output, confidence = groundedness | `github.com/andrisgauracs/needle-2-esp32` README |
| `postcard-rpc` 0.12.1, 2026-06-12, `no_std`, Endpoints + Topics, `host_client` | `docs.rs/postcard-rpc` |
| `tokio-serial` is unmaintained and seeking adoption | crates.io / its README |
| `rumqttd` is "lightweight and embeddable... use it as a library" | rumqtt docs |
| zeroclaw: `zeroclaw-log` 6,616 lines; `Observer` at `zeroclaw-api/src/observability_traits.rs:348-380`; gateway is axum 0.8 **TCP-only, no `UnixListener`**; WS frames are untyped `json!({"type":...})`; no shared protocol crate | `sed -n` against `/projects/pi-improved/_repos/zeroclaw/` |
| zeroclaw egress: deny-by-default at `wasi_http.rs:198-208`; resolve-once-pin at `wasi_http.rs:255-336` / `egress.rs:532-631`; `ConnectionRegistry` at `egress.rs:290-337`; wasmtime **47.0.4**, `consume_fuel` only, **no epoch, no pooling** (`component.rs:464-471`); `MAX_HOST_CALLS_PER_FRAME = 1_000` (`component.rs:25`); channels get **no** wasi:http (`wasm_channel.rs:108-133`) | same |
| zeroclaw WIT worlds are per-role (`channel-plugin`, `tool-plugin`, `memory-plugin`) — a component cannot hold two roles | `/projects/pi-improved/_repos/zeroclaw/wit/v0/*.wit` |
| hermes: `step_callback(api_call_count, prev_tools)` at `conversation_loop.py:2246-2272`; `thinking_callback` at `:3141`; cron is a 60 s-poll daemon thread with a file lock and **no live progress** (`cron/scheduler_provider.py:552-657`, `cron/scheduler.py`) | `/projects/pi-improved/_repos/hermes-agent/` |
| pidag: single crate, redb, **8 tables** at `src/store/redb_store.rs:11-34`; budget breach = **exit 3**; `verify` is delta-based; A2A server deliberately deleted | `/projects/pidag/` |
| `agent-memory-core` layout, 3-lane RRF +11% NDCG, the HNSW-not-persisted bug, the ConflictDetector 3800→18 incident with its exact constants | agent-memory backend, topic `agent-memory` |
| orki diamond bug (single-task fanout + any-predecessor convergence); blackboard 2310 vs 50279 ops/s | agent-memory backend, topic `orki-orchestration` |
| **`rustc` is NOT installed in this container** | `rustc --version` → `command not found`; `~/.cargo/bin` empty |

**Not verified — flagged as assumptions:**

- `agent-memory-core`'s exact redb version and its public API surface. **The source is not on
  this machine** (it lives on a Mac checkout, deployed to the lnx podman host serving
  `10.10.10.217:7420`). Everything in §5.1 comes from that system's own stored research
  notes, not from reading the code. **Decision 1 must not be finalised until someone reads
  the crate.**
- `wasmtime-wasi` p2's exact socket-policy API (`socket_addr_check`, `allow_tcp`/`allow_udp`)
  — asserted from memory, not re-read this pass.
- Needle 2's "n-gram hashed lookup tables + Hadamard transform + 2-bit-from-scratch"
  architecture claims come from the video transcript in the brief; the README confirms the
  measurements and the Cactus quantisation ("compressed to CQ2-bit"), not the internal
  mechanism.
- All Rust dependency versions should be re-resolved at Phase 0; several moved between
  pass 1 and pass 2 (wasmtime 47→48 in ~1 day of elapsed project time).

**The first executable proof of the pass-2 architecture** is Phase 2 exit criterion 1:
`cargo tree -p pii-tui` not containing `pii-graph`. If the client/server split is wrong or
impractical, that assertion is the thing that fails, and it fails in week 10 rather than
week 30.

**Sources (web research, this pass):**
[WASI 0.3](https://wasi.dev/releases/wasi-p3) ·
[wasmtime](https://docs.rs/wasmtime/latest/wasmtime/) ·
[wasmtime-wasi](https://docs.rs/wasmtime-wasi/latest/wasmtime_wasi/) ·
[needle-2-esp32](https://github.com/andrisgauracs/needle-2-esp32) ·
[Needle 2 / Cactus](https://cactuscompute.com/needle) ·
[Needle 2 on HF](https://huggingface.co/Cactus-Compute/needle2) ·
[postcard-rpc](https://docs.rs/postcard-rpc/latest/postcard_rpc/) ·
[Loro](https://github.com/loro-dev/loro) ·
[redb](https://github.com/cberner/redb) ·
[rumqttd](https://crates.io/crates/rumqttd) ·
[btleplug](https://github.com/deviceplug/btleplug) ·
[tokio-serial](https://crates.io/crates/tokio-serial/5.4.1) ·
[ACP](https://agentclientprotocol.com/get-started/introduction) ·
[ESP32-S3 28.9M LLM](https://www.cnx-software.com/2026/08/03/28-9m-parameter-llm-runs-locally-on-esp32-s3-at-9-tokens-s/) ·
[TencentDB-Agent-Memory](https://github.com/TencentCloud/TencentDB-Agent-Memory) ·
[tracing-opentelemetry](https://github.com/tokio-rs/tracing-opentelemetry) ·
[usearch](https://docs.rs/usearch/latest/usearch/) · [hnsw_rs](https://lib.rs/crates/hnsw_rs)

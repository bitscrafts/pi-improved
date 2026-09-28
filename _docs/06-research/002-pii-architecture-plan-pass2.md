<!-- MERGED-CORPUS 2026-09-28T20:00:57Z: this file is the canonical document for its subject.
     It absorbed 2 document(s) which are now sections below, kept verbatim.
     Why: pass 2 carries a table of corrections to pass 1; pass 3 explicitly reframes pass 2's tier model (§9.5).
     Originals archived at _docs/99-archive/06-research-merged-2026-09-28/ (nothing deleted). -->

# Pii Architecture Plan (consolidated: passes 1-3)

> Pass 1 lives at `/projects/pi-improved/_docs/06-research/002-pii-architecture-plan-pass2.md`.
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


---

## [merged] Pii (Pi Improved) — Architecture Plan

<!-- Absorbed from `002-pii-architecture-plan-pass2.md` on 2026-09-28T20:00:57Z during the SSOT corpus merge.
     Content preserved verbatim. Original moved to 99-archive/06-research-merged-2026-09-28/. -->

## Context

**Why this exists.** Four capabilities currently live in four separate stacks: a terminal
coding agent (Claude Code / `pi_agent_rust`), a personal assistant + messaging gateway
(ZeroClaw-class), a home automation engine (Home Assistant + MQTT), and nothing at all for
the self-improvement loop (Hermes is Python and not deployable here). Running all four means
four runtimes, four memory stores, four config surfaces, and no shared learning: a procedure
discovered while coding cannot be reused by the assistant, and neither produces data that
could train a local model.

**Intended outcome.** One static Rust binary, `pii`, that is all four — because all four are
the same shape underneath: *an event arrives, a graph of nodes executes against a capability-
scoped context, state is checkpointed, a trajectory is logged.* A coding turn is a graph. A
Telegram message is a graph. A sensor threshold crossing is a graph. A distilled skill is a
graph. Unifying on that primitive is the whole bet.

**Target envelope.** <50 MB idle RSS for the daemon, <20 ms cold start for one-shot CLI,
single static binary (musl + rustls, no C deps in the default build).

This document is research + architecture only. No code has been written.

---

## 0. Findings that change the brief

Five things I verified in the repos that contradict assumptions in the original prompt. These
are load-bearing — read them before the rest of the plan.

| # | Assumption in brief | What the code actually shows |
|---|---|---|
| 1 | "`pi_agent_rust` has a capability-gated `Cx` context" | `Cx` is `asupersync::Cx` (external crate v0.4.4, not vendored). The wrapper `AgentCx` (`pi_agent_rust/src/agent_cx.rs:26`) exposes `.fs()`, `.http()`, `.process()` accessors that **perform no checks** — `AgentCx::read` at `src/agent_cx.rs:128` calls `asupersync::fs::read` unconditionally. All 9 call sites in `src/tools.rs` use it only to obtain an async context. The real, enforced gating is a *separate* string-keyed permission store (`src/permissions.rs:29`, `PersistedDecision{capability, allow, expires_at, version_range}`) applied to WASM/JS extensions. **Pii must actually build the enforcing version — it does not exist to copy.** |
| 2 | "`zeroclaw-sop-graph` is the graph engine" | It is **pure serde wire/visualisation types** (`GraphNode`, `GraphPin`, `PinClass`, canvas geometry) for rendering in a TUI/web editor. Zero execution logic, no tokio dep. The real engine is `zeroclaw-runtime/src/sop/` — `engine.rs` alone is **645 KB**, `dispatch.rs` 131 KB, and the agent loop `agent/loop_.rs` is **17,727 lines**; `channels/src/orchestrator/mod.rs` is **1.4 MB**. ZeroClaw's clean layering is real in `zeroclaw-api` and aspirational everywhere below it. **Borrow the ABI crate; do not borrow the runtime.** |
| 3 | "`zeroclaw-spawn` = worker/subagent spawning" | It is a single macro wrapping `tokio::spawn` with lifecycle logging (`crates/zeroclaw-spawn/src/lib.rs:22`). Subagents are **in-process** (`zeroclaw-runtime/src/subagent/mod.rs:46`), distinguished only by a narrowed `SecurityPolicy`. **No repo in the reference set implements Option C (UDS worker processes).** Pii is inventing it; it must be specified from scratch (§4.3). |
| 4 | "YantrikClaw has cognitive memory (YantrikDB)" | `yantrikdb` is an **unvendored crates.io dependency** (`yantrikclaw/Cargo.toml:133`, v0.3, optional). YantrikClaw's own code is a thin HTTP proxy (`src/memory/yantrikdb.rs`, POSTs to a companion at `127.0.0.1:8080`) plus a native shim. The entities/relationships claim actually maps to `src/memory/knowledge_graph.rs` — 824 lines, plain SQLite `nodes`/`edges` + FTS5. The genuinely portable assets are small: the tier enum, the MCQ prompt/parser, the family keyword table, the consolidation prompt, and that SQL schema. |
| 5 | "redb is the persistence layer" | **No repo in the reference set uses redb.** All four Rust repos use `rusqlite 0.37` (bundled). This does not make redb wrong, but it does mean you are on an unpaved road and you will be hand-writing what SQLite's FTS5 gives free. More importantly: **redb is single-process** (file lock), which forces a DB-ownership decision (§6.1) that changes the process model. |

Two further corrections worth noting: YantrikClaw's README advertises tier tool budgets of
10/20/25/30 while `src/tools/tier.rs:120` actually implements 5/8/15/20 — trust the code. And
`grok-build` as published contains no `main.rs`; the binary crate is not in the public tree,
so its TUI event loop and headless mode-branch are unavailable for reference (the supporting
library crates are all present).

---

## 1. Comparative analysis — what to take, what to leave

### 1.1 `zeroclaw-api` — the kernel ABI (take the shape, fix three things)

The strongest single artifact in the reference set. What makes it work:

- **Dependency-starved by design.** `crates/zeroclaw-api/Cargo.toml` has no `reqwest`, no
  `hyper`, no HTTP at all; `tokio` is `default-features = false` with only `sync, process,
  macros, rt`. Traits define shape, never transport.
- **One required method, everything else defaulted.** `ModelProvider`
  (`model_provider.rs:492`) has 17+ methods but only `chat_with_system` lacks a default — a
  minimal provider is one function. `Channel` (`channel.rs:619`) has ~30 methods, 3 required
  (`name`, `send`, `listen`); the CLI channel (`zeroclaw-channels/src/cli.rs`) is 89 lines
  total.
- **Capability flags over trait-method-probing.** In the WIT layer
  (`zeroclaw-plugins/wit/v0/channel.wit:119`), `flags channel-capabilities { health-check,
  self-handle, request-approval, ... }` — the host calls `get-channel-capabilities()` once at
  load and only invokes flagged methods. Excellent pattern.
- **Providers self-describe wire format.** `ToolsPayload::{Gemini, Anthropic, OpenAI,
  PromptGuided}` (`model_provider.rs:427`) — the kernel never hard-codes one tool-call shape.
- **`ToolSpec.parameters: Arc<serde_json::Value>`** (`tool.rs:237`) so repeated `spec()` calls
  don't deep-clone schemas. Small detail, right instinct.
- **Blanket `impl<T: Trait + ?Sized> Trait for Arc<T>`** (`model_provider.rs:794`) so
  `Arc`-wrapping needs no manual re-impl.
- **No `inventory`/`linkme`.** Registration is explicit `Vec<Box<dyn Tool>>` construction.
  Correct: link-time registration magic is hostile to feature-gating and to a single binary
  that must reason about what it actually contains.

Three things to fix in Pii:

1. **`anyhow::Result` in the ABI is wrong.** Every trait method returns `anyhow::Result`, so a
   caller cannot distinguish "user denied", "deadline exceeded", "cancelled", "upstream 429"
   and "bug". Graph routing, retry policy, and budget accounting all need that distinction.
   Pii defines a `thiserror` enum (§2.3).
2. **`ChatMessage { role: String, content: String }`** (`model_provider.rs:22`) is too flat —
   no content blocks, so images/thinking/citations are bolted on sideways
   (`reasoning_content: Option<String>` on the response). Pii uses structured content blocks.
3. **The layering is not enforced.** `zeroclaw-runtime` is a monolith with a 17.7k-line agent
   loop. Pii adopts `grok-build`'s enforcement trick instead: `clippy.toml`
   `disallowed-methods` (see §1.4) plus a file-size CI gate.

### 1.2 `pi_agent_rust` — cold start and tool ergonomics (take the measurements discipline)

- **Cold start is real and measured, not claimed.** `docs/evidence/startup-benchmark-report.json`
  records `--version` p95 = **7.14 ms** against a 100 ms threshold, binary 23.05 MB against a
  48 MB threshold, backed by `docs/contracts/startup-benchmark-contract.json` as a CI gate.
  Adopt this posture wholesale: a perf claim that isn't a gated contract is marketing.
- **`[profile.release]`** (`Cargo.toml:914`): `opt-level = "z"`, `lto = true`,
  `codegen-units = 1`, `panic = "abort"`, `strip = true`, with a **separate `[profile.perf]`**
  (`:931`) at `opt-level = 3`, `lto = "thin"`, `debug = 1` for benchmarking. Shipping profile
  and measuring profile are deliberately different. Copy both.
- **Build-time asset compression** (`build.rs:12-83`): a static table of large text assets
  gzipped at build time into `OUT_DIR` with `RAW_LEN`/`CRC32C` constants emitted alongside, so
  runtime decompression allocates exactly once. `src/embedded_assets.rs:19-114` goes further
  with a **`const fn` LZSS codec** that runs at compile time — with a documented table-size
  cap because "larger tables made every target spend minutes initializing const state."
- **`ToolEffects` fails closed.** `src/tools.rs:163` — undeclared tools default to
  `ToolEffects::write()`, i.e. serialised, not parallel. The scheduler uses it to decide
  concurrency. Pii adopts this directly and extends it into the graph scheduler.
- **Truncation is structured, not silent.** `truncate_head()` returns a `TruncationResult`
  recording `truncated_by: Lines|Bytes`, so the UI can explain *why* output was cut.
- **`Context<'a>` uses `Cow<'a, [Message]>`** (`src/provider.rs:82`) to avoid deep-cloning
  conversation history every turn.
- **Bash mediation is a classifier, not a sandbox** (`src/bash_mediation.rs:251`): tiers
  `critical`/`high`, modes `Off|Warn|BlockCritical|BlockHigh`, delegating to an external `dcg`
  binary when present and falling back to a fixed `DangerousCommandClass` enum. Every verdict
  emits an audit record (`{"schema":"pi.bash.mediation.v1", ...}`). Useful as a *policy* layer
  — but it is not confinement. Pii pairs it with kernel enforcement (§1.4).
- **Progressive-migration-behind-a-feature-flag**: `src/interactive.rs` (143 K, bubbletea) and
  `src/interactive_ftui.rs` (275 K, new stack) coexist under `tui`/`ftui` features, cutover
  module-by-module, old stack deleted at the end. Reusable technique.

**Avoid:** the single-crate-with-763 KB-`tools.rs` structure, hand-written JSON schemas per
tool, and the QuickJS runtime entirely (the brief already rules out embedded interpreters —
`src/extensions_js.rs` is 1.4 MB, the largest module in the crate, which is the cost).

### 1.3 `yantrikclaw` — tiering (take four small things, leave the rest)

Worth porting, all small and self-contained:

- **`ModelTier { Tiny, Small, Medium, Large }`** with parameter-count extraction from model
  names (`src/tools/tier.rs:11-82`) — parses both `qwen3.5:0.6b` (Ollama) and `Llama-3.2-1B`
  (HF) forms, falls back to name-sniffing, defaults unknown local models to `Medium`.
- **`ModelCapabilityProfile`** (`tier.rs:120`) — the tier drives `max_tools_per_prompt` (5/8/
  15/20), `tool_call_mode` (`Mcq` / `StructuredJSON` / `NativeFunctionCall`), `max_agent_steps`
  (3/5/10/15), repair-loop enable and attempts (0/1/2/3), confidence threshold.
- **The MCQ path** (`src/tools/mcq.rs:62-237`) — for models too small for function calling:
  render ≤5 tools as `A. name — desc`, ask for a letter, parse with a 3-strategy fallback.
  Directly relevant to running a 9B locally.
- **The consolidation prompt** (`src/memory/consolidation.rs:23`) — a two-field extraction
  (`history_entry`, `memory_update`) run fire-and-forget after each turn at temperature 0.1.

Leave: `yantrikdb` (external, opaque), the "family routing" (`src/tools/family.rs:175` — pure
substring keyword matching over 8 hand-written keyword lists, marketed as embedding-ranked),
and the companion-over-HTTP model (`src/companion_process.rs` supervises a child process but
talks to it over `127.0.0.1:8080` HTTP — Pii uses UDS, §4.3, though the supervision logic
itself — 500 ms health poll, 30 s startup timeout, 3-consecutive-failure restart, SIGTERM
with 5 s grace — is a good template).

### 1.4 `grok-build` — sandboxing and ACP (take both, plus the enforcement trick)

- **ACP: wrap, don't reimplement.** `xai-acp-lib` depends on the external
  `agent-client-protocol` **0.10.4** crate and adds a typed routing layer on top: an `AcpSide`
  trait pairing in/out message types (`message.rs:16`), `AcpRequest { type Response }` for
  compile-time request↔response pairing (`message.rs:47`), and per-request `oneshot` reply
  channels. Two hard-won transport fixes are worth copying verbatim: a **dedicated OS thread
  for stdin** (tokio's `Stdin` uses an uncancellable blocking-pool thread; on Windows they also
  repoint `STD_INPUT_HANDLE` at `NUL` to prevent `StdinLock` deadlock), and **`LineBufferedRead`**
  because `BufReader::read_line` is not cancel-safe under `select!` (partial reads call
  `consume()` internally and lose bytes).
- **Sandbox: kernel-enforced, fail-closed, self-verifying.** `xai-grok-sandbox` uses the
  external `nono` crate (**exact-pinned `=0.53.0`**, with a comment explaining that a bump can
  silently change macOS Seatbelt deny-rule ordering and re-open a `mv x y && cat y` bypass) for
  Landlock/Seatbelt, applied once at process start. Child *network* confinement is separate
  hand-rolled seccomp-BPF (`child_net.rs`) installed per-child via `pre_exec`, including
  syscall-arch gating and `clone3`(435)-blocking so namespace creation can't escape the filter.
  Crucially it **verifies its own enforcement at startup** (`verify_hook_write_deny_enforced`,
  `verify_read_deny_enforced`) and fails closed if it can't. That is the standard to hit.
- **Architecture enforced by lint, not convention.** `clippy.toml` bans
  `reqwest::Client::new` (must use the shared-TLS builder), `Path::canonicalize` (must use
  `dunce::canonicalize`), `dirs::home_dir`, and — most importantly —
  `std::process::Command::spawn` / `tokio::process::Command::spawn` / `SlavePty::spawn_command`
  **outright**, because "an unenrolled child outlives the session that started it, while an
  enrolled one dies with its scope." Pii adopts `disallowed-methods` from day one.
- **Reason-driven crate splits.** `xai-grok-mcp` exists specifically to quarantine `rmcp 2.1`'s
  `reqwest 0.13` requirement away from the workspace's `reqwest 0.12`. That is a real reason to
  split. **Avoid the default-to-many-crates posture** — 154 workspace members and a 15,416-line
  `Cargo.lock` for one application is a coordination tax with no evidence in-tree that it pays.
  Split for a *reason*: version quarantine, platform-only code, ABI stability, or a genuine
  reuse boundary.

### 1.5 `hermes-agent` — the self-improvement loop (take the algorithm, upgrade the data)

- **A skill is a directory with `SKILL.md`** — YAML frontmatter (`name` matching
  `[a-z0-9][a-z0-9_-]*`, `description` **≤60 chars**, `version`, `platforms`,
  `metadata.hermes.{tags, related_skills}`) plus a body with a mandated section order (`When to
  Use` / `Prerequisites` / `How to Run` / `Quick Reference` / `Procedure` / `Pitfalls` /
  `Verification`), ~100–200 lines, with `references/` `templates/` `scripts/` subdirs. Usage
  stats live in a sidecar `~/.hermes/skills/.usage.json`, not in frontmatter — correct
  separation of content from telemetry.
- **Retrieval is progressive disclosure, not vector search.** `prompt_builder.py:2092` renders
  *every* skill's `name: ≤60-char description` into a `## Skills` block in **every system
  prompt**, with the instruction "Err on the side of loading". Full bodies load on demand via
  `skill_view(name)`. The 60-char cap exists precisely because the index is always resident.
  Two-layer cache (in-process LRU + on-disk snapshot keyed by mtime/size manifest).
- **There is no separate distillation engine.** `learn_prompt.py:22` states it explicitly: "the
  agent does the work with its existing toolset." Distillation is the same agent, forked, with
  tools whitelisted to `memory`/`skill_manage`/`skill_view`/`skills_list`.
- **Read-before-write is enforced by the tool**, not by prompt: `skill_manage` refuses a patch
  unless `skill_view(name)` was called in the same review turn (`background_review.py:542`).
  Copy this exactly — it is the only real defence against an LLM blind-overwriting a skill.
- **Preference ordering in the review prompt** ("prefer the earliest action that fits: 1. update
  a currently-loaded skill, 2. update an existing umbrella, 3. add a support file, 4. create a
  new umbrella") is what stops skill-library sprawl. The curator's `CURATOR_REVIEW_PROMPT`
  (`curator.py:429`) reinforces it: "A collection of hundreds of narrow skills where each one
  captures one session's specific bug is a FAILURE of the library... If you end the pass with
  fewer than 10 archives, you stopped too early."
- **Lifecycle state machine**: `active → stale (30 d unused) → archived (90 d, restorable)`,
  `pinned` as opt-out, and **never delete** — archiving is the maximum destructive action.
  Only `created_by: "agent"` skills are curator-manageable.
- **Trigger**: post-turn background review fires when `_iters_since_skill >= 10`, forking a
  daemon-thread copy of the live agent that replays the conversation verbatim (to keep the
  prompt cache warm) — ~30 K tokens per event.

**The one thing to upgrade.** Trajectories (`agent/trajectory.py:30`) are ShareGPT JSONL with a
single `completed: bool` outcome label and no grading step. That is enough for naive SFT and
nothing else. For a 9B that must learn *when to escalate*, you need per-step cost/latency,
programmatic verification, and a reward signal (§6.4).

---

## 2. Workspace layout and kernel ABI

### 2.1 Crates

Reason-driven splits only. Fourteen crates, each with a one-line justification.

```
pii/
├── Cargo.toml                 # workspace; [profile.release] + [profile.perf]
├── clippy.toml                # disallowed-methods — architecture enforced by lint
├── wit/                       # pii:plugin@0.1.0 WIT worlds (the real plugin ABI)
├── xtask/                     # build, bench-gate, wit-check, size-gate
└── crates/
    ├── pii-abi/       # ABI STABILITY. Traits + data types. No transport deps.
    ├── pii-core/      # Ctx, CapSet, CancelToken, Budget, PiiError, ids, Observer bus.
    ├── pii-store/     # THE ONLY CRATE THAT KNOWS redb. Tables, codecs, migrations.
    ├── pii-graph/     # Graph engine: spec, validator, scheduler, checkpoint, resume.
    ├── pii-model/     # ModelProvider impls + SSE. QUARANTINES reqwest/rustls.
    ├── pii-tools/     # Native tools: fs, edit, bash, glob, grep, http, mcp client.
    ├── pii-memory/    # BM25 + vector over pii-store. Skills, trajectories, distiller.
    ├── pii-wasm/      # wasmtime host. QUARANTINES the wasmtime dep tree. Optional.
    ├── pii-worker/    # UDS JSON-RPC 2.0 protocol: client + server halves + supervisor.
    ├── pii-hw/        # PLATFORM-ONLY. rppal/nusb/tokio-serial. Linux, opt-in. Worker-side.
    ├── pii-channels/  # Channel impls: cli, mqtt, telegram, webhook, email.
    ├── pii-tui/       # ratatui + crossterm interactive harness. Feature `tui`.
    ├── pii-acp/       # QUARANTINES agent-client-protocol. stdio transport fixes.
    └── pii-cli/       # The binary. argv → mode dispatch. Thin.
```

Dependency rule, enforced in CI by a `cargo tree` check in `xtask`:

```
pii-cli ──> everything
pii-tui, pii-acp, pii-channels, pii-worker ──> pii-graph, pii-abi, pii-core
pii-graph ──> pii-abi, pii-core, pii-store
pii-memory, pii-model, pii-tools, pii-wasm ──> pii-abi, pii-core [, pii-store]
pii-store ──> pii-abi, pii-core
pii-core ──> pii-abi
pii-abi ──> serde, thiserror, async-trait, futures-core, bitflags   (nothing else)
```

`pii-abi` may **never** depend on `tokio`, `reqwest`, `redb`, or `wasmtime`. ZeroClaw got this
right and it is the single most valuable structural decision available; unlike ZeroClaw, Pii
enforces it (`xtask deps-check` fails the build).

Hard file-size gate: **no source file over 2,000 lines**, checked in CI. This is a direct
reaction to `zeroclaw-runtime/src/agent/loop_.rs` (17,727 lines) and
`zeroclaw-channels/src/orchestrator/mod.rs` (1.4 MB). The gate is cheap and it is the only
thing that actually prevents that outcome.

### 2.2 Async strategy

Every dyn-dispatched trait uses `#[async_trait]`. Native `async fn` in traits (AFIT) is stable
but **not `dyn`-compatible**, and Pii stores `Arc<dyn Tool>`, `Arc<dyn Channel>`,
`Arc<dyn GraphNode>` everywhere. One `Box::pin` allocation per call is irrelevant next to an
LLM round-trip or a syscall. `async-trait` is `pii-abi`'s only proc-macro dependency.

Non-async traits (`Observer`, `RuntimeAdapter`-equivalents) stay sync — ZeroClaw's
`Observer` (`observability_traits.rs:348`) is deliberately synchronous with the comment "called
on the hot path... buffer internally." Correct.

### 2.3 Error type

```rust
// pii-abi/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum PiiError {
    #[error("capability denied: {0}")]        Denied(Capability),
    #[error("cancelled")]                     Cancelled,
    #[error("deadline exceeded after {0:?}")] Timeout(Duration),
    #[error("budget exhausted: {0}")]         BudgetExhausted(BudgetKind),
    #[error("not found: {0}")]                NotFound(String),
    #[error("invalid input: {0}")]            Invalid(String),
    #[error("upstream {provider}: {status:?} {message}")]
    Upstream { provider: String, status: Option<u16>, message: String, retry_after: Option<Duration> },
    #[error("worker {kind} died")]            WorkerDied { kind: String },
    #[error(transparent)]                     Internal(#[from] anyhow::Error),
}

impl PiiError {
    pub fn is_retryable(&self) -> bool { /* Upstream 429/5xx, Timeout, WorkerDied */ }
    pub fn is_terminal(&self) -> bool  { /* Denied, Invalid, BudgetExhausted, Cancelled */ }
}
```

`is_retryable` / `is_terminal` are what the graph scheduler routes on. This is the concrete fix
for ZeroClaw's blanket `anyhow::Result` in the ABI.

### 2.4 `Ctx` — the capability context that actually enforces

The thing `pi_agent_rust` documented but did not build.

```rust
// pii-core/src/ctx.rs
pub struct Ctx {
    cancel:    CancelToken,
    deadline:  Option<Instant>,
    budget:    Arc<Budget>,       // tokens, wall-time, tool-calls, USD spend — atomics
    caps:      Arc<CapSet>,       // the enforced grant set
    principal: Principal,         // User | GraphNode{run, node} | Plugin(PluginId) | Channel(&str)
    trace:     TraceCtx,          // run_id + span for trajectory correlation
    obs:       Arc<dyn Observer>,
}

bitflags! { pub struct CapKind: u32 { const FS_READ; const FS_WRITE; const NET; const PROC;
                                      const HW; const SECRETS; const MEMORY_WRITE; const SPAWN; } }

pub struct CapSet {
    kinds:      CapKind,
    fs_read:    PathScope,      // canonicalised allow/deny prefix sets
    fs_write:   PathScope,
    net_hosts:  HostScope,      // host+port allowlist, wildcard-aware
    proc_argv0: ArgvScope,      // permitted program basenames
    secrets:    SecretScope,    // namespace prefixes
}

impl Ctx {
    /// Accessors RETURN Result — the check happens here, not in a doc comment.
    pub fn fs(&self)   -> Result<FsCap<'_>,  PiiError>;
    pub fn net(&self)  -> Result<NetCap<'_>, PiiError>;
    pub fn proc(&self) -> Result<ProcCap<'_>, PiiError>;

    /// Attenuation only. `narrow` is INTERSECTED with self.caps — never widening.
    /// Budget is CARVED OUT of the parent's remaining budget, not duplicated.
    pub fn child(&self, principal: Principal, narrow: &CapSet, share: BudgetShare) -> Ctx;

    pub async fn checked<F, T>(&self, f: F) -> Result<T, PiiError>  // races cancel + deadline
    where F: Future<Output = Result<T, PiiError>>;
}

impl FsCap<'_> {
    /// Canonicalises (dunce::canonicalize), re-checks scope AFTER resolution to defeat
    /// symlink escape, then opens. There is no unchecked path.
    pub async fn open(&self, p: &Path, mode: OpenMode) -> Result<File, PiiError>;
}
```

Three rules that make this hold:

1. **`child()` is monotonically attenuating.** Intersection, never union. Same invariant as
   ZeroClaw's `ensure_no_escalation_beyond` (`subagent/mod.rs:87`), enforced by construction
   rather than by a validation call that someone can forget.
2. **Re-check after canonicalisation.** The classic escape is `workspace/link -> /etc`; scope
   checks on the pre-resolution path are theatre.
3. **Defence in depth, three layers.** `Ctx` is the *logical* layer (fine-grained, in-process,
   good errors). Kernel confinement via `nono` (Landlock/Seatbelt) applied at worker/child
   process start is the *enforcement* layer. WASI is the *isolation* layer for plugins. `Ctx`
   alone is not a security boundary against native code in the same process, and the plan must
   never pretend it is — that is exactly the trap `pi_agent_rust`'s doc comments fell into.

### 2.5 Core traits

```rust
// ─── pii-abi/src/tool.rs ─────────────────────────────────────────────────────
bitflags! {
    /// Fails closed: Default = WRITES_FS | MUTATES_EXTERNAL, i.e. serialised.
    pub struct Effects: u16 {
        const PURE; const READS_FS; const WRITES_FS; const NET;
        const SPAWNS_PROC; const MUTATES_EXTERNAL; const LONG_RUNNING;
    }
}

pub struct ToolSpec {
    pub name:        SmolStr,
    pub description: SmolStr,
    pub schema:      Arc<serde_json::Value>,   // Arc: cloned per prompt build
    pub output:      Option<Arc<serde_json::Value>>,
    pub effects:     Effects,
    pub required:    CapKind,                  // checked BEFORE dispatch, not inside
    pub tier_min:    ModelTier,                // hidden from models below this tier
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;
    async fn call(&self, ctx: &Ctx, args: serde_json::Value) -> Result<ToolOutput, PiiError>;
    /// Streaming progress for long-running tools (bash, browser). Default: no-op.
    fn subscribe(&self) -> Option<broadcast::Receiver<ToolProgress>> { None }
}

pub struct ToolOutput {
    pub blocks:    Vec<ContentBlock>,          // Text | Json | Image | Diff | FileRef
    pub structured: Option<serde_json::Value>,
    pub truncation: Option<Truncation>,        // { by: Lines|Bytes, kept, total } — pi's pattern
}

// ─── pii-abi/src/model.rs ────────────────────────────────────────────────────
pub enum ContentBlock {
    Text(String),
    Thinking { text: String, signature: Option<String> },   // round-trip opaque provider data
    Image { media_type: SmolStr, data: Bytes },
    ToolUse { id: SmolStr, name: SmolStr, input: serde_json::Value },
    ToolResult { id: SmolStr, blocks: Vec<ContentBlock>, is_error: bool },
}
pub struct Message { pub role: Role, pub content: Vec<ContentBlock>, pub cache_hint: CacheHint }

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn id(&self) -> &str;
    fn caps(&self, model: &str) -> ModelCaps;   // streaming, tools, vision, thinking, ctx_window, tier
    /// The ONE required method. Non-streaming is a default that drains this.
    async fn stream(&self, ctx: &Ctx, req: ChatRequest<'_>)
        -> Result<BoxStream<'static, Result<StreamEvent, PiiError>>, PiiError>;
    async fn complete(&self, ctx: &Ctx, req: ChatRequest<'_>) -> Result<ChatResponse, PiiError> { /* default */ }
    fn tool_encoding(&self, model: &str) -> ToolEncoding;  // Native | JsonInPrompt | Mcq
}
/// Borrowed, like pi's Cow<[Message]> — no per-turn deep clone of history.
pub struct ChatRequest<'a> {
    pub messages: &'a [Message], pub system: Option<&'a str>,
    pub tools: &'a [ToolSpec], pub model: &'a str,
    pub sampling: Sampling, pub thinking: Option<ThinkingBudget>,
}

// ─── pii-abi/src/channel.rs ──────────────────────────────────────────────────
bitflags! { pub struct ChannelCaps: u32 {
    const TYPING; const DRAFTS; const CHOICES; const REACTIONS;
    const ATTACHMENTS; const THREADS; const APPROVALS; const HEALTH; } }

#[async_trait]
pub trait Channel: Send + Sync {
    fn id(&self) -> &str;
    fn caps(&self) -> ChannelCaps;             // host only calls flagged optional methods
    async fn send(&self, ctx: &Ctx, msg: &Outbound) -> Result<MessageRef, PiiError>;
    async fn listen(&self, ctx: &Ctx, tx: mpsc::Sender<Inbound>) -> Result<(), PiiError>;
    // ~10 optional methods, all defaulted, all gated by ChannelCaps.
}

// ─── pii-abi/src/memory.rs ───────────────────────────────────────────────────
#[async_trait]
pub trait Memory: Send + Sync {
    async fn put(&self, ctx: &Ctx, doc: Doc) -> Result<DocId, PiiError>;
    async fn search(&self, ctx: &Ctx, q: &Query) -> Result<Vec<Hit>, PiiError>;
    async fn get(&self, ctx: &Ctx, id: DocId) -> Result<Option<Doc>, PiiError>;
    async fn forget(&self, ctx: &Ctx, sel: &Selector) -> Result<usize, PiiError>;
}
pub struct Query { pub text: String, pub k: usize, pub mode: RetrievalMode, // Bm25|Vector|Hybrid
                   pub filter: Filter, pub rerank: bool }

// ─── pii-abi/src/observe.rs ──────────────────────────────────────────────────
pub trait Observer: Send + Sync + 'static {     // sync — hot path, buffer internally
    fn event(&self, e: &Event);
    fn metric(&self, m: &Metric);
    fn flush(&self);
}
```

`ChannelCaps` and `Effects` as bitflags are what let a **native channel and a WASM channel
present identically** — the WIT world uses the same flag set (§5.2), so the host has one code
path.

---

## 3. The graph engine (`pii-graph`)

The centrepiece. Neither reference implementation has what's needed: ZeroClaw's SOP model is a
numbered step list with string guards bolted on (four-rule routing precedence documented at
`sop/route/mod.rs:49` because the interaction is genuinely confusing), and there is **no
sub-graph support at all** (grepped: `sub_sop`, `call_sop` — zero hits). `zeroclaw-sop-graph`
contributes only wire types.

### 3.1 Model

A **dynamic DAG-with-bounded-cycles, routed by data, checkpointed by journal.**

```rust
// pii-graph/src/spec.rs — the persisted, content-addressed form
pub struct GraphSpec {
    pub name:     SmolStr,
    pub inputs:   JsonSchema,
    pub outputs:  JsonSchema,
    pub caps:     CapSet,                  // ceiling for every node in this graph
    pub nodes:    Vec<NodeSpec>,           // index == NodeId
    pub edges:    Vec<Edge>,               // (from: NodeId, port: PortName, to: NodeId)
    pub entry:    NodeId,
    pub limits:   GraphLimits,             // max_steps, max_wall, max_tokens, max_usd
}
pub struct NodeSpec {
    pub id: NodeId, pub label: SmolStr,
    pub kind: NodeKind,                    // the discriminated payload, below
    pub bind: Vec<Binding>,                // blackboard path → node input field
    pub retry: RetryPolicy,                // { max, backoff, only_if: Retryable }
    pub loop_budget: Option<u32>,          // REQUIRED if this node lies on a cycle
}
```

**Ten built-in node kinds** — the "starter graph" primitive set. Everything composes from these:

| Kind | Purpose | Ports emitted |
|---|---|---|
| `llm` | one provider call, bound prompt + tool subset + optional output schema | `ok`, `refusal`, `overflow` |
| `tool` | invoke one tool by name with bound args | `ok`, `err` |
| `agent` | the full ReAct loop as a single node (llm↔tool until stop) | `done`, `max_steps`, `err` |
| `route` | classifier (rule-based, or `llm` with an enum output) | one port per class |
| `map` | fan-out over an array, bounded concurrency, gather | `ok`, `partial`, `err` |
| `subgraph` | call another `GraphRef` under an isolation policy | `ok`, `err`, + child's terminal ports |
| `gate` | approval / policy check → suspends | `approved`, `denied`, `expired` |
| `wait` | timer, cron, or named external event → suspends | `fired`, `timeout` |
| `transform` | pure JSON transform (small expression language, no I/O) | `ok`, `err` |
| `emit` | send on a `Channel`, publish MQTT, write a file | `ok`, `err` |

```rust
#[async_trait]
pub trait GraphNode: Send + Sync {
    fn kind(&self) -> &'static str;
    fn ports(&self) -> &'static [PortName];   // validated against edges at LOAD time
    fn required_caps(&self) -> CapKind;
    fn effects(&self) -> Effects;             // scheduler uses this for parallelism
    async fn run(&self, ctx: &Ctx, input: NodeInput<'_>) -> Result<Outcome, PiiError>;
}

pub enum Outcome {
    /// Routing is DATA-DRIVEN: the node names its own exit port.
    Next { port: PortName, value: serde_json::Value },
    /// The primitive that makes SOPs resumable, approvals possible, and a 3-day
    /// "wait for the user to reply" automation the same shape as a 200ms tool call.
    Suspend { reason: SuspendReason, key: ResumeKey, timeout: Option<Duration> },
    Fail { error: PiiError },
}
pub enum SuspendReason { Approval { policy: SmolStr, prompt: String },
                         Timer { at: SystemTime },
                         Event { topic: SmolStr, filter: Option<String> },
                         HumanInput { channel: SmolStr, prompt: String } }
```

**Why `Suspend` is on the node return type** rather than a run-status enum (ZeroClaw's
`SopRunStatus::{WaitingApproval, PausedCheckpoint}`, `sop/types.rs:694`): it makes suspension
compositional. Any node kind — including a third-party one — can suspend, and the engine
resumes it uniformly. In ZeroClaw suspension is a property the engine knows about for specific
step kinds; here it is a property of the node protocol.

**Routing.** Primary mechanism is `Outcome::Next{port}` — the node decides. Secondary is an
optional `guard` expression on an edge, evaluated against the blackboard by a small
side-effect-free expression evaluator (comparison, `and`/`or`/`not`, path access, `len`,
`matches`). Guards are the escape hatch; ports are the idiom. This avoids ZeroClaw's situation
where routing needed a documented four-rule precedence order between `when`, `switch`, `next`
and `terminal`.

**Blackboard.** `GraphState` is a versioned `BTreeMap<Path, Json>` plus per-node output slots.
Writes are `(node_id, port) -> value` under `$.nodes.<label>`; explicit `bind` declarations pull
from blackboard paths into node inputs. No implicit global mutation.

### 3.2 Load-time validation — the design's main safety property

`GraphSpec::validate()` runs before a graph can be stored or executed and rejects:

1. An edge whose `port` is not in the source node kind's declared `ports()`.
2. A node whose `required_caps()` ⊄ the graph's declared `caps`.
3. **Any cycle containing no node with a `loop_budget`.** This is the important one: an
   unbounded loop becomes a *load error*, not a runtime hang. Detect with Tarjan SCC
   (`petgraph`, already in three of the four reference lockfiles).
4. Schema incompatibility between a producer's output and a bound consumer's input, as a
   best-effort JSON-Schema subset check (warn, don't fail — schemas are often partial).
5. Unreachable nodes from `entry` (warn), and nodes with no outgoing edge on a declared port
   that isn't terminal (warn).

A distilled or user-authored graph that passes validation cannot hang the daemon or exceed its
capability envelope. That property is what makes accepting machine-generated graphs (§6.5)
tenable at all.

### 3.3 Composition and sub-graph isolation

```rust
pub struct SubgraphNode {
    pub graph: GraphRef,              // Pinned(GraphHash) | Named(name) — see §3.4
    pub policy: SubgraphPolicy,
}
pub struct SubgraphPolicy {
    pub caps:    CapSet,              // INTERSECTED with parent at call time
    pub budget:  BudgetShare,         // CARVED from parent's remaining, not duplicated
    pub state:   StateIsolation,      // Isolated{in,out} (default) | SharedRead | Shared
    pub isolate: Isolation,           // InProcess (default) | Wasm(PluginId) | Worker(kind)
    pub on_fail: FailPolicy,          // Propagate | Port("err") | Default(Json)
}
```

Defaults are the safe ones: intersected caps, carved budget, isolated state, in-process.
`Isolation::Worker` is the opt-in escape to a separate OS process (§4.3) for hardware or
long-running untrusted work; `Isolation::Wasm` runs the subgraph's nodes inside a component
(§5). Because isolation is a *policy field* rather than a structural difference, a subgraph can
be moved between in-process, WASM, and worker execution without editing the graph.

**Starter graphs → composite graphs.** Ship a small library of named starter graphs the user
composes by reference:

- `starter/react` — the coding agent's inner loop (`agent` node + tool set + compaction gate).
- `starter/classify-route` — `route` + N terminal ports; the escalation primitive.
- `starter/verify-fix` — `agent` → `tool:test` → `route(pass/fail)` → loop with `loop_budget`.
- `starter/approve-then-act` — `gate` → `subgraph` → `emit`.
- `starter/sense-decide-act` — `wait(event)` → `transform` → `route` → `emit`. The IoT idiom.
- `starter/distill` — `map(over: trajectories)` → `llm` → `tool:skill_write`.

The coding harness is `starter/react` with a bound tool set. A Telegram automation is
`starter/approve-then-act` wrapping `starter/react`. A thermostat rule is
`starter/sense-decide-act` with zero LLM nodes. This is the unification claim made concrete.

### 3.4 Storage in redb, dynamic routing, resume

**Content addressing.** `GraphHash = blake3(postcard(GraphSpec))`. Bodies are immutable and
keyed by hash; a *name* is a mutable pointer to a hash. This gives dynamic routing without
recompiling (repoint the name) while running instances stay pinned to the hash they started
with — no mid-run definition swap, which is a real hazard in any hot-reloadable workflow engine.

```
# ── graph definitions ────────────────────────────────────────────────────────
graph_body      : GraphHash                    -> Postcard<GraphSpec>
graph_ref       : &str                         -> GraphHash              # mutable pointer
graph_ref_log   : (&str, seq u64)              -> Postcard<RefChange>    # who/when/why, append-only

# ── run state ────────────────────────────────────────────────────────────────
run             : RunId(u128)                  -> Postcard<RunHeader>    # status, graph hash, revision, budget used
run_journal     : (RunId, seq u64)             -> Postcard<RunEvent>     # append-only; THE source of truth
run_snapshot    : (RunId, seq u64)             -> Postcard<Blackboard>   # periodic; an optimisation only
run_by_status   : (Status, RunId)              -> ()                     # secondary index, maintained in-txn
resume_index    : ResumeKey                    -> RunId                  # event/timer/approval → run
timer_wheel     : (fire_at_ms u64, RunId)      -> ResumeKey              # ordered scan for due timers
```

**Checkpointing: event-sourced journal + periodic snapshot.** Every node completion appends a
`RunEvent` **in the same redb write transaction** that advances `RunHeader` and updates
`run_by_status`. redb's single-writer MVCC gives this atomically — ZeroClaw hand-rolled the
equivalent in SQLite with `save_run_with_event` plus revision-guarded CAS
(`sop/store/mod.rs:19`); with redb it falls out of the transaction model. Snapshot every 32
events and unconditionally on `Suspend`.

```rust
pub enum RunEvent {
    Started { graph: GraphHash, input_hash: Hash, caps: CapSet },
    NodeStarted { node: NodeId, seq: u64 },
    NodeFinished { node: NodeId, port: PortName, value_hash: Hash, value: Json,
                   dur_ms: u32, tokens: Option<TokenUsage>, cost_micro_usd: u64 },
    NodeFailed { node: NodeId, error: String, retryable: bool, attempt: u32 },
    Suspended { node: NodeId, reason: SuspendReason, key: ResumeKey },
    Resumed { key: ResumeKey, payload_hash: Hash },
    Finished { terminal: Terminal, output_hash: Hash },
}
```

**Resume = load latest snapshot ≤ cursor, replay journal tail.** Because `NodeFinished` carries
the value, resume **never re-invokes a completed node** — no duplicate LLM spend, no duplicate
side effects. This is ZeroClaw's `llm_calls_saved` counter (`sop/types.rs:880`) done as a
structural property rather than an optimisation.

**Crash recovery.** On daemon start: scan `run_by_status` for `Running | Suspended`, rehydrate
each, re-arm timers from `timer_wheel`, re-subscribe `Event` waits. A run interrupted mid-node
replays that one node (so nodes must be idempotent-or-declared: `Effects::MUTATES_EXTERNAL`
nodes get a `NodeStarted` written *before* execution and are surfaced as "may have executed" on
recovery rather than blindly retried).

**Audit.** `pii graph trace <run_id>` renders the journal. Every LLM call, every tool call,
every routing decision, every capability denial, with cost and duration. This is free once the
journal exists and it is the difference between a system you can debug and one you can't.

### 3.5 Scheduler

Ready-set driven. A node is ready when every inbound edge on a taken port has fired.
Concurrency is bounded per run by a semaphore; `map` nodes take a sub-semaphore.
`Effects` gates parallelism the way `pi_agent_rust`'s `ToolEffects` gates parallel tool calls
(`src/tools.rs:150`): `PURE` and `READS_FS` nodes run concurrently; `WRITES_FS` and
`MUTATES_EXTERNAL` serialise against each other. Cancellation propagates through
`Ctx::cancel`. Budget is checked before dispatch and after each node — `BudgetExhausted` is
terminal and non-retryable.

### 3.6 Authoring format

TOML for humans; postcard-in-redb is canonical. `pii graph fmt | lint | apply | diff` round-trip.

```toml
name = "coding.verify-fix"
caps = ["fs_read", "fs_write", "proc"]
[limits]  max_steps = 40   max_usd = 2.00   max_wall = "20m"

[[node]]
id = "implement"; kind = "agent"
model = "tier:large"; tools = ["read","write","edit","bash","grep"]; max_steps = 25
[[node]]
id = "test"; kind = "tool"; tool = "bash"
args = { command = "cargo test --workspace" }
[[node]]
id = "triage"; kind = "route"; on = "$.nodes.test.exit_code"
rules = [ { eq = 0, port = "pass" }, { port = "fail" } ]
[[node]]
id = "fix"; kind = "agent"; model = "tier:large"; loop_budget = 3

[[edge]] from = "implement"; port = "done"; to = "test"
[[edge]] from = "test";      port = "ok";   to = "triage"
[[edge]] from = "triage";    port = "fail"; to = "fix"
[[edge]] from = "fix";       port = "done"; to = "test"     # cycle — bounded by fix.loop_budget
```

Explicitly **not** copying ZeroClaw's `SOP.md` round-trip Markdown parser (`sop/mod.rs:709`
`parse_steps` + `:1154` `render_steps`) — a bespoke Markdown parser you must keep bidirectional
is a permanent maintenance tax for a cosmetic benefit.

---

## 4. Dual-mode architecture

### 4.1 One binary, argv dispatch

```
pii                      → interactive TUI coding harness         (pii-tui)
pii -p "<task>"          → headless one-shot, ndjson on stdout     (CI / scripting)
pii acp                  → ACP server on stdio                     (embed in Zed/editors)
pii mcp                  → expose Pii's tools as an MCP server
pii daemon               → supervisor: bus, channels, cron, gateway, MQTT
pii worker --kind <k> --socket <path>   → attached worker process (self-spawned)
pii graph {run|resume|apply|ls|trace|lint}
pii skill {ls|show|learn|curate}
pii db {stats|compact|export}
```

### 4.2 Cold-start discipline

The 7.14 ms p95 in `pi_agent_rust`'s benchmark report is achievable and worth defending:

- **Mode dispatch before any global init.** `main()` parses argv and branches before touching
  config, redb, TLS roots, or tokio. No `lazy_static`/`OnceLock` that performs I/O at first
  touch on a path the fast modes hit.
- **Runtime per mode.** `current_thread` for `pii -p`, `pii worker`, `pii acp`;
  `multi_thread` for `pii daemon` and `pii` (TUI).
- **redb opened lazily**, and not at all for `pii --version`, `pii --help`, `pii graph lint`.
- **Build-time compressed assets** via `build.rs` with emitted `RAW_LEN`/`CRC32` constants —
  `pi_agent_rust/build.rs:51-83` is the template.
- **`[profile.release]`**: `opt-level = "z"`, `lto = "fat"`, `codegen-units = 1`,
  `panic = "abort"`, `strip = true`. Plus `[profile.perf]` at `opt-level = 3`, `lto = "thin"`,
  `debug = 1` for benchmarking.
- **CI contract**, not aspiration: `xtask bench-gate` fails the build if
  `pii --version` p95 > 20 ms or the stripped binary exceeds its size budget. Copy
  `pi_agent_rust/docs/contracts/startup-benchmark-contract.json` as the pattern.

### 4.3 Worker processes over UDS JSON-RPC (Option C — designed from scratch)

No reference repo does this, so every detail below is a decision, not a citation.

**Transport.** `SOCK_STREAM` Unix domain socket, newline-delimited JSON-RPC 2.0. Rejected
alternatives: `SOCK_SEQPACKET` (message framing is nice but portability and tooling are worse,
and ndjson framing is trivially correct); LSP-style `Content-Length` framing (heavier, no
benefit); shared memory (premature). Optional `SCM_RIGHTS` side-channel for **file-descriptor
handoff** — a privileged parent opens `/dev/gpiochip0` and passes the fd to an unprivileged
worker. Linux-only, behind a feature.

**Socket layout.** `$XDG_RUNTIME_DIR/pii/<instance-id>/<kind>.sock`, socket mode `0600`,
parent dir `0700`. The daemon **refuses to start** if either permission is wider — fail closed,
following `xai-grok-sandbox`'s self-verification posture.

**Protocol.**

```
→ worker.hello  {abi: 1, kind: "hw", pid, caps: ["gpio","i2c"], build: "<git-sha>"}
← worker.hello  {abi: 1, ok: true, grants: ["gpio:0-27","i2c:1"]}
→ worker.health {}                              (every 5 s; 3 misses ⇒ restart)
→ worker.shutdown {grace_ms: 5000}
→ <kind>.<method> {...}                         (gpio.write, i2c.transfer, mqtt.publish)
← notification: event.publish {topic, payload}  (worker → daemon, unsolicited)
```

`worker.hello` carries an **ABI version and build sha**; mismatch is a hard refusal, which is
what makes hot reload safe.

**Supervision.** Health poll every 5 s, restart after 3 consecutive failures, exponential
backoff 100 ms → 30 s capped, circuit-break to `Degraded` after 5 crashes in 60 s (surfaced on
the `Observer` bus and to the gateway). In-flight RPCs on a dead worker fail with
`PiiError::WorkerDied`; the graph scheduler treats that as retryable. On `Drop`, SIGTERM then
5 s grace then SIGKILL. (YantrikClaw's `companion_process.rs:189-243` is a good template for
the loop shape; the transport differs.)

**Hot reload.** Workers are the same binary. Spawn the new worker on `<kind>.sock.new`,
handshake, verify ABI; stop routing new calls to the old socket; drain in-flight; send
`worker.shutdown`; `rename(2)` the socket atomically. Zero-downtime, and a failed handshake
leaves the old worker untouched.

**When to use a worker — and when not to.** Justified: (a) drivers that panic or block
(`rppal`, `probe-rs`, `nusb`) — crash isolation is real; (b) privilege separation — the GPIO
worker holds capabilities the agent process must not; (c) blocking syscalls that would stall
the async runtime.

> **Scope correction to the brief.** `pii worker --graph <id>` as a default execution model is
> the wrong call. Per-node IPC plus JSON serialisation costs ~1–3 ms and buys nothing for an
> `llm` or `transform` node. **Graphs execute in-process by default;** worker isolation is an
> opt-in `SubgraphPolicy.isolate = Worker(kind)` for hardware, untrusted, or long-running
> subgraphs. Keep the flag for that case; don't build the architecture around it.

### 4.4 Process model

```
pii daemon  ── owns the redb file, the event bus, the cron wheel, the graph scheduler
   ├─ channel tasks   (in-process: cli, mqtt, telegram, webhook, email)
   ├─ gateway task    (axum 0.8: REST + WS + dashboard)
   ├─ graph scheduler (in-process node execution; wasmtime store pool)
   └─ worker children (UDS): pii worker --kind hw | --kind probe | --kind sandbox

pii (TUI) / pii -p / pii acp
   ├─ if a daemon is running → connect over UDS, runs execute daemon-side, events stream back
   └─ if not → standalone: own tokio runtime, own redb at a session path (§6.1)
```

---

## 5. WASM plugin host (`pii-wasm`)

### 5.1 Native vs WASM — the line, drawn explicitly

**Native, in-process** (needs real sockets, streaming, OS access, or is hot-path):
MQTT client and embedded broker (`rumqttc` / `rumqttd`), MCP client (`rmcp 2.1` — it spawns
processes and holds long-lived transports), provider HTTP + SSE, redb, the graph engine and
scheduler, memory/BM25/vector, `fs`/`edit`/`bash`/`grep` tools, TUI, ACP, gateway.

**Native worker process** (crash isolation / privilege / blocking): hardware GPIO/I2C/SPI/USB,
`probe-rs` flashing, anything using `pre_exec` + seccomp for confined child execution.

**WASM component** (untrusted, third-party, pure-ish compute with gated egress): third-party
tools, user-authored transform nodes, payload normalisers (e.g. a Zigbee2MQTT → canonical
schema adapter), format converters, domain adapters, and **executable steps of distilled skills
authored by the model itself**. That last case is why the WASM host earns its place: it is the
only safe way to run machine-generated code.

WASI has no GPIO, no raw sockets, and no pty. Anything needing those is native by definition —
that is the whole rule.

### 5.2 WIT world

`wasmtime 47.x` — both `zeroclaw` (47.0.4) and `pi_agent_rust` (47.0.3) landed there
independently, which is a useful signal. Component model / WASI 0.2 (P2), not core modules.

```wit
package pii:plugin@0.1.0;

interface types {
  type json = string;                       // tool args/results are schema-driven at runtime
  flags effects { pure, reads-fs, writes-fs, net, spawns-proc, mutates-external, long-running }
  record tool-spec { name: string, description: string, schema: json, effects: effects }
  variant err { denied(string), invalid(string), timeout, upstream(string), internal(string) }
}

interface host {
  use types.{json, err};
  log:       func(level: level, msg: string);
  config:    func() -> result<json, string>;         // public config only; x-secret fields stripped
  secret:    func(name: string) -> result<string, string>;  // host derives the namespace
  call-tool: func(name: string, args: json) -> result<json, err>;  // re-entrant, budget-checked
  now-ms:    func() -> u64;
  emit:      func(topic: string, payload: json);
}

interface tools   { use types.{tool-spec, json, err};
  list: func() -> list<tool-spec>;
  call: func(name: string, args: json) -> result<json, err>; }

interface node    { use types.{json, err};           // a plugin can implement a graph node kind
  ports: func() -> list<string>;
  run:   func(input: json) -> result<tuple<string, json>, err>; }   // (port, value)

world pii-tool-plugin { import host; import wasi:http/outgoing-handler@0.2.3; export tools; }
world pii-node-plugin { import host; export node; }
```

Design choices, with reasons:

- **`json-string` for payloads, typed records for the envelope.** `pi_agent_rust`'s
  `docs/wit/extension.wit` makes *everything* a JSON string (28 lines total) — maximally simple,
  but the host learns nothing at load time. ZeroClaw types the envelope richly. Pii splits it:
  the envelope (specs, effects, errors, capability flags) is typed so the host can validate and
  route; the payload stays JSON because tool arguments are schema-driven at runtime anyway.
- **Capability flags, same bitset as the Rust trait.** ZeroClaw's `flags channel-capabilities`
  (`channel.wit:119`) is the right pattern; Pii shares one flag definition between the WIT world
  and `pii-abi`, so `WasmTool` implementing `Tool` is a pure adapter with no capability
  translation layer.
- **`emit` in the host interface** lets a plugin publish onto the event bus — this is what makes
  a WASM protocol adapter useful in the IoT path without giving it a socket.

### 5.3 Capability grants

Manifest (`pii-plugin.toml`) declares required caps; grant decisions persist in redb, borrowing
`pi_agent_rust`'s `PersistedDecision` shape (`src/permissions.rs:29`) including
`version_range` — so a grant does *not* silently carry across a plugin upgrade that widens its
manifest.

```
plugin        : PluginId          -> Postcard<PluginRecord>   # name, version, wasm hash, manifest
plugin_grant  : (PluginId, Cap)   -> Postcard<Decision>       # allow, scope, expires_at, version_range
```

**Enforcement is at link time, not call time.** If `net` is not granted, `wasi:http` is simply
not wired into the `Linker` — the component fails to instantiate with a clear message, rather
than failing deep inside a call. Granted egress goes through a host-mediated `wasi-http`
implementation that checks the grant's host allowlist per request (ZeroClaw's
`PluginEgressHooks` in `wasi_http.rs` is the reference).

### 5.4 Resource limits

```rust
pub struct PluginLimits {
    pub max_memory_bytes: usize,     // StoreLimitsBuilder::memory_size
    pub max_table_elements: usize,
    pub epoch_deadline_ticks: u64,   // PRIMARY CPU control
    pub call_timeout: Duration,      // wall clock, tokio::time::timeout
    pub max_host_calls: u32,         // per call frame; ZeroClaw uses 1_000
    pub fuel: Option<u64>,           // opt-in, for metered/deterministic plugins only
}
```

**Opinionated departure from ZeroClaw**: they use fuel as the primary CPU control
(`consume_fuel(true)`, `component.rs:468`) and no epoch interruption at all (grepped: no epoch
usage). Pii inverts this — **epoch interruption is primary** (a background ticker bumps the
epoch; overhead is a periodic check rather than per-instruction accounting, materially cheaper
for compute-bound plugins), with fuel opt-in where deterministic metering actually matters.
Wall-clock timeout wraps every host-dispatched call regardless. No `Default` impl on
`PluginLimits` — ZeroClaw's reasoning is exactly right: "limits always come from the config
registry so no code path can construct an unsandboxed store by accident."

**Instance pooling.** `wasmtime::PoolingAllocationConfig` with pre-reserved slots. Component
instantiation is the dominant cost; pooling turns a plugin tool call into a µs-scale operation
and is the difference between "plugins are usable in a hot loop" and "plugins are for
occasional work."

---

## 6. Memory and self-improvement

### 6.1 redb: the decision, and its consequence

**Recommendation: adopt redb, with eyes open.** Reasons in favour: single-writer MVCC with real
ACID transactions (which is what makes §3.4's atomic journal+header write free); typed tables;
zero-copy reads; **pure Rust with no C dependency**, which matters directly for a static musl
binary. Reasons against, stated plainly: no repo in the reference set uses it (all four use
`rusqlite 0.37`); no SQL for ad-hoc inspection; **no FTS5, so BM25 is yours to write** (~600
LOC); smaller community.

**The consequence that changes the process model: redb is single-process (file lock).** The
daemon, an interactive TUI, and workers cannot all open the same file. Therefore:

> **The daemon owns the database.** When `pii daemon` is running, `pii`, `pii -p`, `pii acp`,
> and all workers connect to it over UDS and their runs execute daemon-side. When no daemon is
> running, `pii -p` and `pii` open their **own** database at a session path
> (`~/.pii/sessions/<id>.redb`) and merge into the main store on next daemon start.

This is a real architectural constraint, not a footnote — it means the UDS control channel
(§4.3) is needed in Phase 2, not Phase 3. Keep the `Memory` trait clean so a SQLite backend
remains a drop-in if this proves painful.

### 6.2 Schema

```
# ── documents / episodic memory ───────────────────────────────────────────────
doc            : DocId(u128)              -> Postcard<Doc>     # text, source, ts, tags, kind
doc_by_time    : (ts_ms u64, DocId)       -> ()
doc_by_tag     : (&str, DocId)            -> ()

# ── BM25 inverted index (hand-rolled; k1=1.2, b=0.75) ────────────────────────
postings       : (term &str, DocId)       -> Postcard<TermStat>  # tf, field mask
term_df        : term &str                -> u32
corpus         : ()                       -> Postcard<Corpus>    # n_docs, total_len, avgdl

# ── vectors ───────────────────────────────────────────────────────────────────
vec_block      : BlockId u32              -> &[u8]   # 256 × i8-quantised vectors, packed
vec_f32        : DocId                    -> &[u8]   # exact f32, for top-k rerank only
vec_slot       : DocId                    -> (BlockId, u16)

# ── skills ────────────────────────────────────────────────────────────────────
skill          : SkillId(u128)            -> Postcard<Skill>
skill_by_name  : &str                     -> SkillId
skill_ver      : (SkillId, ver u32)       -> Postcard<Skill>     # append-only history
skill_stats    : SkillId                  -> Postcard<SkillStats>

# ── trajectories ──────────────────────────────────────────────────────────────
traj           : RunId                    -> Postcard<TrajectoryMeta>
traj_by_day    : (day u32, RunId)         -> ()                  # export cursor
```

redb has no secondary indexes; every `*_by_*` table is maintained by hand **in the same write
transaction** as its primary. Encapsulate that in `pii-store` and let nothing else write.

### 6.3 Hybrid retrieval — two opinionated calls

**Vector search: i8-quantised brute force, no ANN index.** At 100 k documents × 768 dims × 1
byte = 76 MB, a SIMD dot-product scan runs in ~20 ms. That is well inside budget for a personal
assistant, and it eliminates an entire class of index-corruption, rebuild, and recall-tuning
problems. Scan i8, rerank the top 100 against exact f32. Add HNSW only when a measured p95
exceeds the budget — not before. (ZeroClaw does f32 brute force from SQLite BLOBs
(`vector.rs:4-33`) — same decision, 4× the bytes scanned.)

**Fusion: Reciprocal Rank Fusion (k=60), not weighted score normalisation.** ZeroClaw's
`hybrid_merge` (`vector.rs:71-90`) min-max-normalises BM25 and cosine and applies configured
weights with per-candidate weight renormalisation — thoughtful, but it requires calibrating two
incomparable score distributions and the weights become magic numbers. RRF
(`score = Σ 1/(k + rank_i)`) needs no calibration, is robust to distribution shift, and is one
line. This is a genuine improvement over the reference implementation.

### 6.4 Skills

```rust
pub struct Skill {
    pub name:    SmolStr,                 // [a-z0-9][a-z0-9_-]{0,63}
    pub summary: String,                  // ≤80 chars — ALWAYS resident in the prompt index
    pub triggers: Vec<String>,            // cheap prefilter terms
    pub body:    String,                  // markdown: When to Use / Prerequisites / Procedure /
                                          //           Pitfalls / Verification  (hermes' order)
    pub attachments: Vec<Attachment>,     // references/ templates/ scripts/
    pub graph:   Option<GraphRef>,        // ◄── the Pii differentiator (§6.5)
    pub provenance: Provenance,           // created_by, from_runs: Vec<RunId>, model, ts
    pub state:   SkillState,              // Active | Stale | Archived
    pub pinned:  bool,
    pub version: u32,
}
pub struct SkillStats { uses: u32, views: u32, wins: u32, losses: u32,
                        median_tokens_saved: i64, last_used: SystemTime, last_patched: SystemTime }
```

**Retrieval: progressive disclosure, with a scaling fix.** Hermes renders every skill's
`name: ≤60-char description` into every system prompt (`prompt_builder.py:2092`) and lets the
model self-select. That is correct and cheap — and it stops scaling somewhere past ~150 skills.
Pii's rule: below 150 skills, render the full index (hermes behaviour); above that, render all
`pinned` skills plus the top 60 by hybrid retrieval against the current task, with a
`skills_search(query)` tool as the escape hatch. Bodies load on demand via `skill_open(name)`.

**Content lives in redb; a file mirror is exported** to `~/.pii/skills/<name>/SKILL.md` for
`git`-based review and hand-editing, reconciled on daemon start by mtime.

### 6.5 The SkillDistiller loop

Hermes' central insight — *there is no separate distillation engine; the agent does it with its
own tools* (`learn_prompt.py:22`) — is right and Pii keeps it. What Pii adds is a **promotion
path from prose to executable graph.**

**Triggers** (all optional, all configurable):
1. Explicit `pii skill learn` / `/learn` in the TUI.
2. Post-run: a `distill` graph is enqueued when the run's outcome carries a user correction, or
   when `runs_since_distill >= 10` (hermes' nudge-interval, keyed on runs rather than tool
   iterations — runs are the natural unit here).
3. Weekly curator pass.

**Distillation run** = `starter/distill`, a graph like anything else: an `agent` node with tools
whitelisted to `skill_open` / `skill_write` / `memory_*`, fed the run's **journal** from redb
(structured, already summarised by node — much cheaper than hermes' verbatim conversation
replay, at the cost of prompt-cache warmth).

**Rules copied verbatim from hermes because they are load-bearing:**
- **Read-before-write enforced by the tool.** `skill_write` refuses a patch unless
  `skill_open(name)` was called in the same run (`background_review.py:542`). Not a prompt
  instruction — a tool precondition.
- **Preference ordering**: patch a loaded skill → extend an existing umbrella → add a support
  file → create a new umbrella skill. Last resort, not first.
- **Never delete**; archive is the maximum destructive action. Append-only `skill_ver` makes
  this free.
- **Do-not-capture list**: environment-specific failures, transient errors that resolved, one-off
  task narratives, unresolved failures written up as reliable workflows.
- **Lifecycle**: `active → stale (30 d) → archived (90 d)`, `pinned` opts out, only
  `created_by = "agent"` skills are curator-manageable.

**The promotion pass — Pii's addition.** When a skill has `uses >= 8` and its source runs share
a stable node sequence (edit-distance over the journal's node/port trace below a threshold),
the distiller proposes a `GraphSpec` and stores it as a **candidate**, not as the skill's
active graph. The candidate then **shadow-runs** alongside the next K=5 live invocations,
comparing terminal output; on ≥95% agreement it is promoted to `Skill.graph` and subsequent
invocations execute deterministically with **zero LLM tokens**. Disagreement demotes it and
records why in the journal.

This is the concrete mechanism behind the 9B ambition: the local model's job is not to be as
smart as a frontier model, it is to (a) route, (b) run already-promoted graphs, and (c) escalate
what neither covers. Every promotion moves work out of the LLM entirely. And because promotion
produces a `GraphSpec` that must pass §3.2 validation — bounded cycles, capability subset,
declared ports — machine-generated procedures cannot hang the daemon or exceed their envelope.

### 6.6 Trajectory logging for the 9B fine-tune

Two layers: the redb journal is the queryable source of truth; JSONL is the export.
`~/.pii/trajectories/YYYY-MM-DD.jsonl`, one record per run, written by `pii db export` or a
daemon cron.

```json
{
  "schema": "pii.trajectory.v1",
  "run_id": "01J...", "parent_run_id": null,
  "graph": {"name": "coding.verify-fix", "hash": "blake3:a41f..."},
  "started_at": "2026-09-01T22:10:03.221Z", "ended_at": "...", "wall_ms": 48211,
  "model": {"provider": "anthropic", "id": "claude-...", "tier": "large"},
  "env": {"os": "linux", "arch": "x86_64", "pii": "0.4.0", "cwd_hash": "blake3:...", "repo_head": "..."},
  "messages": [
    {"role": "system", "content": [{"type":"text","text":"..."}]},
    {"role": "user",   "content": [{"type":"text","text":"..."}]},
    {"role": "assistant", "content": [
       {"type":"thinking","text":"..."},
       {"type":"tool_use","id":"t1","name":"bash","input":{"command":"cargo test"}}]},
    {"role": "tool", "content": [{"type":"tool_result","id":"t1","is_error":false,"text":"..."}]}
  ],
  "steps": [
    {"node":"implement","kind":"agent","port":"done","dur_ms":31002,
     "tokens":{"in":18422,"out":2011,"cached_in":16000},"cost_micro_usd":41200,"retries":0},
    {"node":"test","kind":"tool","port":"ok","dur_ms":16880,"cost_micro_usd":0,"retries":0}
  ],
  "outcome": {
    "terminal": "success",
    "verifier": {"kind":"tests","passed":true,"detail":"cargo test: 218 passed, 0 failed"},
    "user_signal": {"accepted":true, "edits_after":0, "correction_text":null, "followup_within_min":null},
    "reward": 0.91,
    "reward_components": {"verified":1.0,"no_correction":1.0,"cost_penalty":-0.06,"latency_penalty":-0.03}
  },
  "skills": {"used":["rust-error-refactor"], "written":[], "promoted":[]},
  "escalation": {"local_attempted": true, "local_model": "qwen3-9b", "escalated": true,
                 "escalation_reason": "low_confidence_after_2_repairs"},
  "redaction": {"applied": true, "policy": "secrets-v1", "count": 3}
}
```

Five things this has that hermes' format does not, each for a reason:

1. **`steps[]` with per-node cost, tokens, and duration** — required to train cost-aware
   routing, which is the 9B's actual job.
2. **`outcome.verifier`** — a *programmatic* pass/fail (tests, compile, schema validation), not
   an LLM judgement. This is the difference between a reward signal and a vibe.
3. **`outcome.user_signal`** — accepted, edits-after, correction text, follow-up latency. The
   cheapest high-quality preference data available, and it is free to collect.
4. **`escalation`** — whether the local model tried, why it gave up. Directly trains the
   escalate/don't-escalate decision, which is the single highest-value behaviour to learn.
5. **`redaction`** — mandatory secret scrubbing *before* write, with the policy version
   recorded. A trajectory corpus is exactly the artifact that leaks credentials into a
   fine-tune. Non-negotiable, and it goes in from day one because retrofitting it means
   discarding the corpus.

**Failures are exported too**, in the same file, with `terminal: "failure"` and a reward below
zero. Hermes splits successes and failures into separate files and labels only `completed:
bool`; that discards the most informative examples.

### 6.7 Model tiering

Port `ModelTier` and `ModelCapabilityProfile` from `yantrikclaw/src/tools/tier.rs` (using the
code's 5/8/15/20 tool budgets, not the README's stale 10/20/25/30), plus the MCQ prompt and
its 3-strategy response parser from `src/tools/mcq.rs:62-237`.

**Pii's change: tier is a property of the `llm` node's bound provider, not a global setting.**
A single graph can run a `route` node on a local 9B (`tier: small`, MCQ encoding, 8 tools) and
an `implement` node on a frontier model (`tier: large`, native function calling, 20 tools). The
escalation path becomes structural — an edge in a graph — rather than a runtime policy branch
buried in the agent loop. That is also what makes escalation *observable*: it shows up in the
journal and in `escalation` in the trajectory.

---

## 7. Implementation phases

Every exit criterion is machine-checkable. A phase is not done because the code exists.

### Phase 0 — Prerequisites (½ week)

There is **no Rust toolchain in this environment** (`rustc: command not found`); the local
cargo registry cache holds 819 crates, so an offline build is partial at best.

Deliverables: toolchain pinned via `rust-toolchain.toml` (latest stable, edition 2024 — note
`zeroclaw` requires 1.96 and `pi_agent_rust` 1.95, so the floor is high); workspace skeleton;
`clippy.toml` with `disallowed-methods` from day one; `xtask` with `deps-check`, `size-gate`,
`bench-gate`, `file-size-gate`; CI.

**Exit:** `cargo xtask ci` green on an empty workspace; `deps-check` provably fails when
`pii-abi` is made to depend on `tokio`.

### Phase 1 — Core engine (3–4 weeks)

`pii-abi` (all traits), `pii-core` (`Ctx`, `CapSet`, `Budget`, `PiiError`, `Observer`),
`pii-store` (redb tables + migrations), `pii-graph` (spec, validator, scheduler, journal,
snapshot, resume), `pii-model` (Anthropic + Ollama), `pii-tools` (read, write, bash, glob,
grep), `pii-cli` (`graph run|resume|lint|trace`).

**Exit:**
1. `pii graph run examples/hello.toml` executes `llm → tool → llm` and prints the output.
2. `kill -9` the process mid-run; `pii graph resume <run_id>` completes it **without re-calling
   any already-completed node** (assert on journal: zero duplicate `NodeFinished` for the same
   `(node, seq)`).
3. `pii graph lint` rejects: an undeclared port, a cycle with no `loop_budget`, and a node whose
   `required_caps` exceed the graph's `caps`. Three tests, three distinct error variants.
4. A tool denied by `Ctx` returns `PiiError::Denied` and the denial appears in the journal.
5. `pii --version` p95 < 20 ms, gated in CI.

### Phase 2 — Coding harness (3–4 weeks)

`pii-tui` (ratatui 0.30 + crossterm 0.29), `pii -p` headless with ndjson, `pii-acp` (wrapping
`agent-client-protocol` 0.10.x, including the dedicated-stdin-thread and cancel-safe line
reader fixes from `xai-acp-lib`), edit/multi-edit tools, bash mediation + `nono` sandbox
profiles, sessions-as-runs, context compaction as a graph node, UDS control channel to the
daemon (needed early — see §6.1).

**Exit:**
1. Pii makes a real multi-file change in its own repo and `cargo test --workspace` passes,
   driven end-to-end by `starter/verify-fix`.
2. `pii -p "..."` runs in CI, exits non-zero on failure, emits parseable ndjson.
3. `pii acp` connects from Zed and completes a turn.
4. Sandbox self-verification: with a `workspace` profile active, a write to `/etc` fails at the
   **kernel** level (not just the `Ctx` check) and Pii refuses to start if it cannot verify
   enforcement.
5. Interactive p95 render latency under a 10k-line session stays under a documented budget
   (a `tui_perf` bench, following `pi_agent_rust/benches/tui_perf.rs`).

### Phase 3 — Assistant / gateway (3–4 weeks)

`pii daemon` (supervisor, event bus, cron/timer wheel), `pii-channels` (cli, webhook, telegram,
email), `pii-worker` (protocol + supervisor + hot reload), gateway (axum 0.8: REST + WS + a
minimal dashboard), `pii-memory` (BM25 + i8 vectors + RRF), MCP client (`rmcp 2.1`, in its own
crate to quarantine the `reqwest` version).

**Exit:**
1. 72 h continuous daemon uptime with RSS < 50 MB (sampled every 60 s; assert max, not mean).
2. Telegram message → graph run → reply, with the full journal visible in `pii graph trace`.
3. `kill -9` a worker; the daemon restarts it within 2 s, in-flight RPCs fail with
   `WorkerDied`, and the graph retries and completes.
4. Hot reload: replace the binary, reload the worker, zero dropped requests.
5. Hybrid retrieval beats BM25-alone on a 20-query hand-labelled set (recall@10).

### Phase 4 — IoT / reactive (2–3 weeks)

MQTT client + embedded broker, `pii-hw` worker (GPIO/I2C/SPI/USB via `rppal`/`nusb`/
`tokio-serial`, Linux + opt-in, with `SCM_RIGHTS` fd handoff), reactive rule graphs
(`starter/sense-decide-act`), retained device state in redb, `pii-wasm` host with the
`pii-node-plugin` world for protocol adapters.

**Exit:**
1. Sensor topic → rule graph → actuator, p95 end-to-end < 100 ms, measured over 1 h.
2. Zero LLM calls on the reactive path (assert: journal contains no `llm` node executions).
3. `kill -9` the hardware worker mid-automation; automations resume within 2 s; no lost events
   (the journal proves it).
4. A WASM payload-normaliser plugin runs with `net` denied and cannot reach the network —
   verified by an instantiation failure, not a runtime error.
5. Daemon RSS with MQTT + 20 devices + 10 automations still < 50 MB.

### Phase 5 — Self-improvement (3–4 weeks)

Skills store + file mirror, `starter/distill`, read-before-write enforcement, curator lifecycle,
graph promotion with shadow-running, trajectory export with redaction, local 9B routing with
tiered tool presentation and MCQ encoding.

**Exit:**
1. ≥10 skills distilled from real usage; on a held-out repeat-task set they measurably reduce
   median tokens per task (target ≥25%) — measured, not asserted.
2. ≥1 prose skill promoted to a `GraphSpec` and executing with **zero LLM tokens**, having
   passed shadow-run agreement ≥95% over 5 invocations.
3. ≥1,000 trajectories exported; a redaction audit over the corpus finds zero secrets (run an
   independent scanner, e.g. `gitleaks`, over the JSONL).
4. A local 9B handles ≥50% of `route` nodes with escalation on the rest, and the `escalation`
   block is populated on every run.
5. The curator archives stale skills without deleting anything; `skill_ver` history is intact
   and every archive is restorable.

**Total: roughly 15–19 weeks** to Phase 5 at a steady solo pace. Phases 1–2 alone (~7 weeks)
produce a coding agent that replaces the current one, which is the natural first stopping point.

---

## 8. Risks and tradeoffs

### 8.1 Where this over-engineers — and what to cut for an MVP

| Risk | Mitigation / cut |
|---|---|
| **Graph engine for a coding turn.** A ReAct loop is one `agent` node, not a graph. Forcing every turn through graph machinery adds ceremony and latency for nothing. | The `agent` node kind exists precisely so the loop stays a loop. Use graphs where there is genuine branching, suspension, or resumability. Do not decompose the ReAct loop into nodes. |
| **WASM host before any third-party plugin exists.** `wasmtime 47` is a large dependency tree and `bindgen!` breaks on majors. | Defer `pii-wasm` to Phase 4. It is behind a feature flag from day one; nothing in Phases 1–3 links it. |
| **Workers everywhere.** Per-node IPC costs ~1–3 ms for zero benefit on `llm`/`transform` nodes. | In-process by default; `isolate = Worker(kind)` opt-in for hardware/untrusted only (§4.3). |
| **14 crates for a solo project.** `grok-build`'s 154 crates and 15 k-line lockfile are a coordination tax with no in-tree evidence of payoff. | Split only for a stated reason (ABI stability, dependency quarantine, platform gating). If a crate has no reason line in §2.1, merge it. Starting at 6 crates and growing is also defensible. |
| **Distillation loop costs tokens.** Hermes spends ~30 K tokens per background review. | Feed the redb **journal**, not a verbatim conversation replay. Key the trigger on runs, not iterations. Make it opt-in in Phase 5 and measure §7-Phase-5 exit criterion 1 before enabling by default. |
| **Cognitive-memory scope creep.** Entities/relationships/beliefs/decay is a research project. YantrikClaw's version is an unvendored external crate for a reason. | Ship documents + BM25 + vectors + skills. Add a `nodes`/`edges` graph table (YantrikClaw's `knowledge_graph.rs:150` schema, ~200 LOC) only if a concrete query needs it. |

**MVP cut list — the smallest thing that is still Pii:** `pii-abi` + `pii-core` + `pii-store` +
`pii-graph` + one provider + five tools + `pii -p`. No TUI, no WASM, no workers, no channels
beyond CLI, BM25 only (no vectors), graphs from TOML files with no dynamic ref indirection.
That is roughly Phase 1 plus a week, and it is enough to prove the central claim — that graph +
journal + `Ctx` is the right substrate. If it isn't, everything after is wasted.

### 8.2 Crate and version pitfalls

- **redb single-writer.** All writes serialise. Never hold a write transaction across an
  `.await`. Batch journal appends. And the single-*process* lock forces §6.1's daemon-owns-the-DB
  model — the biggest downstream consequence in this document.
- **redb has no secondary indexes.** Every `*_by_*` table is hand-maintained in the same
  transaction. One crate (`pii-store`) owns all writes, or they will drift.
- **redb has no FTS.** Budget ~600 LOC for BM25 (tokeniser, postings, scorer, incremental
  update). SQLite's FTS5 is genuinely free; this is the concrete cost of the redb choice.
- **`wasmtime 47`** pins a large tree and bumps majors frequently; `bindgen!` output changes.
  Pin exact and budget for churn. Both `zeroclaw` (47.0.4) and `pi_agent_rust` (47.0.3) landed
  there independently — a useful signal, but not a stability guarantee.
- **`nono` must be exact-pinned.** `grok-build` pins `=0.53.0` because rule-emission *order*
  affects macOS Seatbelt deny precedence and a bump silently re-opened a
  `mv x y && cat y` bypass. Copy the pin and the comment.
- **`rmcp 2.1` requires `reqwest >= 0.13.2`** while most of the ecosystem is on 0.12. This is
  exactly why `xai-grok-mcp` exists as its own crate. Quarantine it the same way.
- **`async-trait` boxes per call.** Fine. AFIT is not `dyn`-compatible and Pii needs trait
  objects; revisit only if `dyn` AFIT stabilises.
- **musl static linking**: `rustls` only, never OpenSSL. `rppal`/`probe-rs`/`nusb` may not build
  cleanly against musl — the hardware worker can ship as a separate glibc build (it is a
  separate process anyway, which is a nice second-order benefit of §4.3).
- **`panic = "abort"` + worker processes** is a good pairing: a panicking worker dies cleanly and
  the supervisor restarts it, rather than unwinding into a half-initialised driver.
- **MSRV floor is high** (1.95/1.96 across the references, edition 2024). Fine for a new project;
  it does constrain distro packaging.

### 8.3 Process discipline worth adopting from the references

- **`clippy.toml` `disallowed-methods`** (`grok-build`) — ban `reqwest::Client::new`,
  `Path::canonicalize`, `dirs::home_dir`, and raw `Command::spawn`. Architecture enforced by the
  compiler beats architecture enforced by a document.
- **Module-reachability lint** (`pi_agent_rust/AGENTS.md:619`) — every `pub mod` needs ≥1
  non-test call site. Written after five modules shipped with green tests that nothing ever
  called. Tests are not reachability.
- **Perf claims as CI contracts** (`pi_agent_rust/docs/contracts/startup-benchmark-contract.json`)
  — cold start, binary size, and TUI frame budget are gates, not README numbers.
- **File-size gate** — 2,000 lines, hard fail. The direct antidote to a 17,727-line agent loop.
- **Read-before-write as a tool precondition** (`hermes-agent/background_review.py:542`) — the
  only reliable defence against an LLM blind-overwriting its own knowledge base.

---

## 9. Decision points for the user

Three forks where I have a recommendation but the call is yours. All are captured above; listed
here so they are not buried.

1. **redb vs SQLite.** Recommendation: **redb**, accepting the hand-written BM25 (~600 LOC) and
   the daemon-owns-the-DB consequence (§6.1). SQLite/`rusqlite` is what all four reference repos
   chose, gives FTS5 free, and allows multi-process access — if the daemon-ownership constraint
   feels wrong, that is the signal to switch, and the `Memory`/store traits keep it a swap
   rather than a rewrite.
2. **First milestone.** Recommendation: **Phases 1–2 (~7 weeks)** to a coding agent that
   replaces the current one, then reassess. The alternative is the §8.1 MVP cut (~5 weeks) to
   prove the graph substrate before committing to the harness.
3. **`pii worker --graph <id>`.** Recommendation: **drop it as a default execution mode** and
   keep worker isolation as an opt-in `SubgraphPolicy` field (§4.3). Worker processes are for
   hardware and untrusted code, not for general graph execution.

## 10. Verification

There is no code yet, so verification here means the plan's own claims:

- **Repo claims**: every citation in §1 and §0 is `path:line` against
  `/projects/pi-improved/_repos/` and re-checkable with `sed -n '<line>p' <path>`.
- **Version claims**: from the four `Cargo.lock` files — `wasmtime` 47.0.4 (zeroclaw) / 47.0.3
  (pi), `agent-client-protocol` 0.10.4, `rmcp` 2.1.0, `nono` 0.53.0, `ratatui` 0.30.0,
  `rusqlite` 0.37.0, `axum` 0.8.x, `rumqttc` 0.25.1, `landlock` 0.4.5, `petgraph` 0.8.3. **redb
  appears in none of them.**
- **Environment**: `rustc`/`cargo` are not installed here (Phase 0), and no `target/` directories
  exist in any cloned repo.
- **First executable proof of the architecture** is Phase 1 exit criterion 2: kill a run
  mid-flight and resume it without re-invoking a completed node. If the journal + snapshot
  design is wrong, that test fails, and it fails in week 4 rather than week 15.


---

## [merged] Pii — Pass 3: Tier 0 on the CPU, and the harness/DAG/pipeline substrate

<!-- Absorbed from `002-pii-architecture-plan-pass2.md` on 2026-09-28T20:00:57Z during the SSOT corpus merge.
     Content preserved verbatim. Original moved to 99-archive/06-research-merged-2026-09-28/. -->

> Extends `/projects/pi-improved/_docs/06-research/002-pii-architecture-plan-pass2.md`.
> Where pass 3 and pass 2 disagree, pass 3 wins and says why. Pass 2 §9 is the section
> that changes most; §3 (pass 1), §6.4 and §14 change materially; everything else stands.

---

## Context

Pass 2 §9 (`002-...-pass2.md:1453`) framed Needle 2 as an **MCU** story: a 45 M-param
grammar-constrained tool-call dispatcher on an ESP32-S3-N16R8 at 1.87 tok/s, ~25 s per
request, Phase 6, optional, gated on buying a board (§15 decision 6, `:2201`).

That framing made a category error. It attached a **role** (Tier-0 dispatcher) to a
**deployment** (a $10 microcontroller), and then correctly concluded that the deployment
was slow and marginal — which buried the role. The role is the valuable part, and its
natural home is the same CPU the daemon already runs on.

Two facts verified this pass:

1. **Needle 2 decodes at ~500 tok/s on a Raspberry Pi 5**, 300–700 tok/s on sub-$200
   phones, 400–1500 on VR-class devices. On the daemon host it is a **~250× speedup over
   the ESP32 port**. The user's premise is correct.
2. **There is no GGUF, no Ollama tag, and no llama.cpp path.** Weights are CQ2-bit
   ("Cactus Quants") trained at 2-bit and *baked into Cactus's own C++ engine*; schemas
   compile into the decode grammar; the engine binary is fetched once from HuggingFace and
   cached. The only first-class binding is Python (`pip install cactus-needle`).

Fact 1 makes the CPU reframe right. Fact 2 makes it impossible to satisfy the reframe with
`candle` and this model, and forces the architecture to be written against a **role**, not
against Needle.

**Intended outcome of pass 3:** Tier 0 becomes a real, testable, hardware-free part of the
daily-work loop starting in Phase 1 instead of a Phase-6 gadget; and the scheduler
semantics that orki got wrong get specified precisely enough that the diamond test can
actually be written against them.

---

## 1. Critical verdict — is nano-on-CPU a real helper inside Pii?

**Yes, but only in one shape, and pass 2's own §9.2 already half-knew it.** A 45 M-param,
256-token-context, single-call, grammar-constrained model is a **classifier with a typed
output**. Nothing more. Every honest use follows from that sentence.

### 1.1 What it is genuinely good for

| Job | Why nano wins | Displaces |
|---|---|---|
| **Route an utterance to a pipeline** | one call, closed label set, output is a port name | a 9 B call (~1 s) or a hand-written regex table that rots |
| **Extract structured args from NL** | this is literally its training objective; grammar guarantees the JSON parses and is in-bounds | a 9 B call + a repair loop |
| **Cheap adjudication of a node's output** ("is this test failure a flake or a real break?") | runs in the *inner* loop of `starter/verify-fix`, many times per task | a 9 B call per loop iteration |
| **Always-on trigger gate** ("does this journal event warrant waking the self-improvement graph?") | fires 10⁴×/day at zero cost so the frontier fires 5×/day | a cron heuristic, or nothing |
| **Offline degradation floor** | works when Ollama is down, the network is out, or the API key expired | a hard failure |

The load-bearing property is **not** the parameter count and **not** the latency. It is
**grammar-constrained decoding**: the output is schema-valid *by construction*, so the
graph engine's parse step is infallible. Pass 2 §9.1 states this correctly ("Parsing is
infallible. This is the real value.") and then spends the rest of §9 on flash offsets.

Honest latency, corrected: ~40–100 ms end-to-end on a Pi 5 for a full dispatch (short
prefill over a pruned tool table + ~20–40 decoded tokens), likely 10–30 ms on a modern
x86 desktop. **Not** the 1–10 ms in the brief — the brief conflated decode throughput with
end-to-end latency. Still 10–50× faster than a local 9 B and 20–100× faster than a
frontier round-trip, and it does not occupy the single Ollama slot.

### 1.2 What it cannot do — state this in the doc so nobody tries

- **Anything over 256 tokens of context.** No file contents, no diffs, no stack traces, no
  conversation history, no repo layout.
- **Anything multi-step.** `max_agent_steps: 1` in pass 2 §9.5 is not a policy, it is a
  physical description.
- **Anything requiring world knowledge.** 45 M params holds a grammar and a tool table.
- **Anything novel.** New tool, new phrasing, new domain → it guesses inside the grammar,
  confidently, and the grammar makes the guess *look* well-formed.
- **Judgement about quality.** "Is this code good", "is this plan sound", "did the agent
  actually do what was asked" — all Tier 2/3.

**The trap to write down explicitly:** grammar-valid ≠ correct. Pass 2 §9.1's quoted claim
"guaranteed present, correctly typed, within declared bounds — **no validation needed**"
is about *parsing*, and it is one careless reading away from being taken as a correctness
guarantee. Confidence measures **groundedness**, not correctness (pass 2 §9.1 says so, in a
table cell that is easy to skim past). Therefore, as a hard architectural rule:

> **A `nano` node may never be the last decision before a `MUTATES_EXTERNAL` effect
> without an intervening `gate` or `verify`.** Enforced at load time (§4.6, rule V7).

### 1.3 "Self-improvement with a nano model" — naive as stated, real when inverted

**Naive:** the nano model performs self-improvement. False, and not marginally. It cannot
read a trajectory, cannot read a diff, cannot author a `GraphSpec`, cannot judge whether a
skill helped. Self-improvement (pass 1 §6.5 `SkillDistiller`, pass 2 §14 Phase 5) is a
9 B/frontier job and stays one.

**Real, inverted — and this is the good version:** *the nano tier is the **object** of
self-improvement, not the subject.*

Pii's journal already emits, for free, the exact supervised dataset a router needs:
`(utterance → port actually taken → whether the run's `verify` passed)`. Pass 1 §3.4's
`RunEvent::NodeFinished` carries the port; pass 2 §6.2's `Verify::Delta` supplies a
**programmatic** correctness label with no LLM in the loop. So:

```
pii nano tune
  1. read the journal: every `nano` node dispatch, its predicted port, its confidence,
     the port the run actually ended up taking, and the terminal `Verify` result
  2. refit the classifier / prune the label set / fit per-label confidence thresholds
     — maximise direct-dispatch rate subject to verified-error-rate < ε
  3. report the delta; apply only with --apply; keep the old model addressable by hash
```

This is pass 2 §9.6's `pii mcu tune` — *generalised off the MCU and onto the host*, where
it can run from day one against real traffic instead of waiting for 100 dispatches from a
board nobody bought yet. It is LLM-free, measurable, and it produces the labelled
escalation corpus that pass 1 §6.6 wants for the 9 B fine-tune.

**Second real use:** nano as the *trigger* for self-improvement, not the performer. A
`nano` node in mode `classify` sitting on the event bus deciding "is this failure pattern
worth distilling?" costs nothing and gates a $-costing frontier graph.

### 1.4 "DAG running with a nano model" — naive as stated, real as a node kind

**Naive:** the nano model orchestrates/schedules the DAG. False *and* actively harmful. It
would destroy the single most valuable property in pass 2 §6.2's closing paragraph — **the
scheduler makes no judgements**. A scheduler with a model in it is not resumable (the model
is nondeterministic), not auditable (the journal records a decision with no rule behind
it), and not testable.

**Real:** the harness runs the DAG; the nano model is a **node inside it**, in three modes:

- `mode = "route"` — pick an exit port from a closed label set. Ports = labels + `escalate`.
- `mode = "classify"` — adjudicate another node's *output* into a closed set. Ports =
  classes + `escalate`.
- `mode = "extract"` — NL → schema-valid args for a named tool. Ports = `ok`, `escalate`.

All three share one backend call, one confidence contract, one tune loop. They differ only
in port set, which is why they are one node kind with a `mode` field and not three kinds.

**Be honest about `classify`:** for "did the build succeed?" you do not need a model —
exit code plus a regex is better, cheaper, and deterministic. The nano `classify` node
earns its keep only where the signal is genuinely fuzzy natural language: triaging a test
failure into `flake | compile-error | assertion | oom`, triaging a log line, deciding
whether a user's follow-up is a correction or a new request. Do not put a model where an
`if` will do — say this in the docs, because the temptation is strong once the node kind
exists.

### 1.5 Where nano is *worse* than what pass 2 already has

Three cases where pass 2's existing machinery beats a nano call, worth stating so the node
kind does not metastasise:

1. **Closed structured input** → `transform` (pure JSON expression, `Effects::PURE`, zero
   latency, deterministic, journal-replayable). If the input is already JSON, do not
   embed it and classify it.
2. **Rule-expressible routing** → `route` with rules. Pass 2 §9.5's `mcu.dispatch` example
   is exactly this and correctly uses `route`, not a model.
3. **Anything needing repair** → 9 B. `repair_attempts: 0` in pass 2 §9.5 is right: nano's
   answer to a bad answer is *escalate*, never *retry*.

---

## 2. The reframed tier model — same role, two homes

Replace pass 2 §9.5's three-row table (`:1566`) with this.

**Tier 0 (Nano)** is a *role*: one grammar-constrained call producing a schema-valid
routing/classification/extraction decision plus a groundedness score. It has **two homes**,
and the same tool-table downcast, the same confidence thresholds, and the same
`pii nano tune` loop serve both.

| Tier | Role | Home A (primary) | Home B (edge) |
|---|---|---|---|
| **0 · Nano** | route / classify / extract; one call; no reasoning; escalates | **host CPU**, in the daemon or a `pii-nano` worker — **10–100 ms**, $0, always on. **This is where daily work happens.** | **ESP32-S3-N16R8** wall panel / voice button — ~25 s, $0, *offline & private*. Same decision contract, different transport. |
| **1 · Small** | routing, running promoted graphs, tool-arg repair, summarisation, cheap planning | local 9 B via Ollama — ~1 s, $0 | — |
| **2 · Large** | novel reasoning, multi-file code, planning, distillation, self-improvement | frontier API — 2–30 s, $ | — |

**What this reframe buys, concretely:**

- Tier 0 becomes testable in **Phase 1** with zero hardware, instead of Phase 6 gated on a
  purchase decision (pass 2 §15 decision 6).
- The tool-table downcast (pass 2 §9.4 `:1534`), the confidence-threshold fitting (§9.6),
  and the escalation-ladder graph (§9.5) all get exercised against real daily traffic
  months before an ESP32 exists. **None of the pass-2 MCU research is wasted** — it stops
  being the definition of the tier and becomes the *deployment* half of a tier whose
  *decision* half is now validated on the host.
- The MCU's genuine, non-substitutable value is stated cleanly and narrowly: **offline
  resilience, privacy (the utterance never leaves the room), and cheap physical
  distribution** (many wall panels at $10 each). It is not, and never was, a latency or
  throughput play. Pass 2 §9.2 says this correctly; pass 3 just stops burying it.

**Naming:** `ModelTier::Nano` (pass 2 §9.5) stays as the capability profile. Add
`NanoHome::{Host, Mcu(McuId)}` so a `nano` node can be pinned to a home or left to the
router (default: `Host`, fall back to `Mcu` only if explicitly configured — never the
reverse, because a 25 s fallback on a latency path is worse than an error).

---

## 3. The inference backend decision — the one the brief got wrong

The brief proposed candle vs ggml-FFI vs llama.cpp-FFI. **All three are moot for Needle 2**,
because Needle 2 does not exist in any of those formats. Verified: no GGUF, no Ollama tag,
no llama.cpp path; CQ2-bit weights are baked into Cactus's own C++ engine, which is fetched
as a prebuilt binary from HuggingFace and cached, with Python as the only first-class
binding.

That is a **vendor-and-artifact lock**, and it is a much bigger deal than a C dependency:

- The engine is a **prebuilt binary from a remote host**, not source you compile. That is a
  supply-chain surface `cargo deny` cannot see and `cargo vendor` cannot pin.
- Reproducibility: "fetched once and cached" means the build is not hermetic.
- musl/static-linking and Linux-ARM64 support are **unstated**. The stated accelerated
  targets are NVIDIA and Apple Silicon.
- Apache-2.0 covers the *model*; the engine's redistribution terms need reading before
  anything ships.

### 3.1 The decision

**Do not put Needle 2 — or any nano model — in the main binary. Put the *role* in the ABI
and the *model* behind a trait with three backends, defaulting to the one with no new
dependencies at all.**

```rust
// pii-abi/src/nano.rs  — ABI STABILITY, no deps beyond serde/thiserror
#[async_trait]
pub trait NanoRouter: Send + Sync {
    fn id(&self) -> &str;                     // content-addressed: backend + weights hash
    fn labels(&self) -> &[SmolStr];           // the closed decision space
    async fn decide(&self, req: NanoReq<'_>) -> Result<NanoDecision, PiiError>;
}
pub struct NanoReq<'a> { pub mode: NanoMode, pub text: &'a str,
                         pub labels: &'a [SmolStr], pub schema: Option<&'a JsonSchema> }
pub struct NanoDecision { pub label: SmolStr, pub args: Option<Value>,
                          pub confidence: f32,          // GROUNDEDNESS, not correctness
                          pub backend: SmolStr, pub dur_us: u32 }
```

| # | Backend | What it is | Latency | Deps | Verdict |
|---|---|---|---|---|---|
| **B1** | `Embed` | `Arc<dyn Embedder>` (already required by pass 2 §5.3) + nearest-centroid / multinomial-logistic head fitted from the journal | **1–5 ms** local, ~10–30 ms via `OllamaEmbedder` | **none new** | ✅ **DEFAULT.** Pure Rust, static-musl-clean, deterministic, calibrated, *and it is the only backend that learns from `pii nano tune` without retraining anyone's LLM.* |
| **B2** | `Worker(cactus)` | Needle 2 in a **`pii-nano` T4 worker process** (pass 2 §6.1 tier T4), UDS + the existing worker protocol | 10–100 ms + ~1 ms IPC | Python/C++ **outside** the Rust binary | ✅ **Optional.** Use when you need true NL→args *extraction*, which B1 cannot do. The worker split (pass 2 §16.1 risk 9) is exactly the mechanism that makes a non-musl dependency survivable — same trick as `pii-hw`. |
| **B3** | `Candle` | `candle` + `llguidance` + Qwen-0.5/0.6B-class GGUF-free weights | 0.5–3 s | +candle, +llguidance, +350 MB weights | ❌ **Do not build.** At 0.5 B and ~1 s this *is* Tier 1, and Ollama already serves Tier 1. Building it duplicates Ollama in-process for no latency win. |

**Why B1 is the default and not a compromise.** The routing job — "which of N known
pipelines does this utterance belong to?" — is a closed-set classification problem with
labelled training data arriving continuously from the journal. A fitted classifier beats a
prompted 45 M LLM on that task on every axis that matters here: it is deterministic
(replayable from the journal), it is *calibrated* (confidence means something, unlike
groundedness), it costs one embedding you were already computing for memory, it adds zero
build dependencies, and it improves automatically as the system runs. The nano LLM's
advantage — free-form NL → arbitrary schema-valid args — is real but is a *different job*
(`mode = "extract"`), and that is the job B2 exists for.

**Coupling pass 2 did not have (flag it):** pass 2 §15 decision 8 (`:2201`) picks
`OllamaEmbedder` as the default embedder. If B1 is the default Tier-0 backend, then
"Tier 0 works offline" is **false in the default configuration** — the router needs Ollama
up. Either accept that (Tier 0's offline story is then B2-on-worker or the MCU), or let
this push decision 8 toward option (c) `candle` embeddings. Do not leave it unstated.

**If Needle 2 is used (B2), non-negotiables:**
1. Pin the engine artifact **by content hash**, vendored into the repo or an internal
   store. Never fetch-at-runtime from HuggingFace on a production path.
2. `pii-nano` ships as a **separate build** (glibc/whatever it needs). The main binary
   stays pure-Rust static-musl.
3. The worker speaks the pass-1 §4.3 protocol verbatim: `hello` with ABI version + build
   sha, 5 s health poll, circuit-break to `Degraded`, `PiiError::WorkerDied` (retryable).
   Worker down ⇒ the `nano` node takes its `escalate` port. **Tier 0 being unavailable
   must degrade to Tier 1, never fail the run.**
4. Read the engine's redistribution terms before Phase 3.

---

## 4. The harness, and the scheduler semantics orki got wrong

### 4.1 Define "harness" once, and only once

> **The harness is the runtime that executes graphs.** It is exactly five things:
> **(1)** `Ctx` — capabilities, budget, cancel, trace (pass 1 §2.4);
> **(2)** the **scheduler** — readiness, join, effects, concurrency (§4.3 below);
> **(3)** the **journal** — event-sourced truth and resume (pass 1 §3.4);
> **(4)** the **node registry** — kind → impl, where an impl may be native, WASM, a worker,
> a subgraph, or a nano backend;
> **(5)** the **validator** — load-time safety (pass 1 §3.2).
>
> The node contract is the harness ABI, and pass 1 already has it:
> `async fn run(&self, ctx: &Ctx, input: NodeInput<'_>) -> Result<Outcome, PiiError>`.
> The harness never inspects what is behind a node.

**Consequence, stated loudly in the docs:** there is exactly **one** harness. "The coding
harness" is not a harness — it is `pipeline:coding.react`, a graph. The IoT rule engine is
a graph. The nano router is a node. **If a second scheduler is ever written, the design has
failed.** This one sentence is worth more than a page of diagrams, because "let me write a
small loop for this special case" is exactly how orki ended up with a scheduler that could
not run a diamond.

### 4.2 The orki bug, restated as two precise defects

Pass 2 §6.4 (`:1143`) records the symptom. The design fix requires naming the mechanism:

1. **Single-task fan-out.** The scheduler took *one* ready task per iteration and did not
   revisit the rest. Root cause: `ready.pop()` where `ready.drain()` was needed.
2. **Any-predecessor convergence.** A node with multiple `depends_on` fired when *any* one
   completed. Root cause: no distinction between "an inbound edge fired" and "the join is
   satisfied".

Impact: `setup → {architect, builder} → fusion` silently skipped `builder` **and fabricated
provenance**. Confidently wrong output, green tests.

Pass 1 §3.5 (`001-...:670`) currently specifies readiness in one sentence — *"a node is
ready when every inbound edge on a taken port has fired"* — which is **not implementable as
written** for a data-routed graph. In a routed graph, some inbound edges are on ports that
will *never* be taken. Under a naive "every inbound edge must fire" rule, every join
downstream of a `route` hangs forever. That ambiguity is precisely the gap orki fell into,
and it must be closed with an explicit state machine.

### 4.3 The state machine — edge liveness, dead propagation, join policy

```rust
// pii-graph/src/sched.rs
pub enum EdgeState { Pending, Fired { value: ValueRef }, Dead }

pub enum JoinPolicy {
    All,                                     // default: every inbound Data edge Fired
    Any    { cancel_losers: bool },          // first Fired wins    ← replaces the `race` node kind
    Quorum { k: u16, cancel_losers: bool },
    AllSettled,                              // every inbound edge settled; pass the Fired subset
}
```

**Edge transitions.** When node `n` completes on port `P`:

```
for e in outbound(n):
    e.state = if e.kind == Order || e.port == P { Fired } else { Dead }
```

**Dead propagation** — the piece that makes routed joins terminate:

```
when node n becomes Dead (all its inbound Data edges are Dead):
    for e in outbound(n, Data):  e.state = Dead
    for e in outbound(n, Order): e.state = Fired   // Order edges SURVIVE dead branches.
                                                   // That is the entire point of Order
                                                   // (pass 2 §6.2: "always run the cleanup").
```

**Readiness** — one function, total, no hidden cases:

```rust
fn readiness(n: NodeId) -> Readiness {
    let data  = inbound(n, Data);
    let order = inbound(n, Order);
    if order.iter().any(|e| e.state == Pending) { return NotReady }   // Order waits for settle
    match n.join {
        All        => if data.any(Pending)      { NotReady }
                      else if data.all(Dead)    { DeadNode }
                      else if data.any(Dead)    { UnsatisfiableJoin }   // FAIL LOUDLY
                      else                      { Ready },
        Any{..}    => if data.any(Fired)        { Ready }
                      else if data.all(Dead)    { DeadNode }
                      else                      { NotReady },
        Quorum{k,..}=> if data.count(Fired) >= k                       { Ready }
                      else if data.count(Fired)+data.count(Pending) < k { UnsatisfiableJoin }
                      else                                              { NotReady },
        AllSettled => if data.any(Pending)      { NotReady }
                      else if data.all(Dead)    { DeadNode }
                      else                      { Ready },
    }
}
```

`UnsatisfiableJoin` → `Terminal::Failure` with `PiiError::Invalid("join can never be
satisfied")`, **never a hang**. This is the single most important property in the section: a
broken join is loud, immediate, and named in the journal. orki's version was silent.

**The scheduler tick** — the fan-out fix, three lines that matter:

```rust
loop {
    let batch: Vec<NodeId> = nodes.iter().filter(|n| readiness(n) == Ready).collect();
    //         ^^^^^^^^^^ DRAIN ALL READY NODES. Never `.next()`, never `.pop()`.  ← orki bug 1
    if batch.is_empty() {
        if in_flight == 0 { return terminal_determination(); }   // complete, or no-progress → Fail
        await_next_completion().await;  continue;
    }
    let (par, ser) = batch.partition(|n| n.effects().is_parallel_safe());
    //   PURE | READS_FS run concurrently; WRITES_FS | MUTATES_EXTERNAL serialise
    //   against each other (pass 1 §3.5, pi_agent_rust ToolEffects at src/tools.rs:150)
    obs::record!(graph.batch.width = batch.len());               // ← see §4.5
    futures::future::join_all(par.map(|n| dispatch(n, &sem))).await;
    for n in ser { dispatch(n, &sem).await; }
}
```

`map` node internal fan-out uses its own sub-semaphore and is **orthogonal** to this. Worth
stating, because it is a plausible misreading of pass 1 §3.1 that `map` is how fan-out
happens — it is not; orki's diamond had no `map` in it, and `map` would not have saved it.

### 4.4 `JoinPolicy::Any` replaces the `Race` node kind — a simplification of pass 2

Pass 2 §6.4 (`:1143`) proposes an 11th/12th node kind:
`Race { arms: Vec<NodeId>, win: WinCondition }`. **Drop it.** It requires the arms to be
structurally enumerated *inside* one node, which duplicates the edge list, breaks
content-addressing symmetry, and makes a raced arm un-routable (an arm cannot itself fan
out). Every racing scenario is expressible as: fan out normally, and set
`join = "any"` (or `"quorum:2"`) on the joiner.

```toml
[[node]] id="fanout"; kind="transform"; expr="$.input"
[[node]] id="local";  kind="llm";   model="tier:small"
[[node]] id="cloud";  kind="agent"; model="tier:large"
[[node]] id="use";    kind="transform"; join="any"; cancel_losers=true
[[edge]] from="fanout"; port="ok"; to="local"
[[edge]] from="fanout"; port="ok"; to="cloud"
[[edge]] from="local";  port="ok"; to="use"
[[edge]] from="cloud";  port="done"; to="use"
```

`cancel_losers` propagates through `Ctx::cancel`, which pass 1 already has. `WinCondition::
{FirstOk, Quorum(k)}` maps onto `JoinPolicy::{Any, Quorum}` exactly.

**Net node-kind count:** pass 2 §14 Phase 1 (`:2067`) says "12 node kinds incl. `net`/`race`".
Pass 3 removes `race` and adds `nano` → still 12, better factored: the 10 from pass 1 §3.1
plus `net` (pass 2 §4.5) plus `nano` (§5.2 below).

### 4.5 Detecting the bug at runtime, not just in a test

Emit `graph.batch.width` as a histogram from the scheduler tick. **If that histogram is
pinned at 1 on a graph known to contain a diamond, you have shipped orki's bug.** This is a
production detector for a bug class whose defining symptom is silence, and it costs one
`record!` per tick (pass 2 §2.5 budgets < 250 ns). Add alongside:

- `graph.join.unsatisfiable` — counter, should be 0 in steady state
- `graph.node.dead` — counter, dead-branch propagation is working
- `nano.escalation_rate`, `nano.confidence` — histogram, the Tier-0 health signal

### 4.6 New load-time validation rules

Extend `GraphSpec::validate()` (pass 1 §3.2, `001-...:559`), which currently has 5 rules:

- **V6 — statically unsatisfiable join.** A node with `JoinPolicy::All` whose inbound Data
  edges include ≥2 edges originating from *different ports of the same node* can never
  fire. Load error. (This is the routed-diamond deadlock, caught before it runs.)
- **V7 — nano escalation is mandatory.** A `nano` node must declare an `escalate` port and
  that port must have an outgoing edge. A nano node whose only outbound path leads to a
  `MUTATES_EXTERNAL` node without an intervening `gate` or `verify` is a load error (§1.2).
- **V8 — pipeline contract.** Every exit port of a pipeline must produce a value satisfying
  the pipeline's declared `outputs` schema. At pipeline boundaries this is a **failure**,
  not pass 1 rule 4's warning — a pipeline that does not honour its schema is not a
  pipeline.
- **V9 — `Any`/`Quorum` require ≥2 inbound Data edges.** A one-armed race is a typo.

### 4.7 The strengthened diamond test

Pass 2 §14 Phase 1 exit 3 (`:2067`) has four assertions. Make it six, plus two siblings:

**Test A — plain diamond** `entry → {a, b} → join(All)`:
(i) `a` and `b` both executed; (ii) their execution windows **overlapped in wall time**;
(iii) `join` executed **exactly once**; (iv) `join` started after both finished;
(v) `graph.batch.width` recorded a tick of width ≥ 2.

**Test B — routed diamond (dead propagation)** `entry → route → {a, b} → join(AllSettled)`:
route takes port `a`; assert `b` is marked `Dead`, `join` fires **exactly once** with only
`a`'s value, and the run **terminates** (does not hang). *This is the test pass 1 §3.5's
one-sentence readiness rule cannot pass.*

**Test C — unsatisfiable join is loud** same shape as B but `join(All)`: assert the run
ends `Terminal::Failure` with `join can never be satisfied` **within one tick**, and that
`pii graph lint` rejects it at load time (rule V6) before it ever runs.

**Test D — race cancels** `fanout → {slow, fast} → use(Any, cancel_losers)`: assert `use`
ran once with `fast`'s value, and `slow` received a cancel and recorded `Cancelled` in the
journal (not `Failure`).

Four tests, ~120 lines total. They catch the bug that shipped in a sibling project and
fabricated provenance. Pass 2 already calls this "the cheapest high-value test in the whole
plan"; pass 3 makes it a test *suite* because the routed case is the one that actually
bites in Pii (orki had no data-routed ports; Pii's whole design is data-routed, so Pii is
*more* exposed to this bug class, not less).

### 4.8 Blackboard — confirm pass 2's write-through decision, and name the pattern

orki measured 2,310 ops/s for a durable write-through blackboard at 32-way fan-out vs
50,279 ops/s in-memory — a 22× cost. Pass 2 §6.4 correctly concludes: **do not make the
blackboard write-through.** Pass 3 adds the precise shape, because "not write-through" is
under-specified:

> **Reads are memory-only. Writes go to the in-memory blackboard *and* append a
> `NodeFinished` event to the journal in the same redb write transaction that advances
> `RunHeader`** (pass 1 §3.4 already specifies this transaction). Snapshot every 32 events
> and unconditionally on `Suspend`.

That is durable-enough *by a different mechanism* than orki's per-key temp+rename: one
batched append per node completion instead of one fsync per blackboard `set`. orki's 22×
penalty came from the granularity (per-key), not from durability itself. Stating this
prevents someone "fixing" the blackboard later by adding per-key durability and rediscovering
the 22×.

---

## 5. Pipelines — named composable subgraphs with an enforced contract

### 5.1 The distinction, stated in one line each

- A **graph** is the general object: arbitrary topology, bounded cycles, data-routed ports,
  may suspend, may have no clean IO contract. Content-addressed by `GraphHash`.
- A **pipeline** is a **named, versioned graph that declares and *enforces* an IO
  contract**: `inputs: JsonSchema`, `outputs: JsonSchema`, one `entry`, and a set of exit
  ports each of which must satisfy `outputs`.

**A pipeline is sugar over `subgraph` + a schema + a stable name.** No new execution
mechanism, no new scheduler path. The brief's hypothesis is correct and pass 3 adopts it.

```rust
// pii-graph/src/pipeline.rs
pub struct PipelineSpec {
    pub name:    SmolStr,          // "coding.fix", "ops.housekeeping"
    pub version: u32,              // monotone; the sync/promotion unit (§7, Phase 5)
    pub inputs:  JsonSchema,
    pub outputs: JsonSchema,
    pub graph:   GraphHash,        // the body, content-addressed and immutable
    pub exits:   BTreeMap<PortName, ExitContract>,
    pub caps:    CapSet,           // the ceiling; intersected with the caller's, as always
    pub limits:  GraphLimits,
}
pub enum GraphRef { Pinned(GraphHash), Named(SmolStr), Pipeline(SmolStr, VersionReq) }
```

### 5.2 What having the *word* buys — four things, none cosmetic

1. **Typed composition at load time.** Pass 1 §3.2 rule 4 makes producer/consumer schema
   mismatch a *warning* because "schemas are often partial". At a pipeline boundary that
   excuse evaporates — the pipeline *promised* a schema. So the check becomes a **failure**
   (rule V8), and composition of pipelines is statically checkable in a way composition of
   raw graphs is not.
2. **It is the unit of versioning, sync, and shadow-running.** Pass 2 §7 (multi-site sync)
   and §14 Phase 5 exit 6 (promote a skill to a `GraphSpec` after ≥95% shadow agreement)
   both need a stable, versioned, contract-bearing object. `GraphHash` is too fine (any
   whitespace change is a new hash); a bare name is too coarse (no contract). Pipeline is
   the right granularity, and pass 2 currently has no object at that granularity.
3. **It is the nano router's label space.** This is the composability payoff:
   > **The Tier-0 router's classes *are* the pipeline registry.** `labels =
   > ["pipeline:coding.fix", "pipeline:ops.housekeeping", ...]`. Registering a pipeline
   > extends the router's decision space; `pii nano tune` refits against journal traffic.
   Routing stops being a hand-maintained rules table that rots and becomes a *derived
   artifact of the pipeline registry*. Adding a capability automatically makes it routable.
4. **Starters become pipelines.** Pass 1 §3.3's `starter/react`, `starter/verify-fix`,
   `starter/sense-decide-act`, `starter/distill` are currently *templates* — parameterised
   and inlined. A starter that has been given a schema **is** a pipeline. That is the
   promotion path, and it means `starter/*` and `pipeline:*` are two lifecycle stages of
   one object rather than two concepts.

### 5.3 orki's three patterns, mapped onto node kinds

| orki concept | Pii mechanism | Notes |
|---|---|---|
| **fan-out** | multiple outbound edges on one port; scheduler drains the full ready set (§4.3) | not `map` — `map` is fan-out *over an array* |
| **fusion / convergence** | `JoinPolicy::All` on the joiner + dead propagation | orki's actual bug lived here |
| **racing** | `JoinPolicy::Any { cancel_losers: true }` (§4.4) | replaces pass 2's proposed `Race` node kind |
| **partial gather** | `JoinPolicy::AllSettled` | the "collect whatever survived" case orki had no answer for |
| **role routing** (`llm\|agent` vs `shell\|http\|docker\|mcp`) | `NodeKind` — nothing to build | pass 2 §6.4 already notes this; confirms the taxonomy |
| **pipeline (orki JSON schema v2.0)** | `PipelineSpec` | orki's is a linear list; Pii's is a graph with a contract |

### 5.4 Storage

Two tables alongside pass 1 §3.4's `graph_body`/`graph_ref`:

```
pipeline_body : (name, version u32) -> Postcard<PipelineSpec>   # immutable
pipeline_ref  : name                -> u32                      # mutable: the active version
```

Running instances pin `(name, version)` at start, so a hot-reload of the active version
never mutates a run in flight — the same discipline pass 1 §3.4 applies to `graph_ref`.

---

## 6. Worked composite graph — all four tiers, one DAG, routed by data

`pipeline:coding.request` — nano routes, 9 B plans, a frontier agent implements, a native
tool verifies. Contains a real diamond (fan-out to plan + context-gather, converging on
implement), a nano `classify` in the inner loop, and a raced escalation. Authoring format
per pass 1 §3.6 plus the pass-3 fields (`join`, `mode`, `escalate_below`, `verify`).

```toml
name    = "coding.request"
version = 3
caps    = ["fs_read", "fs_write", "proc"]

[inputs]  type="object"; required=["utterance","repo"]
[outputs] type="object"; required=["summary","verified"]
[limits]  max_steps=60; max_wall="30m"; max_model_calls=40

# ── TIER 0 ── route the utterance. ~5 ms, $0. Escalates when unsure. ──────────
[[node]]
id="triage"; kind="nano"; mode="route"
on="$.input.utterance"
labels=["fix","feature","housekeeping"]
escalate_below=0.72                    # below this → the `escalate` port (V7: mandatory)
# ports: fix | feature | housekeeping | escalate

[[node]]                               # Tier 1 fallback when Tier 0 abstains
id="triage9b"; kind="llm"; model="tier:small"; output_schema="@route_enum"
# ports: ok | refusal | overflow

# ── DIAMOND ── plan and gather context concurrently, converge on implement ────
[[node]]
id="plan"; kind="llm"; model="tier:small"
prompt="@plan_prompt"; output_schema="@plan_schema"
[[node]]
id="ctx"; kind="tool"; tool="grep"      # Effects::READS_FS → runs parallel with `plan`
args={ pattern="$.nodes.plan.symbols", path="$.input.repo" }

[[node]]
id="implement"; kind="agent"; model="tier:large"
join="all"                              # ← fires only when BOTH plan and ctx are done
tools=["read","write","edit","bash"]; max_steps=25
verify={ delta=["$.input.repo/**/*.rs"] }   # pass 2 §6.2: fail if it changed nothing

# ── NATIVE VERIFY ── deterministic. No model. ────────────────────────────────
[[node]]
id="test"; kind="tool"; tool="bash"
args={ command="cargo test --workspace" }

# ── TIER 0 ── classify the failure. Runs on EVERY loop iteration → this is
#              where nano actually pays for itself vs a 9B call each time.
[[node]]
id="triage_fail"; kind="nano"; mode="classify"
on="$.nodes.test.stderr"
labels=["flake","compile_error","assertion","oom"]
escalate_below=0.65

[[node]] id="gate_exit"; kind="route"; on="$.nodes.test.exit_code"
rules=[ { eq=0, port="pass" }, { port="fail" } ]

# ── RACE ── a hard failure gets both tiers at once; first good answer wins ────
[[node]] id="fix_small"; kind="agent"; model="tier:small"; loop_budget=2
[[node]] id="fix_large"; kind="agent"; model="tier:large"; loop_budget=2
[[node]]
id="refix"; kind="transform"; expr="$.winner"
join="any"; cancel_losers=true          # ← §4.4: replaces the `race` node kind

[[node]] id="done"; kind="emit"; channel="cli"

# ── EDGES ────────────────────────────────────────────────────────────────────
[[edge]] from="triage";   port="escalate";     to="triage9b"
[[edge]] from="triage";   port="fix";          to="plan"
[[edge]] from="triage";   port="feature";      to="plan"
[[edge]] from="triage";   port="housekeeping"; to="hk"      # subgraph, elided
[[edge]] from="triage9b"; port="ok";           to="plan"

[[edge]] from="plan"; port="ok"; to="ctx"                  # plan feeds ctx its symbol list
[[edge]] from="plan"; port="ok"; to="implement"            # ┐ diamond
[[edge]] from="ctx";  port="ok"; to="implement"            # ┘ join="all"

[[edge]] from="implement";   port="done";           to="test"
[[edge]] from="test";        port="ok";             to="gate_exit"
[[edge]] from="gate_exit";   port="pass";           to="done"
[[edge]] from="gate_exit";   port="fail";           to="triage_fail"

[[edge]] from="triage_fail"; port="flake";          to="test"        # cycle, bounded
[[edge]] from="triage_fail"; port="assertion";      to="fix_small"
[[edge]] from="triage_fail"; port="compile_error";  to="fix_small"
[[edge]] from="triage_fail"; port="compile_error";  to="fix_large"   # ┐ race
[[edge]] from="triage_fail"; port="oom";            to="fix_large"
[[edge]] from="triage_fail"; port="escalate";       to="fix_large"
[[edge]] from="fix_small";   port="done";           to="refix"       # │ join="any"
[[edge]] from="fix_large";   port="done";           to="refix"       # ┘
[[edge]] from="refix";       port="ok";             to="test"        # cycle, bounded

[[edge]] from="test"; port="err"; to="done"; kind="order"  # always report, even on failure
```

**What this graph demonstrates, point by point:**

| Property | Where |
|---|---|
| Nano routes, does not orchestrate | `triage` is a node; the scheduler is untouched |
| Nano always has an escalation path (V7) | `triage.escalate → triage9b`, `triage_fail.escalate → fix_large` |
| Nano never terminal before a mutation (§1.2) | `implement` carries `verify: delta`; `refix` re-enters `test` |
| True fan-out | `plan.ok` → two edges; scheduler drains both |
| True convergence | `implement` has `join="all"` — the orki fix, in the hot path |
| Dead propagation | `triage` takes one port; the other branches' edges go `Dead`; nothing hangs |
| Race replaces the `Race` node kind | `refix` with `join="any"`, `cancel_losers` |
| Order edge survives a dead branch | `test.err → done` with `kind="order"` |
| Deterministic verification, no model | `test` is a `tool` node; `gate_exit` is a rule `route` |
| Cost shape | Tier 0 on every loop iteration (~5 ms, $0); Tier 1 once; Tier 2 only on hard failure |

The tiers are **routed by data**, not by config: which model runs is a consequence of
`Outcome::Next{port}` values, all of which are in the journal, all of which are replayable,
and all of which are labelled by the terminal `verify` result — which is exactly the corpus
`pii nano tune` consumes to make `triage` better next week.

---

## 7. Precise edits to `002-pii-architecture-plan-pass2.md`

| # | Location | Change |
|---|---|---|
| **E1** | **§9 title + §9 preamble, `:1453`** | Retitle **"§9. Tier 0 — the nano dispatcher (`pii-nano`), on the CPU and on the MCU"**. Open with the reframe: Tier 0 is a *role*, with two homes; the host CPU is primary, the ESP32 is the offline/private/distributed option. |
| **E2** | **§9.1, `:1455`** | Keep the ESP32 table verbatim — it is good research — but retitle it *"Home B: the ESP32-S3 port, measured"* and add a **Home A** row set: ~500 tok/s decode on a Pi 5, 300–700 on sub-$200 phones; ~40–100 ms end-to-end per dispatch on a Pi 5, 10–30 ms on x86. Add the verified constraint in bold: **no GGUF, no Ollama tag, no llama.cpp path; CQ2-bit weights baked into Cactus's own C++ engine, fetched as a prebuilt binary from HuggingFace; Python is the only first-class binding.** |
| **E3** | **§9.2, `:1492`** | Rewrite. Currently answers "what an MCU model is for". Split into **§9.2 What Tier 0 is for** (route / classify / extract; the grammar guarantee is the value) and **§9.2.1 What Tier 0 cannot do** (§1.2 above, including the bold rule that a nano node may never precede a `MUTATES_EXTERNAL` effect without a `gate` or `verify`, and the explicit statement that grammar-valid ≠ correct). Move the four MCU-specific bullets (offline, privacy, zero cost, physical tiering) under a new **§9.7 Home B**. |
| **E4** | **new §9.2.2** | The `NanoRouter` trait and the three-backend table from §3 above. State the decision: **B1 `Embed` is the default**, B2 `Worker(cactus)` optional for `extract`, **B3 candle is not built**. Include the four non-negotiables for B2 (hash-pinned artifact, separate build, worker protocol, degrade-to-Tier-1). |
| **E5** | **§9.3 / §9.4, `:1506` / `:1534`** | Keep `postcard-rpc`, `WireTransport`, and the tool-table downcast **unchanged** — but reframe the downcast as *"projecting Pii's `ToolSpec` set onto the Tier-0 label space"*, which is now needed for Home A too (the 256-token budget applies to both homes; the flashing step applies only to Home B). This is the sentence that rescues the pass-2 MCU work: the downcast, the ranking, and the truncation are shared. |
| **E6** | **§9.5, `:1566`** | Replace the 3-row tier table with the **2-home Tier-0 table** from §2. Keep `ModelTier::Nano`'s capability profile verbatim (it is correct). Add `NanoHome::{Host, Mcu(McuId)}`, defaulting to `Host`, with an explicit rule: **never silently fall back Host → Mcu on a latency path.** Keep the `mcu.dispatch` example as an illustration of Home B and note it uses a rule `route`, not a model — deliberately. |
| **E7** | **§9.6, `:1608`** | Retitle **"Tier 0 is the object of self-improvement, not the subject"**. Generalise `pii mcu tune` → **`pii nano tune`**, working from journal data on the host from Phase 3, with `pii mcu tune` as the Home-B specialisation. State plainly that a 45 M model cannot perform self-improvement, and that its role in that loop is (a) the thing being tuned and (b) the cheap trigger deciding *when* to wake a Tier-2 self-improvement graph. |
| **E8** | **§6.4, `:1143` — the `Race` pattern** | Replace the proposed `Race { arms, win }` node kind with **`JoinPolicy::Any { cancel_losers }`** and the reasoning in §4.4. Keep the racing *pattern* endorsement; drop the node kind. |
| **E9** | **§6.4 — the diamond warning** | Expand "write the diamond test" into the **four-test suite** of §4.7 (plain / routed-dead / unsatisfiable-loud / race-cancel). Add the observation that Pii is *more* exposed than orki, not less, because Pii's graph is data-routed and orki's was not. |
| **E10** | **new §10.6 (or a new §11 before "swiss-army knife")** | **"The harness, and the scheduler contract."** The five-part harness definition (§4.1) with the one-harness rule; the `EdgeState` / `JoinPolicy` / dead-propagation / readiness state machine (§4.3); the drain-all tick; `graph.batch.width` as a runtime detector (§4.5). This is the section pass 2 does not have and the one that would have prevented orki's bug by construction. |
| **E11** | **new §10.7** | **"Pipelines."** `PipelineSpec`, `GraphRef::Pipeline`, the four things the word buys (§5.2) — especially *the router's label space is the pipeline registry* — the orki-pattern mapping table (§5.3), and the two redb tables (§5.4). Cross-reference pass 1 §3.3's `starter/*` list as the promotion path. |
| **E12** | **§12.1 crate list, `:1914`** | Add **`pii-nano/`** — *"Tier-0 routing: `NanoRouter` backends, the journal-fitted classifier, `pii nano tune`. Isolated so classifier math and the optional nano-worker client never enter `pii-graph`'s dependency tree."* 18 → 19 crates. Acceptable alternative if crate count is a concern: a module in `pii-model`, but **not** if that pulls the reqwest quarantine into the Tier-0 path. |
| **E13** | **§14 Phase 1, `:2067`** | (a) node-kind list: `net`/**`nano`**, drop `race`, add `JoinPolicy` — still 12 kinds. (b) Exit 3 becomes the **four-test diamond suite** (§4.7). (c) New exit 9: `pii graph lint` rejects a statically-unsatisfiable join (rule V6). (d) New exit 10: a `nano` node with backend `Static` (a fixed test stub implementing `NanoRouter`) routes correctly and takes `escalate` below threshold — **proves the node kind and the escalation contract with zero model and zero hardware.** |
| **E14** | **§14 Phase 3, `:2112`** | Add: **`pii-nano` backend B1 (`Embed`) lands here**, because it needs the `Embedder` that Phase 3 already brings for memory. New exit: after ≥200 journaled routing decisions, `pii nano tune` raises direct-dispatch rate at constant verified-error rate — *the pass-2 §9.6 exit criterion, moved from Phase 6 to Phase 3 and de-coupled from hardware.* |
| **E15** | **§14 Phase 6, `:2170`** | Narrow to Home B only: `postcard-rpc` transports, tool-table flashing, offline degradation, `pii mcu tune`. Exits 5 and 6 stay. Note explicitly that Phase 6 now validates a **transport and a deployment**, not a tier — the tier was proven in Phase 3. |
| **E16** | **§15 decisions, `:2201`** | Amend **decision 6** (buy an ESP32): the answer is still (a)-but-later, and now **nothing in the Tier-0 story blocks on it at all** — which is the point. Note the **new coupling to decision 8**: with B1 as the default nano backend, `OllamaEmbedder` means Tier 0 is not offline in the default config. Add decisions 9–12 (§8 below). |
| **E17** | **§16.3 risks, `:2248`** | Add the three new risks in §8 below. |
| **E18** | **§0 corrections table, `:44`** | Add correction 8: *"Pass 2 §9 framed Needle 2 as an MCU story. It is a **role** (Tier-0 dispatcher) with two homes; the primary one is the daemon's own CPU (~500 tok/s on a Pi 5), where it is testable from Phase 3 with no hardware. The MCU is the offline/private/distributed deployment of the same role."* Add correction 9: *"Pass 1 §3.5's readiness rule — 'every inbound edge on a taken port has fired' — is not implementable for a data-routed graph and is exactly the ambiguity orki fell into. See the new §10.6."* |

---

## 8. New decisions and risks

### 8.1 New open decisions (append to §15)

| # | Decision | Options | Recommendation |
|---|---|---|---|
| **9** | **Default Tier-0 backend** | (a) `Embed` — existing `Embedder` + journal-fitted classifier; (b) `Worker(cactus)` — Needle 2 in a T4 worker; (c) `Candle` — 0.5 B + llguidance in-process | **(a) default, (b) optional for `mode="extract"`, (c) never.** (a) adds zero dependencies, is deterministic and calibrated, and learns from the journal. (c) is Tier 1 wearing a Tier-0 label; Ollama already serves Tier 1. |
| **10** | **`Race` node kind vs `JoinPolicy::Any`** | (a) pass 2's `Race { arms, win }`; (b) `JoinPolicy::Any { cancel_losers }` on the joiner | **(b).** One mechanism, no duplicated edge list, raced arms can themselves fan out, and `Quorum` falls out for free. Removes a node kind. |
| **11** | **Where Tier 0 lands in the roadmap** | (a) Phase 6 with the MCU, per pass 2; (b) split: trait + `nano` kind + `Static` stub in Phase 1, `Embed` backend + `pii nano tune` in Phase 3, MCU home in Phase 6 | **(b).** Gets Tier 0 validated ~14 weeks earlier against real traffic, at $0 hardware, and turns Phase 6 into a transport exercise rather than a tier bring-up. |
| **12** | **Pipeline as a distinct persisted object** | (a) just a named graph with a schema field; (b) `PipelineSpec` with its own `pipeline_body`/`pipeline_ref` tables | **(b).** The contract must be enforceable at load time (rule V8) and versionable independently of `GraphHash`; (a) cannot do either without becoming (b) badly. |

### 8.2 New risks (append to §16.3)

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| **Nano over-trust.** "Grammar-constrained, no validation needed" is read as a correctness guarantee; a nano node ends up gating a destructive action. | **Medium-High** | **High** | Validation rule V7 (mandatory `escalate` port; no `MUTATES_EXTERNAL` without an intervening `gate`/`verify`). `nano.escalation_rate` + `nano.confidence` in `/metrics`. Never let a nano decision be terminal. |
| **Cactus engine lock.** Prebuilt binary fetched from HuggingFace, no GGUF, Python-only bindings, musl/ARM64 support unstated, engine redistribution terms unread. | **High** *if B2 is adopted* | Medium | `NanoRouter` trait; B1 is the default so Needle is never load-bearing; hash-pin and vendor the artifact; `pii-nano` ships as a separate non-musl build; read the licence before Phase 3. |
| **Router drift.** A fitted classifier silently degrades as the pipeline registry grows; escalation rate creeps up and nobody notices because everything still *works*, just more expensively. | Medium | Medium | `nano.escalation_rate` alert; `pii nano tune --dry-run` in the weekly housekeeping graph; shadow-mode comparison against the Tier-1 router on a sampled 1% of traffic. |
| **Hung runs from dead-branch bugs.** Dead propagation is subtle; a bug means a run that neither completes nor fails. | Medium | **High** | `UnsatisfiableJoin` → `Terminal::Failure`, never a hang; load-time rule V6; the four-test diamond suite; a run-level "no ready nodes, nothing in flight, not terminal" detector that fails loudly. |

### 8.3 One risk this pass *downgrades*

Pass 2 §16.1 risk 10 — *"MCU integration cost: a C engine, ESP-IDF, a 3-minute flash cycle,
and a board that must be bought"* — drops from Medium/Low to **Low/Low**. Not because the
cost changed, but because the tier no longer depends on it. Under pass 2, skipping Phase 6
meant Tier 0 never existed. Under pass 3, skipping Phase 6 means Tier 0 has one home
instead of two. That is the test that §9 is now correctly layered, and it is the same test
pass 2 §16.1 proposed for itself and failed.

---

## 9. Summary of what changes

1. **Tier 0 is a role, not a board.** Primary home = the daemon's CPU (~5–100 ms, $0);
   the ESP32 is the offline/private/distributed deployment of the same decision contract.
2. **Nano cannot self-improve and cannot run DAGs.** It is a node. Its real roles are
   route / classify / extract, plus being the *object* of a journal-driven tuning loop
   that needs no LLM at all.
3. **Needle 2 cannot be embedded in a static-musl Rust binary** — no GGUF, no llama.cpp,
   Cactus's own C++ engine only. So the architecture is written against a `NanoRouter`
   trait, defaults to an embedding classifier with zero new dependencies, and puts Needle
   in a T4 worker if it is wanted at all.
4. **The harness is one thing, defined once**, and the scheduler contract (edge liveness,
   dead propagation, join policy, drain-all ticks) is specified precisely enough that
   orki's diamond bug is unrepresentable — and detectable at runtime via
   `graph.batch.width`.
5. **`JoinPolicy::Any` replaces the `Race` node kind**; `AllSettled` and `Quorum` come free.
6. **Pipelines = named, versioned subgraphs with an enforced IO contract**, and the Tier-0
   router's label space *is* the pipeline registry — which is what makes routing a derived
   artifact instead of a rotting rules table.

## Verification

This pass writes no code. Its claims verify as follows:

- Needle 2 throughput, the absence of a GGUF/llama.cpp path, and the Cactus-engine
  distribution model were checked against the vendor's model card, repo README and
  independent write-ups this pass (see Sources in the accompanying message).
- The scheduler state machine is verified by the **four-test diamond suite** (§4.7), all of
  which are behavioural and all of which fail if the feature is removed — the standard pass
  2 §16.1 risk 11 demands.
- The Tier-0 claim is verified by Phase-1 exit 10 (a `Static` `NanoRouter` stub proves the
  node kind and escalation contract with no model) and Phase-3's tune criterion (≥200
  journaled decisions; direct-dispatch rate rises at constant verified-error rate).
- The "no wasted MCU research" claim is verified by E5: the tool-table downcast, ranking
  and truncation are shared between both homes and are exercised in Phase 3.

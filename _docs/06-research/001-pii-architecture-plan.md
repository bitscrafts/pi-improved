# Pii (Pi Improved) — Architecture Plan

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

# In-process LLM support via `llama-cpp-2` (embedding Granite 4.2 inside Pii)

> **Status update 2026-10-09:** IMPLEMENTED, no longer a draft. The in-process `llama-cpp-2` provider landed in `a9dce5d` (spec 35) with config + real-model acceptance in `2b30c00`, and in-process tool calls in `a37ac8a`/`bb131bb` (spec 36). The "nothing implemented" status above is historical.

**Date**: 2026-09-10 · **Author**: research agent (claude-sonnet-5) · **Status**: DRAFT — feasibility study, nothing implemented

## Context

Today Pii's only live model backend is [`OllamaProvider`](/projects/pi-improved/crates/pii-model/src/ollama.rs)
— an HTTP client (`Arc<dyn HttpTransport>` → `POST {OLLAMA_HOST}/api/chat`)
implementing the sync [`ModelProvider`](/projects/pi-improved/crates/pii-model/src/lib.rs)
trait. `014-local-model-spec-implementation.md` already established that Granite 4.2's
thinking-mode toggle (`enable_thinking=False`) is **not controllable through
Ollama at all** — Ollama replaces the model's own Jinja chat template, so the
probe had to fall back to `llama-server` (the llama.cpp reference HTTP
server) with `--reasoning off` to get clean, non-empty completions. That
finding is the entire motivation for this doc: if Pii wants Granite's
non-thinking mode without depending on an external `llama-server` process,
the only remaining path is linking llama.cpp directly into the Pii binary.

This doc asks: is `llama-cpp-2` (Rust bindings to llama.cpp) the right way
to do that, and if so, what does the integration look like.

`llama-cpp-2` is **not currently a workspace dependency** (`grep -rn
"llama-cpp" Cargo.toml crates/` returns nothing).

## 1. The `llama-cpp-2` crate

Queried live from crates.io (2026-09-10):

- **Latest**: `0.1.156` (published 2026-09-02, i.e. the same week as the
  granite probe), 134 published versions, ~632k recent downloads. Actively
  maintained by `utilityai` (github.com/utilityai/llama-cpp-rs), who run it
  in production ("part of the project powering all the LLMs at utilityai").
- **License**: MIT OR Apache-2.0 — compatible with Pii's own
  `MIT OR Apache-2.0` (`Cargo.toml` `workspace.package.license`).
- **Companion crate**: `llama-cpp-sys-2` (raw bindgen bindings, pinned to
  the same version) — this is where the actual llama.cpp C++ source is
  vendored and compiled.
- **Safety posture (README, verbatim)**: *"This crate is not safe. There is
  absolutely ways to misuse the llama.cpp API provided to create UB... Do
  not use this code for tasks where UB is not acceptable."* This does **not**
  block adoption under Pii's `unsafe_code = "forbid"` workspace lint — that
  lint is `#![forbid(unsafe_code)]` scoped to the crate it's declared in
  (every `crates/pii-*` crate), and only bans `unsafe` written *inside
  Pii's own source*. A dependency using `unsafe` internally (as any FFI
  wrapper must) does not trip it, the same reasoning already applied to
  other native-code deps in `016-ml-applications.md` (§ hard constraints).
  The risk this shifts onto Pii is different from a lint violation: an
  attacker-controlled or malformed GGUF/prompt could in principle trigger
  memory-unsafety in llama.cpp's C++ core, not just a `PiiError`. Framed as
  a supply-chain/blast-radius risk, not a lint gap.

### Build requirements

From `llama-cpp-sys-2`'s `Cargo.toml` build-dependencies: `bindgen` (needs
**clang** installed), `cc`, **`cmake`** (llama.cpp's own build system, vendored
under `llama.cpp/cmake`, `llama.cpp/ggml/cmake`, `llama.cpp/common/cmake`),
`find_cuda_helper`, `glob`, `walkdir`. Confirmed on this box: neither `cmake`
nor `clang` is currently installed (`which cmake clang` → nothing) — a real
adoption needs both added to the dev/CI image, plus a C++ toolchain. This is
a materially heavier build than any current Pii dependency (all pure-Rust or
thin FFI today) and a first `cargo build` compiles the vendored llama.cpp
C++ sources from scratch (multi-minute, `-Ctarget-cpu=native`-sensitive for
performance).

### Feature flags

```
default = ["openmp", "android-shared-stdcxx", "common"]
cuda, cuda-no-vmm, metal, vulkan, opencl, rocm, mkl   # GPU backends
openmp, static-openmp                                  # CPU threading
dynamic-link, dynamic-backends, system-ggml[-static]   # linking strategy
common          # JSON-schema→grammar helper (pulls in ~14MB libcommon.a)
sampler         # exposes low-level sampler internals
mtmd            # multimodal (vision/audio) support
llguidance      # structured generation via llguidance/toktrie
```

**CPU-only is fully supported and is the default** — none of `cuda`,
`metal`, `vulkan`, `rocm` are in the default feature set, and the probe doc
already confirms CPU-only Granite 4.2 works (14–28 tok/s on 32 cores, no
GPU, via plain `llama-server`, which is built from the same llama.cpp core
this crate binds). For Pii, depending on `llama-cpp-2` with `default-features
= false, features = ["common"]` (or dropping `common` too if the JSON-grammar
helper isn't needed) is the right shape — no GPU feature, no OpenMP if Pii
wants to control its own thread budget instead.

### API surface (verified against `examples/simple/src/main.rs` and
`llama-cpp-2/src/model.rs`, both fetched live from the `main` branch)

Entirely synchronous, blocking, no `async` anywhere in the crate:

```rust
let backend = LlamaBackend::init()?;                       // once per process
let model = LlamaModel::load_from_file(&backend, path, &LlamaModelParams::default())?;
let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(4096));
let mut ctx = model.new_context(&backend, ctx_params)?;

let tokens = model.str_to_token(&prompt, AddBos::Always)?;
let mut batch = LlamaBatch::new(512, 1);
// ... batch.add(token, pos, &[seq_id], is_last_logit) per token ...
ctx.decode(&mut batch)?;                                    // prompt prefill

let mut sampler = LlamaSampler::chain_simple([
    LlamaSampler::dist(seed), LlamaSampler::greedy(),        // or top_k/top_p/temp/min_p
]);
loop {
    let token = sampler.sample(&ctx, batch.n_tokens() - 1);
    sampler.accept(token);
    if model.is_eog_token(token) { break; }
    let piece = model.token_to_piece(token, &mut decoder, true, None)?;  // ← streaming delta
    batch.clear();
    batch.add(token, pos, &[0], true)?;
    ctx.decode(&mut batch)?;
}
```

This maps almost exactly onto `ModelProvider::chat_streaming`'s
`on_delta: &mut dyn FnMut(&str)` contract: each `token_to_piece` call in the
sample-loop *is* one delta. No adapter needed for the streaming shape — the
crate's native loop already produces text piece-by-piece.

**Thread-safety** (checked directly in the crate source):
`unsafe impl Send for LlamaModel {}` and `unsafe impl Sync for LlamaModel {}`
(`model.rs:128,130`) — a loaded model can be shared read-only across threads
(`Arc<LlamaModel>`). `LlamaContext<'_>` has **no** `Send`/`Sync` impl — it
owns the KV cache and is mutated by every `decode` call, so it is
single-threaded by construction (matches llama.cpp's real C API contract:
one context = one sequential decode stream). This is the load-bearing fact
for the concurrency section below.

## 2. Integration into `pii-model`

### Fitting the sync constraint

`pii-model/src/lib.rs`'s own doc comment already states the constraint this
research was asked to check: *"`ModelProvider` is sync on purpose:
`GraphNode::run` is sync and runs execute on a blocking thread."* `llama-cpp-2`
is a **perfect match** for this, better than `OllamaProvider` in one sense —
`OllamaProvider` is sync only because it blocks inside `HttpTransport::request`
on what's really a network I/O wait; `llama-cpp-2`'s `ctx.decode()` is sync
because there is no I/O at all, it's a CPU-bound function call. No channel,
background thread, or `tokio` runtime is needed to make the *call shape* fit
— `fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError>`
can call straight into `ctx.decode()` in a loop and return.

A background thread *does* become useful for a different reason: serializing
concurrent access to the one non-`Send` `LlamaContext` (see §4 concurrency).
The pattern:

```rust
pub struct LlamaCppProvider {
    model: Arc<LlamaModel>,          // Send+Sync, shared freely
    backend: Arc<LlamaBackend>,      // process-wide singleton
    ctx_slot: Mutex<LlamaContext<'static>>,  // or: a channel to an owning thread
    n_ctx: u32,
}
```

Two implementation choices for `ctx_slot`:
1. **`Mutex<LlamaContext>`** — simplest. `GraphNode::run` already executes on
   a blocking thread per pii-graph's worker model, so a `chat()` call that
   blocks on the mutex while another turn is decoding is exactly the "one
   model slot" semantics `014-local-model-spec-implementation.md` already assumes
   for tier 1 ("occupies the single Ollama slot"). No new concurrency model,
   just a different lock type.
2. **Dedicated owning thread + `mpsc` channel** — `chat()` sends
   `(ChatRequest, Sender<ChatResponse>)` to a thread that owns the
   `LlamaContext` outright and never crosses it between threads. Strictly
   more code for the same externally-observed behavior (serialized turns);
   only worth it if Pii later wants e.g. mid-generation cancellation via a
   second channel, which `Mutex` can't express as cleanly.

Recommendation: start with `Mutex<LlamaContext>` (option 1) — it's less
code, `unsafe`-free at the call site either way, and gets the same
one-slot-at-a-time behavior the granite probe already designed around.

### Wiring into `provider_from_env`

`pii-model/src/lib.rs`'s `provider_from_spec`/`resolve_with_transport` and
`ollama.rs`'s `provider_from_env` establish the pattern: a `<kind>:<rest>`
spec string, a `pii_core::env_var`-sourced config, dispatch by `kind`. Adding
`llama:<model-path>` (or `llamacpp:<model-path>` to avoid ambiguity with
"llama" the model family vs. "llama.cpp" the backend) slots into the same
`match kind` arms:

```rust
// pii-model/src/llamacpp.rs (new)
pub fn provider_from_env(spec: &str) -> Result<Arc<dyn ModelProvider>, PiiError> {
    let Some((kind, rest)) = spec.split_once(':') else { ... };
    match kind {
        "llamacpp" => {
            let n_ctx = pii_core::env_var("PII_LLAMACPP_NUM_CTX")
                .and_then(|s| s.parse().ok())
                .unwrap_or(4096);
            let n_threads = pii_core::env_var("PII_LLAMACPP_THREADS")
                .and_then(|s| s.parse().ok()); // default: all cores
            Ok(Arc::new(LlamaCppProvider::load(Path::new(rest), n_ctx, n_threads)?))
        }
        ...
    }
}
```

Unlike `ollama:`, this needs **no `HttpTransport`** (no network at all) —
which actually simplifies `resolve_with_transport`'s call site rather than
complicating it; `llamacpp:` can be handled by `provider_from_spec` directly
(alongside `scripted:`), since it needs no transport injection, only a
filesystem path already covered by `fs_read` capability checks.

### Capability fit (`pii-core::cap`)

The existing `CapKind` bits are `FS_READ/FS_WRITE/NET/PROC/...` (`pii-abi`,
re-exported by `pii-core::cap`). An in-process model load is a pure
`fs_read` operation against the GGUF path (checked once, at provider
construction, against the caller's `PathScope`) — **no `NET` capability is
needed at all**, which is a genuine capability-model win over `ollama:`
(which needs `NET` scoped to `OLLAMA_HOST`) and a strict narrowing of the
attack surface for any node whose `CapSet` should never reach the network.
This is worth surfacing to whoever owns the capability policy: a graph node
that should get "an LLM but never the network" is inexpressible with
`ollama:` today and directly expressible with `llamacpp:`.

## 3. Granite 4.2 specifics

### GGUF availability

`014-local-model-spec-implementation.md` already pins a verified artifact:
`granite-4.2-3b-Q4_K_M.gguf` (2.24GB) from `ibm-granite/granite-4.2-3b-GGUF`
on Hugging Face, kept at `/projects/_tmp/`. The 8B variant (opus's
recommended tier-1 primary, 5.3GB) is presumably available from the sibling
`ibm-granite/granite-4.2-8b-GGUF` repo (not independently re-verified in
this doc — treat as "very likely, same publisher pattern" not "confirmed").

### The thinking-mode problem — this is the crux of the whole doc

The granite probe found the non-thinking gate requires `--reasoning off` on
`llama-server`, and explicitly noted this flag is `llama-server`'s own flag,
**not** a wire-level `enable_thinking` field respected by the model itself.
Digging into *why*, by reading llama.cpp's chat-template dispatch
(`src/llama-chat.cpp`, fetched live from `ggml-org/llama.cpp@master`):

- `llama-cpp-2::LlamaModel::apply_chat_template` binds the **raw C API**
  `llama_chat_apply_template`, which is llama.cpp's legacy, hardcoded
  template renderer — a big `if/else` chain matched by template name
  (`llama-chat.cpp:60-65` lists `granite`/`granite-4.0`/`granite-4.1` as
  recognized names). This is a **different code path** from the full Jinja2
  engine (`minja`, in `common/chat.cpp`) that `llama-server` uses to render
  the model's *actual* embedded chat template with `enable_thinking`/
  `--reasoning` support. `llama-cpp-2` does not bind `common/chat.cpp` at
  all — only the legacy renderer.
- Reading the Granite branches of the legacy renderer directly
  (`llama-chat.cpp:644-666`, Granite 4.0/4.1): the emitted format is
  `<|start_of_role|>{role}<|end_of_role|>{content}<|end_of_text|>` per
  message, with **no `<think>` scaffolding, no reasoning preamble, and no
  `enable_thinking` conditional at all** — the legacy path simply doesn't
  know thinking mode exists. This is a genuinely useful finding: it means
  the legacy renderer's output, by omission rather than by explicit
  `enable_thinking=False` support, structurally matches the *shape* of
  Granite's non-thinking template output (no `<think>...</think>` block).
  **This is not independently verified against the real Jinja template's
  `enable_thinking=False` branch byte-for-byte** — before relying on it,
  diff `apply_chat_template`'s output for a 2-turn conversation against
  `llama-server --reasoning off`'s `/apply-template` endpoint (or manually
  against the Jinja source in the GGUF's `tokenizer.chat_template` metadata
  field) to confirm they match. Flagging as a verification gap, not a
  settled fact.
- **Safer alternative that sidesteps the whole question**: don't call
  `apply_chat_template` at all. Hand-build the prompt string directly using
  the documented Granite 4.x wire format above (`<|start_of_role|>...`) —
  this is what `OllamaProvider::message_value` already does conceptually
  (constructs the wire shape explicitly rather than trusting a template
  engine to infer it), and removes any dependency on llama.cpp's template
  name→branch matching being correct for whatever GGUF metadata string this
  specific quant carries. Given the granite probe already had one
  "same empty-content trap as glm-5.3" surprise from thinking-mode
  ambiguity, the hand-built-prompt path is the recommended one — it makes
  the non-thinking behavior an explicit, testable Rust string-formatting
  function instead of an implicit property of which llama.cpp code path
  fires.

### Generation params

Per the probe's IBM-doc citation: `temperature=1.0, top_p=0.95` across all
Granite 4.2 tasks/backends — map directly onto `LlamaSampler::chain_simple`
(`temp(1.0)`, `top_p(0.95, 1)`, `dist(seed)`), replacing the `simple.rs`
example's greedy-only chain.

## 4. Tradeoffs: in-process llama.cpp vs external server vs cloud

| Axis | In-process `llama-cpp-2` | External `llama-server`/Ollama | Cloud (Anthropic/etc via HTTP) |
|---|---|---|---|
| **Process model** | One `LlamaModel` load inside the Pii binary; model weights live in Pii's own address space | Separate process, Pii talks HTTP (`OllamaProvider` today) | Separate service, HTTP + auth |
| **Memory footprint** | Model weights (2.2GB Q4 for granite-4.2-3b, ~5GB for 8B) + KV cache counted **against Pii's own RSS** — directly visible to whatever OOM/budget accounting Pii does on itself | Model weights live in the external server's RSS, invisible to Pii's own budget/`Ctx` accounting | Zero local memory |
| **Load time** | Model load (mmap + first-touch) happens on first `LlamaCppProvider::load()` call — multi-second for an 8B GGUF, paid once per Pii process lifetime (or per daemon restart) | Same load cost, but paid by the external server, often already warm before Pii starts | None (network round-trip only) |
| **Concurrency** | One `LlamaContext` = one sequential decode stream (not `Send`/`Sync`); needs a `Mutex` or owning thread — matches the granite probe's existing "single tier-1 slot" assumption, doesn't regress it | Ollama/llama-server already serialize (or queue) requests to one loaded model themselves — same externally-observed behavior | Provider-side concurrency, no local serialization needed |
| **Network capability** | **None needed** — pure `fs_read` on the GGUF path, no `NET` `CapKind` required | Needs `NET` scoped to `OLLAMA_HOST`/`llama-server` host:port | Needs `NET` scoped to the provider's API host, plus a `Secret` for the API key |
| **Build complexity** | New: `cmake` + `clang` + C++ toolchain in every dev/CI image; multi-minute cold build compiling vendored llama.cpp C++ | None — Ollama/llama-server are installed once, outside Pii's build | None |
| **Crash blast radius** | An llama.cpp segfault (the crate's own README disclaims UB is possible) **takes down the whole Pii process** — no process boundary | A crashed `llama-server` is an `Unavailable`/`Timeout` `PiiError` from `OllamaProvider::send` — Pii's `panic = "abort"` release profile note about "a panic inside a thread::scope worker must not unwind into the scheduler" is exactly the kind of blast-radius concern this doc should flag for a genuine C++ FFI crash (not a Rust panic — `catch_unwind` doesn't help against a real segfault) | No local crash risk at all |
| **Distribution** | Every Pii binary/release artifact must either vendor-build llama.cpp per platform or ship a separate native lib per OS/arch — a real packaging cost the `[profile.release]` `strip = true`/`lto = "fat"` config doesn't currently anticipate | No packaging cost to Pii — the server is the user's own concern | No packaging cost |
| **Thinking-mode control** | **Full control** — hand-built prompt, no vendor toggle needed | `--reasoning off` (llama-server) works; Ollama cannot do this at all (granite probe finding) | Vendor-specific (Anthropic uses extended-thinking params, not applicable to Granite) |
| **Offline / air-gapped** | Fully local, no server process to manage | Fully local, but needs the server process running and discoverable | Requires network egress |

## 5. Recommendation

**Yes, in-process `llama-cpp-2` is worth adopting, but as an additive third
provider alongside `ollama:`, not a replacement — and only once the crash
blast-radius risk is explicitly accepted.**

Reasoning:
- The one concrete problem this doc was asked to solve — Granite's
  thinking-mode toggle being uncontrollable through Ollama — is real and
  already blocking tier-1 adoption per the probe doc. `llama-cpp-2` solves
  it more robustly than shelling out to `llama-server` as a managed
  subprocess would, because Pii already needs `PROC` capability plumbing
  and process-lifecycle management to spawn+supervise an external
  `llama-server`, whereas `llama-cpp-2` needs neither `PROC` nor `NET` —
  strictly fewer capabilities for a strictly more controllable outcome
  (hand-built prompts, no vendor template-detection guesswork).
- It fits `ModelProvider`'s sync contract natively — better than
  `OllamaProvider` does, since there's no I/O wait to justify being sync,
  it's actually CPU-bound work on the same blocking thread `GraphNode::run`
  already uses. Zero architectural friction on the trait side.
- The concurrency model (`Mutex<LlamaContext>`, one slot at a time) doesn't
  regress anything — it's the same "one tier-1 slot" assumption the granite
  probe already designed pass-3's tier system around.
- The real costs are operational, not architectural: `cmake`+`clang` in
  every build image, multi-minute cold builds, per-platform native-lib
  packaging, and — the one that should gate the decision — **an llama.cpp
  segfault kills the whole Pii process**, unlike an `ollama:`/`llama-server`
  crash which surfaces as a recoverable `PiiError::Unavailable`. Given
  Pii's `panic = "abort"` release profile is already tuned around not
  letting a worker-thread panic corrupt shared state silently, a real C++
  segfault from a native FFI crate is a materially different, more severe
  failure mode that the current architecture has never had to reason
  about. This should be an explicit, named risk acceptance before merging,
  not something the release-profile comment quietly covers by analogy.

### Concrete integration sketch (order of operations, not a spec)

1. Add `llama-cpp-2` to `Cargo.toml` `[workspace.dependencies]`,
   `default-features = false, features = ["common"]` (drop `openmp`/GPU
   features; `common` only if JSON-schema-grammar constrained generation is
   wanted later).
2. New module `crates/pii-model/src/llamacpp.rs`: `LlamaCppProvider` struct
   (`Arc<LlamaModel>` + `Arc<LlamaBackend>` + `Mutex<LlamaContext<'static>>`),
   implementing `ModelProvider::chat` and `chat_streaming` via the
   tokenize→batch→decode→sample loop from `examples/simple/src/main.rs`,
   feeding each `token_to_piece` through `on_delta`.
3. A dedicated `granite::build_prompt(&[ChatMessage]) -> String` helper in
   the same module (or a `pii-model/src/granite.rs`) implementing the
   `<|start_of_role|>{role}<|end_of_role|>{content}<|end_of_text|>` wire
   format by hand — **not** `apply_chat_template` — so non-thinking mode is
   an explicit, unit-testable function, per §3's verification gap.
4. `provider_from_spec` gains a `"llamacpp"` arm (no transport needed,
   unlike `"ollama"`), spec shape `llamacpp:<path-to-gguf>`.
5. Before any adoption gate is declared green, replay the granite probe's
   own acceptance check (`content>0 AND reasoning==0`, wire field recorded)
   against the in-process provider, not just against `llama-server` — the
   hand-built-prompt approach in step 3 needs the same empirical proof the
   external-server path already got.
6. Add `cmake`+`clang` to whatever dev-container/CI image definition Pii
   uses (not located/verified in this pass — a follow-up should confirm
   where that image is defined before this is buildable in CI).

### What this doc did *not* verify (explicit gaps for a follow-up)

- Byte-for-byte diff of `apply_chat_template`'s Granite-4.1 output vs. the
  real Jinja template's `enable_thinking=False` rendering (§3).
- Whether `ibm-granite/granite-4.2-8b-GGUF` exists on HF with the same
  naming pattern as the already-verified 3B repo.
- Actual measured build time / binary size delta on this box (cmake/clang
  aren't installed here, so a real build wasn't attempted — instructed not
  to run `cargo build` for this research pass regardless).
- Where Pii's CI/dev-container image is defined, to scope the `cmake`+
  `clang` toolchain addition.

# Tiny chat models (Needle and similars) — real usage inside Pii

**Date**: 2026-09-10 · **Author**: sonnet (research) · **Status**: RESEARCH — no decision, no code touched

## 0. Scope and method

This revisits the "tiny models" question raised and partly settled by
[`004-granite-nano-tier-probe.md`](004-granite-nano-tier-probe.md) and
[`003-pii-architecture-plan-pass3.md`](003-pii-architecture-plan-pass3.md) §1/§3. That prior
work already rejected a tiny **chat** model at tier 0 in favor of a deterministic embedder
(B1) and picked granite-4.2-3b for tier 1. This doc asks the narrower question: is there
*any* real spot inside Pii — not necessarily tier 0 — where a 0.5B–3B **chat** model earns
its keep over (a) the existing deterministic/embedder path or (b) just calling tier-1
granite?

Method: read the prior decision docs and the actual current code (not just the plan) to see
what's implemented vs. still paper, then reason about each candidate use case against that
ground truth. External model facts (§2) are drawn from training knowledge, not re-verified
against vendor pages in this session (no live web access was available) — treat the specs
table as a starting point to re-verify before any adoption, the same way 004 insisted on a
live CPU probe before trusting a spec sheet.

`_docs/06-research/016-ml-applications.md` does not exist in this repo — there is no prior
ML-applications doc to cross-reference.

## 1. What Pii actually has today (ground truth, not plan)

This matters because the prior docs (pass 3, 004) describe a **target** architecture with a
`nano` node kind, a `NanoRouter` trait, and B1/B2/B3 backends. None of that exists in code
yet:

- `crates/pii-graph/src/node.rs` registers exactly five built-in node kinds:
  `transform`, `wait`, `emit`, `route`, `timer`. `pii-server` adds `agent` (`agent.rs`) and
  `tool` (`tool.rs`). **There is no `nano` kind, no `NanoRouter` trait, no `pii-nano` crate.**
  `grep -rn "NanoRouter\|kind.*Nano"` across `crates/` returns nothing.
- The single model in the system today is whatever `ModelProvider` the daemon config
  resolves — in the shipped default (`crates/pii-server/src/daemon/config.rs:388`),
  `model = "ollama:granite4.2:3b"`. Every `agent` node run goes through this one provider
  (`crates/pii-server/src/agent.rs:26,42,263`). There is no tier split in the runtime: one
  model does routing, planning, tool calls, and repair alike.
- The "B1 embedder" pass 3 speced (`Arc<dyn Embedder>` for memory, doubling as the nano
  router's feature extractor) is `TrigramEmbedder` in
  `crates/pii-memory/src/embed.rs:34` — a **pure-Rust, non-learned, deterministic** 256-dim
  character-trigram hash embedder (FNV-1a into buckets, `sqrt(tf)`, L2-normalized). It is not
  a model at all, let alone a tiny model. `OllamaEmbedder` — the one alternate embedder
  backend pass 3 discussed and flagged as a coupling risk ("Tier 0 works offline is false in
  the default config if B1 needs Ollama") — **was deleted** in the phase4.7 cleanup
  (`72d1aa2 fix(phase4.7): residual defects B1/B4/B5 - delete OllamaEmbedder, ...`). So that
  coupling risk pass 3 flagged is now moot: memory embedding is 100% offline, 100%
  non-ML, by construction. This is worth stating plainly because it changes the tiny-model
  question — there is currently *no* embedding-quality gap a tiny model could close either;
  `TrigramEmbedder` was a deliberate choice, not a placeholder.
- There is no tool-output summarization node. `agent.rs` has a hard `max_tokens` cap
  (`crates/pii-server/src/agent.rs:237,291`) that errors out ("max_tokens exceeded") rather
  than compressing — a real, currently-unhandled gap (see §3.3).
- `pii-model`'s `ModelProvider` trait (`crates/pii-model/src/lib.rs:118`) is the one seam
  that matters for any of this: `fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError>`,
  sync, provider-agnostic. Anything — Ollama, llama.cpp HTTP, a future in-process GGUF
  runner — plugs in here without touching `pii-graph` or `pii-server`. This is the correct,
  already-built seam for wiring in a tiny model; no new abstraction is needed.

**Bottom line**: "tiny model real usage" in Pii today is not a tuning question on an existing
nano tier — it's a green-field decision, because the nano tier doesn't exist yet and the one
model in the loop (granite-4.2-3b) is currently doing tier-0-shaped jobs (routing, quick
classification) by brute force, at tier-1 latency (~1s+, per the 004 probe: 14–28 tok/s CPU),
because there is nothing cheaper installed.

## 2. Tiny chat model landscape (2026)

Sizes/specs below are from training knowledge (last verified against vendor docs before this
session's knowledge cutoff); the granite-4.2-3b row is the exception — those numbers are
**live-verified on this repo's own CPU box**, per 004. Re-probe anything else the same way
before trusting it for a latency or license decision.

| Model | Params | License | GGUF? | Ctx window | CPU tok/s (rough, no-GPU, Q4) | Instruction-following |
|---|---|---|---|---|---|---|
| **Needle (Cactus Compute)** | ~45M (per pass 3 §1.3: "a 45M model") | Apache-2.0 (model); **engine redistribution terms unread/unstated** | **No** — CQ2-bit weights baked into Cactus's own C++ engine; no GGUF, no Ollama tag, no llama.cpp path | Unstated in prior research; likely small (mobile-NLU scale) | N/A — not llama.cpp-compatible; own engine, prebuilt binary fetched from HF | Weak by design — built for closed-set NLU (intent/slot), not open chat |
| **granite-4.2-3b** | 3B (dense) | Apache-2.0 (verified: crypto-signed, ISO cert) | Yes — `ibm-granite/granite-4.2-3b-GGUF` | 128K | **14–28 tok/s, live-probed** on this box (`--reasoning off`) | Good — IBM-tuned for agentic/tool-call use; needs the thinking-mode gate (004 §"Probe") or content comes back empty |
| **Qwen2.5-0.5B / 1.5B** | 0.5B / 1.5B | Apache-2.0 | Yes, widely (Qwen team + community quants) | Up to 32K (native config; quality degrades hard near the top at these sizes) | 0.5B: ~60–100 tok/s; 1.5B: ~25–40 tok/s (extrapolated from the granite-3B probe's ~20 tok/s at 3B — roughly inverse to param count on the same box) | Best-in-class for size on instruction-following/JSON/function-calling benchmarks; the strongest sub-2B chat model as of early 2026 |
| **SmolLM2** (135M/360M/1.7B) | 135M–1.7B | Apache-2.0 | Yes (HF ships GGUF quants directly) | 2K–8K depending on variant | 135M: 100+ tok/s; 1.7B: ~15–25 tok/s | 1.7B-Instruct is usable for simple structured tasks; 135M/360M are closer to completion models — thin instruction-following, prone to schema drift |
| **Phi-3-mini** | 3.8B | MIT | Yes | 4K (`-4k-instruct`) or 128K (`-128k-instruct`) variant | ~10–20 tok/s | Strong for size — curated "textbook-quality" training data, above-average structured output and light reasoning |
| **TinyLlama-1.1B** | 1.1B | Apache-2.0 | Yes | 2K (extendable) | ~40–80 tok/s | Weak — largely superseded by Qwen2.5-0.5B/1.5B and SmolLM2 for anything needing schema-constrained output; treat as legacy |

Two observations that matter more than any single row:

1. **Needle is not a drop-in candidate for anything in this table.** It's not a general chat
   model — Cactus built it for closed-set on-device NLU (this matches pass 3's own framing:
   "the nano LLM's advantage is free-form NL → arbitrary schema-valid args", i.e. extraction,
   not conversation). And it fails Pii's own build-dependency bar from pass 3 §3 — no GGUF,
   prebuilt binary fetched at runtime from HuggingFace, Python-only bindings, musl/ARM64
   support unstated. That verdict hasn't changed; nothing in this research surfaces a reason
   to revisit it.
2. **Qwen2.5-1.5B is the only genuinely new option** relative to what 004 already
   evaluated (which only looked at granite-4.2-3b and ornith-1.5:9b for tier 1, and B1/B2/B3
   for tier 0). It's the model actually worth a probe if this doc's recommendation (§4) is
   pursued.

## 3. Where a tiny chat model could plug in — evaluated against Pii's real shape

Pass 3 §1.5 already established the load-bearing principle: **a nano/tiny model must never
be the last decision before a `MUTATES_EXTERNAL` effect**, and it loses to a deterministic
check whenever one is available (§1.5's example: exit code + regex beats a nano `classify`
call for "did the test pass"). Apply that filter to each candidate:

### 3.1 Prompt routing / classification ("is this a coding task or a question?")
Pass 3 already answered this in detail (§3.1): a fitted classifier over `TrigramEmbedder`
features (B1) wins over any LLM here — deterministic, replayable from the journal,
calibrated confidence, zero new deps, improves via `pii nano tune`. A tiny chat model adds
non-determinism and an extra process/IPC hop for a closed-set decision a linear classifier
already solves better. **No change to that verdict.** A tiny chat model is not the tool for
this job, full stop — this was already the right call.

### 3.2 Tool-argument repair (fix malformed JSON args)
This is genuinely closer to what a small *instruction-tuned* model is good at — it's
schema-constrained (you have the target JSON schema right there), short input/output, and
"free-form NL → arbitrary schema-valid args" is exactly the capability pass 3 §3.1 credited
to the nano-LLM backend (B2) over B1. But two things cut against adopting a *tiny chat
model* specifically for it in Pii:
- **Malformed-JSON repair from a well-formed-but-broken LLM output is usually a parsing
  problem, not a language problem.** The common failure modes (trailing comma, unescaped
  quote, truncated at `max_tokens`, wrapped in markdown fences) are handled correctly and
  deterministically by a tolerant JSON5/JSONC parser or a bracket-balancing repair pass —
  no model call needed, 0ms, no failure mode of its own.
  Only the harder case — the model genuinely produced semantically-wrong-shaped args (wrong
  field names, hallucinated keys) — needs re-reasoning about *what the tool wants*, and a
  0.5–3B model does not reliably outperform just re-asking tier-1 granite-4.2-3b with the
  schema and the error appended, at a cost of maybe 200–400ms extra on an already-degraded
  path. Given repair is already rare (pass 3 §1.5 point 3: `repair_attempts: 0` in the
  existing design), this isn't a hot enough path to justify a second model in the process.
- **Verdict: deterministic JSON-repair pass first (cheap, catches most cases); fall back to
  the already-present tier-1 provider for the rest.** No tiny model needed.

### 3.3 Summarization of long tool output before it hits context window
This is the strongest real candidate, because it's a **currently-unhandled gap**, not a
displaced alternative — `agent.rs` today just errors on `max_tokens exceeded`
(`crates/pii-server/src/agent.rs:291`) rather than compressing. Properties that favor a tiny
model here:
- The task is genuinely generative (summarize N KB of shell/test output into a paragraph),
  which a deterministic heuristic (head/tail truncation, grep-for-errors) does *worse* than
  even a weak LLM — truncation silently drops the one error line that mattered.
- It's off the critical decision path — a summarization node's output feeds *back into* the
  next tier-1 reasoning turn; it doesn't itself gate a `MUTATES_EXTERNAL` effect, so pass 3's
  §1.5 "never terminal" rule isn't violated.
- Latency budget is generous (this happens once per oversized tool call, not per token), so
  even Phi-3-mini or granite-4.2-3b's ~15-20 tok/s is tolerable — but a 0.5–1.5B model at
  40-80 tok/s turns a ~2s summarization pause into ~0.5s, which matters if this fires
  multiple times in one agent run (e.g. a build log, then a test log, then a diff).
- Risk: summarization quality directly affects downstream reasoning correctness (an omitted
  error line silently misleads tier 1). This needs eval against real tool-output samples
  before trusting it in a production run — not a blind adopt.

**Verdict: real candidate.** Qwen2.5-1.5B or granite-4.2-3b (already installed, zero new
dependency) are both plausible; §4 recommends starting with granite-4.2-3b itself run at
short `max_tokens` rather than installing a second model, because the summarization job
doesn't need to be *fast*, it needs to not drop the wrong line, and a bigger model is safer
at that until proven otherwise.

### 3.4 Extract structured facts for memory
Same shape as tool-arg repair (schema-constrained extraction), and the same B1-vs-B2 framing
pass 3 already laid out applies: `TrigramEmbedder` (B1) cannot do free-form NL→structured
extraction (it has no generative capability at all — it's a hash embedder), so *if* Pii wants
memory writes to include extracted structured facts (not just raw text + BM25/vector
indexing, which is all `pii-memory` does today per `crates/pii-memory/src/{bm25,vector,doc}.rs`),
this is a real gap a tiny model could fill that B1 structurally cannot.
But: nothing in the current `pii-memory` crate does structured extraction at all yet — there
is no consumer for this capability in the codebase today (`consolidate.rs`/`decay.rs` operate
on existing documents/scores, not on extracting new facts from text). Building this is
speculative ahead of a concrete need. **Verdict: plausible future use, not a current gap** —
revisit when `pii-memory` grows a feature that actually wants structured facts (a "memory
consolidation" pass that turns a raw conversation into typed facts would be the trigger).

### 3.5 Gate decisions (pass/fail on test output)
Pass 3 §1.5 already covers this explicitly and correctly: "exit code plus a regex is better,
cheaper, and deterministic" than any model call, tiny or not. Test pass/fail has a ground
truth (the exit code, the assertion count) that no model should be interposed in front of —
doing so adds a hallucination surface to a decision that's already unambiguous.
**Verdict: no model, tiny or otherwise. Deterministic check only. Confirmed, not revisited.**

## 4. Recommendation

**Do not adopt Needle, or any dedicated tiny chat model, right now.** Two independent reasons:

1. **The nano tier itself doesn't exist in code yet.** Nothing in §3 needs a *second* model
   process today — the one real gap (§3.3, tool-output summarization) can be served by the
   tier-1 model already configured (`granite4.2:3b` via `ollama:`), through the exact same
   `ModelProvider` seam, by adding a `summarize` mode/prompt and a shorter `max_tokens`/
   `num_ctx` on that call. That's a few hours of work behind the existing seam, not a new
   dependency, new build target, or new process.
2. **Every candidate that *would* justify a dedicated tiny model (3.1 routing, 3.5 gating)
   is better served by what pass 3 already chose — a deterministic classifier or a plain
   exit-code check — and that verdict holds up under this second look, not just the first.**
   The one genuinely new capability a tiny *generative* model adds over B1
   (`TrigramEmbedder`) — free-form extraction — has no current consumer in `pii-memory` to
   justify building it speculatively.

**If/when a second, smaller model becomes worth installing** (e.g. summarization volume
grows enough that granite-4.2-3b's ~20 tok/s becomes the bottleneck, or `pii-memory` grows a
fact-extraction consumer), the concrete pick is **Qwen2.5-1.5B-Instruct**, not Needle and not
TinyLlama:
- GGUF-native (no vendor/artifact lock, unlike Needle's prebuilt-binary Cactus engine —
  same objection pass 3 §3 raised against Needle applies unchanged today).
- Apache-2.0, same license family already in use (granite, and matches pass 3's
  license bar).
- Best-in-class instruction-following at its size for schema-constrained tasks
  (summarization, extraction) — the two jobs §3 actually found real gaps for.
- Runs through the **same `ModelProvider` seam** already built (`crates/pii-model/src/lib.rs`)
  — either as a second `ollama:qwen2.5:1.5b` spec, or via llama.cpp HTTP exactly like the
  004 probe already validated for granite (`llama-server -m ... -c 4096 --reasoning off`,
  if Qwen2.5 needs the same thinking-mode gate — verify before adoption, don't assume).
- **Gate before adoption** (same bar 004 set for granite): a live CPU probe on this box
  confirming `content` is non-empty, no reasoning-token trap, and a real tok/s number — do
  not trust the table in §2 uncritically.

Needle specifically stays rejected for the same reason pass 3 §3 already gave: it is a
vendor-and-artifact lock (prebuilt binary fetched at runtime, no GGUF, Python-only bindings,
musl/ARM64 unstated, engine redistribution terms unread), not a model-capability problem —
and that objection is orthogonal to whether a tiny chat model has a job to do in Pii. It
doesn't have a viable integration path into a static-musl Rust binary regardless of how good
the model itself is at 45M params.

## 5. Summary table

| Candidate use | Real gap today? | Tiny chat model wins? | What wins instead |
|---|---|---|---|
| Route/classify (§3.1) | No — already B1 (deterministic) | No | `TrigramEmbedder` + fitted head (unchanged from pass 3) |
| Tool-arg JSON repair (§3.2) | Rare (repair_attempts≈0) | No | Tolerant JSON parser first; tier-1 model as fallback |
| Tool-output summarization (§3.3) | **Yes — currently errors out** | Only for latency, not correctness | Tier-1 model (granite-4.2-3b) via existing seam, new prompt mode |
| Structured fact extraction for memory (§3.4) | No current consumer | Plausible later | Not yet — no feature needs it |
| Gate pass/fail (§3.5) | No | No | Exit code + regex (unchanged from pass 3) |

**Net: zero new model processes recommended right now.** The one real gap is closeable with
the model Pii already runs, through the seam it already has.

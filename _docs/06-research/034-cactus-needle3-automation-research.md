# Cactus Needle 3 — research and relevance to pii (2026-09-19)

Status: research findings. No code, no spec. Written for later analysis.
Sources: cactuscompute.com (release + 7 guides), the HF model card, the
GitHub repo, the HN thread, the two YouTube reviews and the ESP32 port
repo. Every number below is quoted from a source, not estimated.

## What it is

Cactus Compute's third release of **Needle**, an open (Apache-2.0)
**automation foundation model** for tiny devices. It does **not chat** —
that is a deliberate trade, not a limitation. Three jobs, all on-device:

1. **Tool calls** — given declared functions, pick the right ones and fill
   every argument from what the user said.
2. **Structured extraction** — declare a shape, hand over messy text, get
   typed fields back (an invoice, a booking, a form). Extraction generalises
   to classification via enums.
3. **Text embedding** — the same model returns a sentence vector, so an app
   can search/match/route locally.

Released **2026-09-17** (pyproject 3.0.1, engine 3.0.1). 4.2k+ GitHub stars.

## The numbers that matter

| | |
|---|---|
| Parameters | **29–121M**, sliceable at any depth 2–20 layers from ONE weight set |
| Binary size | **8–29 MB** shipped CQ2 (4L = 8 MB, 20L = 29 MB) — *not locally reproducible, see below* |
| Quantization | **CQ2, 2.125 bits per weight**, trained in (not post-hoc) |
| Engine | under 1 MB per platform, 13 platform folders |
| Decode speed | **400–4k tok/s decode, 1–10k tok/s prefill** on a Raspberry Pi 5 |
| Context | **1024-token window**, global attention at layers 4, 9, 14, 19 |
| Training | **360B tokens** of a proprietary structured dataset |
| Languages | EN, FR, ES, DE, NL, IT, PL |
| License | Apache-2.0 (weights + code); the 2-bit quantisation runs on their paid platform |

### CORRECTION (2026-09-19, from the Colab notebook, cells 27-29)

**The advertised 8 MB floor is not reproducible from the open repo.** A local
`needle build --layers N` produces **4-bit** artifacts, because
`needle/model/quantize.py:61` sets `WEIGHT_BITS = 4` and `export.py`'s
`_cq_pack` raises `ValueError(f"CQ packing supports bits={WEIGHT_BITS}")`
for any other width (both verified directly against `main`). The 2-bit
quantiser behind the shipped artifact is **not in the open repo** — and the
official release page confirms this from its side: *"The Cactus Platform is
the full path: Cactus datasets, the **2-bit quantisation behind the shipped
model**…"*. Local builds measured:

| layers | size | share of bytes |
|---|---|---|
| 2 | **13.3 MB** | 38% |
| 8 | 27.4 MB | 78% |
| 20 | 35.3 MB | 100% |

**The ladder is a compute dial, not a size dial.** 2 layers is 10% of the
depth but still ~38% of the bytes, because ~70.8M of the 121M parameters
live in the *engram* — the hashed n-gram tables the ladder cannot cut. What
slicing saves is MFLOPs per token (the page claims **over 2x fewer MFLOPs
per token than a transformer of the same configuration**, Figure 1), i.e.
battery and latency on a watch or an MCU — not flash.

> **CORRECTION 2 — context window.** An earlier draft of this document said
> "256-token sliding window, tools pinned as KV sinks". **That is a Needle 2
> / ESP32-port figure, not Needle 3.** The official architecture block on
> `cactuscompute.com/needle` states **`window 1024` with global attention at
> layers 4, 9, 14, 19**. Treat 1024 as the Needle 3 context, and 256 as the
> ESP32 port's *implementation* limit (see the side section below) — the
> distinction matters a lot for the tool-fitting arithmetic.

Benchmarks (full test splits, exact-match):
- **Mobile Actions** (961 instances): 20-layer Needle 3 = **86.0**, vs
  DeepSeek V4 Flash (cloud) 88.4 and LFM2.5 **1.2B** 82.4.
- **DroidCall** (200 instances, two sequential calls): **47.0**, second
  behind DeepSeek V4 Flash, above LFM2.5 1.2B (35.5) and Qwen3.5 0.8B (28.0).
- **BFCL v4** (general function calling): only **50.2** — *behind* LFM2.5
  1.2B (62.0), LFM2.5 350M (59.1) and Qwen3.5 0.8B (56.8).

**Read that third row carefully**: on the general benchmark it loses to
models a fraction of its size. Its advantage is *domain-shaped* device
actions, not function calling in general. Cactus says so themselves.

## Architecture, in one paragraph

A **Laddered Simple Attention Network**: the dense FFN layers are gone,
replaced by a **Monarch Hadamard MLP** (25.6K parameters per layer — a
fixed Walsh–Hadamard transform does the mixing for free). The knowledge an
FFN would have memorised instead sits in an **engram**: hashed n-gram tables
read by gather at zero arithmetic. Most parameters are in the engram, so the
121M model does the arithmetic of a ~50M one. Plus GQA attention with causal
conv taps and multi-lane hyper-connections. The **ladder** is the key
property: blocks 0 and 19 are always kept, the rest added by bisection, so
every subnetwork nests inside the next — one artifact, many depths.

**The official architecture block** (`cactuscompute.com/needle`) names the
components precisely, and adds detail worth having: `mHC lane read/write`
over **4 residual lanes** (`X' = P X + 2σ(h) ⊗ y`, `P = Sinkhorn(A)` — a
Sinkhorn-normalised lane mixer); `Engram fusion · hashed 2- and 3-gram
memory` (`x ← x + σ(⟨x̂,k̂⟩/√d)·v`) at **sites 3, 7, 11, 15, 19**, with
**18,432 slots**, read by gather; `Q,K,V` at **12q : 2kv** with a
**3-tap causal conv per channel**; RoPE + QK-norm; `Monarch Hadamard FFN`
as `y = D₄M₃D₃M₂ silu(D₂c(x)M₁D₁x + b)` with `Mᵢ = Aᵢ ⊗ Bᵢ` at 32×32;
`ZCRMSNorm`; embedding 8,192 × 768, **tied unembed**, SentencePiece BPE with
8,192 pieces; and an **`mtp t+2` auxiliary head (training only)**. Total
**20 × 768 = 121M, of which 70.8M sits in the engram tables**.
Its claim: **over 2× fewer MFLOPs per token** than a transformer of the same
configuration.

## The four mechanisms worth stealing (mapped to pii)

### 1. Grammar-constrained decoding = the output cannot be malformed

Schemas compile to a **byte-level grammar** that constrains every token.
Bounds, enums, patterns, lengths and item counts are compiled in, so an
invalid value is **unrepresentable rather than discouraged**. One tool
declared ⇒ exactly one call of that name is admissible (a `tool_choice`
equivalent). No JSON mode, no repair pass.

**pii relevance — high.** This is the same principle as pidag's "the
scheduler does not reason during the run, it follows declared rules", and
the same spirit as pii's F-11 structured-error work (`StoreOp` replacing
free-text `Internal(format!)`): *make the wrong shape impossible instead of
validating it after the fact*. pii's tool layer (`pii-tools`, the
`ToolSpec`/`schema` JSON in `bash.rs`/`fs.rs`/`worktree.rs`) already
declares schemas — those schemas are currently used for description, not
enforcement (spec-62 was exactly a case where `signature` was described as
"blake3 hex" but never constrained). A grammar-constrained decode path
would have made spec-62's bug structurally impossible.

### 2. Confidence-gated escalation — act / confirm / refuse

Every response carries a **calibrated** `confidence`: the **minimum** of
(a) a post-hoc head scoring the full prompt + the produced call, and
(b) the decode probability of the call tokens. Both must agree. The engine
**withholds anything under 0.1** (`suppressed_calls`, `function_calls`
empty). Above that, the documented pattern is three bands:

```
if calls and confidence >= threshold:  execute(calls)      # act
elif calls or held:                    confirm(calls, reasoning)  # show the call
else:                                  refuse()
```

Because wrong calls sit low and right ones sit high, **raising the
threshold removes wrong executions first and costs confirmations second**.

**pii relevance — very high, and it validates tonight's own finding.** This
is precisely the `free → account → topup` ladder, and precisely pidag's
`models_for_iter` escalation slot. But note the difference in *signal*:
pidag escalates on a **validator verdict**; Needle escalates on a
**calibrated self-assessment of the specific call**. Tonight's pidag defect
(`docs/FINDINGS.md` `26c1ff7`) was that a timed-out iteration's partial work
passed validation, so the stronger rung was never asked. A confidence-gated
design gives the ladder a second, cheaper signal to fire on — and the
"show the call and its reasoning for approval" middle band is the missing
"paused, awaiting human input" state that the earlier pidag research
(spec-129..132's candidate #5) deferred as an architecture item.

The `reasoning` line matters here: it is the model's own derivation of each
argument from its source span (`'ten minutes' -> minutes 10`), which is what
makes a one-tap human confirmation possible.

### 3. Tool retrieval — an unselected tool is *unreachable*, not unlikely

Five or fewer tools render directly. Above that, every schema is embedded
once (a built-in contrastive head), each request is embedded, and **only the
five closest enter the context with the grammar rebuilt over just them**. An
unselected tool cannot be emitted at all. Embeddings persist to disk keyed
by a fingerprint over schemas + model.

**pii relevance — high, directly answers a finding.** The fable review
(`_docs/06-research/032-*`) flagged that `pii-mcp/src/main.rs` registers
`ToolSet::builtin` **wholesale**, so every MCP client sees every tool —
including the one with the spec-62 vulnerability. That is a growing surface.
Needle's model — narrow the admissible set per turn rather than hope the
model picks well — is a concrete design answer, and it is *enforcement*
(grammar over the subset), not a prompt hint.

### 4. Frozen environment test suites (32 cases, six categories)

`needle.environments` ships, per environment, a **frozen suite of 32 cases**
in six categories: the exact call for a positive request; and `[]` when a
required value is missing, when no tool covers the request, **when the
request is negated**, when a stated value is out of bounds; plus two calls
from one request (order-insensitive). They run against the shipped engine
and again at the product's confidence threshold.

**pii relevance — very high; this is the direct fix for tonight's weakest
spec work.** The measured problem in the 62-69 run was exit criteria that
(a) passed before implementation at all (spec-62's baseline scored 6/6 on an
untouched tree) and (b) tested a weaker surface than the gate (`-p <crate>`
vs `--all-targets`). Needle's category list is a ready-made template for
criteria that **discriminate by construction**: every category includes the
*refusal* cases, which cannot pass vacuously — a system that does nothing
also refuses, so the positive case must carry the weight, and the negative
cases catch over-eager implementations. Worth adopting as a spec-authoring
rule: every spec's criteria should include at least one "must NOT do X" case.

### 5. Triggers — reaching intents a description cannot enumerate

`triggers` are case-insensitive regexes matched against the request. A match
**restricts the decode to the tools that matched and removes the empty
call**, so the phrasing reaches the named tool even below the confidence
floor — while contradiction gates (negated, reported speech) still withhold.
The guide is candid that a catch-all trigger must exclude the nouns other
tools own, or it swallows them.

**pii relevance — medium.** This is a declarative routing rule layered onto
a model decision, i.e. exactly pidag's "declared rules, no runtime
reasoning" pattern applied *inside* a model call. Note the honest
limitation: a trigger bypasses the guess gates but not the contradiction
gates — a two-tier safety model worth copying (cheap deterministic override
for recall; non-negotiable gates for safety).

## The Colab notebook — measured results (the most valuable artifact)

The video by Elvis Saravia (**Prompt Engineering**, 2026-09-18, "Needle 3:
You Don't Need an LLM for Function Calling") links a **Colab notebook**,
resolved from `tinyurl.com/yewmt3za` to
`colab.research.google.com/drive/1lugh7FfHKYmcghyip5d7XYGDCVCXeeR5`.
A local copy is at `_docs/06-research/assets/034-needle3-colab-notebook.ipynb`
(37 cells). Its framing: *"Every number below was measured, not copied from
the docs. Where the model does badly, that's shown too."* It delivers on
that, and **several results contradict the vendor material above.**

Measured on laptop CPU: cold load ~10 s (35 MB from HF, cached after);
two-call turn **~100 ms**; `decode_tps` reported per response.

### N1. `reset()` between independent requests, or you get phantom calls

A `Needle` instance keeps conversation history. Reused across independent
requests it **re-emits a previous call for an unrelated prompt** — "order me
a pizza" set the thermostat. The call looks perfectly well-formed and
confidence only drops to ~0.45. With `reset()` it correctly returns `[]`.

This is the **most important operational finding in the notebook**: silent,
plausible wrong behaviour, not an error. Any server handling multiple users
from one instance has a correctness bug.

### N2. A required argument with no default gets the call withheld

`set_thermostat(room, celsius)` with no default means "make it warmer in
here" produces **nothing** — there is no number span to ground `celsius` in,
and Needle refuses to invent one. Deliberate, mostly good — but the failure
mode is **silence, not an error**, and the fix is in the *tool definition*
(add a default), not the prompt.

### N3. Refusal is the actual feature

"water the plants" — a plausible smart-home request with no matching tool —
correctly does nothing. This is the reason to use this over prompting a big
LLM to "only output JSON", and it held up under test.

### N4. Extraction: copies, does not normalise (and `strict` does not help)

`Invoice(vendor, total, due_date)` on clean templated text works, and
`total` is a **real float**, not a string. But on conversational text
("payable by the 14th of next month (Oct 2026)") **both `strict=True` and
`strict=False` return `None`** — "the 14th of next month" is not a span it
can copy into `due_date`.

**Use for:** receipts, notifications, log lines, form-shaped text.
**Do not use for:** free-form prose needing inference or reformatting.

### N5. Classification: an honest NEGATIVE result — 1 out of 5

The Cactus site claims extraction "generalised well to classification
problems too". **Verified 2026-09-19: the official release page still carries
that exact line** — under *Structured extraction*: *"The decode grammar
guarantees the output parses. Extraction generalised well to classification
problems too."* The notebook tested a 6-way banking intent task, following
Cactus's own advice to name enum options the way users speak. Result:
**1/5 correct** — two returned `None`, two picked the wrong label. For
scale, a hosted classifier API on a comparable 77-way banking task scores
~80%.

"Do not read the marketing line as a general text-classification claim."
This is a live discrepancy between the vendor page and measured behaviour,
not a stale reading of an older page.

### N6. Embeddings are strongly anisotropic — rank, never threshold

Raw cosine: a paraphrase scores ~0.95 and a **completely unrelated**
sentence still scores ~0.91. Absolute values are nearly meaningless. The
notebook applies the classic mean-centring ("all-but-the-top") fix and
measures top-1 routing accuracy raw vs centred.

**Rule: never threshold on raw cosine here — rank, and centre the index.**

### N7. Confidence routing FAILS on vague input — the strongest caveat

The notebook ran Cactus's own documented act/confirm/refuse pattern
(threshold 0.7) against vague commands:

| request | call | confidence |
|---|---|---|
| "turn off all the lights" | (correct) | — |
| "lock up" | (correct) | — |
| "do the thing" | `lock_door(door="the thing")` | 0.996 |
| "handle it" | `lock_door(door="handle")` | **1.000** |
| "sort that out for me" | `lock_door(door="out")` | **1.000** |
| "just do it" | `lock_door(door="just")` | 0.989 |

**Every vague-but-meaningless command locks the front door at near-maximum
confidence.** The argument is an arbitrary word lifted from the sentence:
the span-grounding rule is satisfied (every value *is* a span of the
request), the grammar is satisfied (it is a valid string), and **nothing
downstream objects**.

This is a direct, measured counterexample to mechanism 2 as a safety gate.
The confidence score measures *how well the call follows from the text*, not
*whether the request was meaningful*. A calibration that survives benchmark
inputs does not survive adversarial or merely lazy ones.

### N8. A working end-to-end pattern (~20 lines)

Final cell: three tools (`lights`, `door`, `fan` with a `Literal` enum),
`reset()` per command, and — the detail worth copying — **`complete()`
decides, it does not execute; the caller dispatches `[call["name"]](**args)`
explicitly** so the application stays in control of what actually runs.
(`agent.run()` does both.) Multi-call turns work: "dim the bedroom to 20 and
put the fan on high" produced both calls in order.

### N9. Packaging defects found

The published quickstart **does not install cleanly**:
`cactus-needle` is missing three dependencies its own documented examples
require — `pydantic` (extraction), `numpy` and `sentencepiece` (`needle
build`). Also: telemetry is **on by default** (function name, version, OS —
not prompts or outputs); `NEEDLE_TELEMETRY=0` opts out.

## Side section: the Needle **2** ESP32 port, measured

> **VERSION NOTE.** This section is about **Needle 2** — a separate,
> independent port, kept only as an on-device reality check. The rest of this
> document, including the Colab notebook above, is **Needle 3**. Do not
> confuse the two: the `needle-2-esp32` repo contains Python *scripts*
> (`tools/*.py`), **not** the notebook; the Needle 3 notebook is the Colab in
> the previous section.

An independent port (`github.com/andrisgauracs/needle-2-esp32`, C99, ~3,700
lines, no dependencies, not derived from Cactus's C++ engine) runs Needle 2
fully offline on an ESP32-S3:

| | |
|---|---|
| Chip | ESP32-S3, **16 MB flash** (model alone needs 13.1 MB), **8 MB octal PSRAM** |
| Per token | 534 ms = **1.87 tok/s** at 240 MHz |
| One request | ~25 s (reasoning off) / **~47 s** (reasoning on) |
| Boot prime | ~51 s, once |
| Workload | **compute-bound**, not memory-bound — faster flash/PSRAM buys nothing |

Hard limits **of this port** (Needle 2-era C implementation, not of Needle 3
itself): 256-token window is the real ceiling here (example schema = 104
prompt tokens; a second tool takes it to 182); **2–3 tools fit**, ~5 needs
the retrieval head the port doesn't implement; types supported are `string`
(+`enum`), `integer`, `number`, `boolean` — **nested objects and arrays are
rejected at compile time**; tool names ≤ 39 chars. Needle 3's own window is
1024 (see CORRECTION 2 above), so this port's tool budget is a limitation of
the port, not of the model.

**Relevant to the parked Google Home / home-automation thread**: on-device
natural-language intent routing on a ~$10 board is real and verified, but
~47 s per request for the full model is far too slow for a voice assistant
("turn on the lights" must be instant), and it is a *dispatcher*: no world
knowledge, no free text. The 500 tok/s figure is a **Raspberry Pi 5**, which
is the realistic host for anything interactive — the ESP32 result is a
proof of feasibility, not a product.

## Caveats and known weaknesses (do not skip these)

1. **The confidence head is not tuned.** Load a tuned `.cact` via `weights=`
   and the package reports `confidence` as `None` and warns once. So the
   escalation signal that makes mechanism 2 valuable **disappears exactly
   when you fine-tune** — i.e. in the deployment Cactus recommends.
2. **Non-English scoring is unreliable.** Cactus measured **correct Spanish
   calls at 0.0**. For a Portuguese-speaking deployment (relevant here:
   UFC coursework, local voice) the confidence band model would need
   re-measurement, not assumption.
3. **BFCL v4 = 50.2.** General function calling is its weakest area, behind
   far smaller general models. Do not evaluate it on general tool-calling.
4. **Community reception is tempered.** The daily.dev summary of the release
   notes commenters found "frequent nonsensical outputs and low robustness
   to ambiguous or out-of-distribution inputs" in the web demo (45% positive
   / 40% mixed / 15% skeptical). The company's own docs are unusually honest
   about this ("small models can be finicky", "no small model is perfect").
5. **2-bit post-training is not fully open.** Local fine-tuning exports
   **4-bit**; the 2.125-bit path runs on Cactus's paid platform. The shipped
   artifact is open; the exact recipe behind it is not.
6. **Ambiguous input fails in measured ways**: in the ESP32 demo a request
   for the light to "display" yellow was interpreted as *flash*, and scored
   the lowest confidence of the run (0.46). Description wording is described
   as "a tuning knob".

## Secondary Python: the Needle 2 ESP32 tooling (NOT the notebook)

> The **Needle 3 notebook** is the Colab in the earlier section
> (`assets/034-needle3-colab-notebook.ipynb`). The Python below is a
> **different, Needle 2-era** set of scripts with a different purpose:
> validating a C engine, not demonstrating the model.

Helper scripts in `andrisgauracs/needle-2-esp32/tools/`:

- **`ref_forward.py`** — a **NumPy re-implementation of the Simple
  Attention Network forward pass, read straight from the `.cact` blob**,
  full-sequence and non-incremental on purpose (so comparing it against the
  C engine's incremental decode also validates the KV cache and ring buffer).
  This is the single most valuable artifact for anyone wanting to understand
  the architecture without JAX: the architecture in readable Python.
- **`check_engine.py`** — decodes the `.cact` container (v3 tag
  `0x05E12A83`, 30-word header, 44-byte records) and validates the C engine's
  dequant + GEMV against NumPy to < 3e-07.
- **`check_tokenizer.py`** — validates the embedded SentencePiece against
  the real tokenizer id-for-id, **including the chat markers**
  (`<|im_start|>`, `<tool_call>`, `<think>`), which is where the prompt
  template contract lives.
- **`needle_tui.py`** — the demo console; documents the device line protocol
  (`EVT priming`, `TOK`, `CONF <float>`, `ACT led color=...`, `ACT none`).

Reference validation targets from that repo: CQ2/CQ4 dequant + GEMV vs NumPy
< 3e-07; tokenizer exact 15/15; 27-layer forward ~2.5e-05 with identical
argmax; grammar 18/18.

## So-what for pii

Ranked, with the honest caveats attached:

1. **Adopt Needle's negative-case test-suite shape for specs.** The
   six-category list (positive; refuse when missing / uncovered / negated /
   out-of-bounds; multi-call) directly addresses tonight's measured
   non-discriminating exit criteria. This is free, needs no dependency, and
   is the highest-leverage item here.
2. **The confidence-band routing design (act / confirm / refuse) is a
   *starting point, not a safety gate*.** The notebook's N7 measured it
   locking a door at 0.989–1.000 confidence on meaningless input. Adopt the
   *shape* (three bands, show the call + reasoning in the middle) — but treat
   the score as "does this call follow from the text", never as "was this
   request meaningful". Any gate still needs a deterministic check the model
   cannot satisfy by finding a plausible span. Also note Needle's own head
   goes dark (`None`) once fine-tuned.
3. **Consider a Needle-backed router node** for pii's graph: a cheap, local,
   grammar-guaranteed node that turns free text into a structured call, for
   the class of graph decisions that are *retrieval-and-assembly* rather
   than reasoning. Cactus's own framing — "tool calling is retrieval and
   assembly, not reasoning" — is the argument for why a 121M model can do it.
   Measure against BFCL-shaped tasks before believing it generalises.
4. **Grammar-constrained output as the principled version of F-11 and
   spec-62** — make invalid shapes unrepresentable rather than validated.
5. **Tool-set narrowing (retrieval) or per-turn declarations** as the answer
   to `pii-mcp` registering every builtin tool wholesale.
6. **For the home-automation thread**: a Pi-5-class host, not an ESP32, for
   anything interactive; and budget for the multilingual-confidence caveat.

## Facts confirmed on the official release page (2026-09-19)

Checked line by line against `cactuscompute.com/needle`:

- **Confirmed:** the 2-bit quantisation behind the shipped 8–29 MB binary is
  **platform-only** ("*the 2-bit quantisation behind the shipped model*" —
  Cactus Platform section). This independently corroborates the notebook's
  finding that local builds are 4-bit only.
- **Confirmed:** the ladder sizes map to **4L = 8 MB, 20L = 29 MB**, and
  *"the first 4 layers are an 8 MB model, all 20 are a 29 MB one"*.
- **Corrected:** context is **1024** (`window 1024`, global at layers
  4/9/14/19) — not 256.
- **Corrected:** parameters are **29–121M** (the page's own range), and the
  speed claim is **400–4k tok/s decode, 1–10k prefill** on a Pi 5 (the
  earlier "up to 4k" was the decode ceiling only).
- **New:** training is **360B tokens of a proprietary structured dataset**;
  decode 400–4k, prefill 1–10k on a Pi 5.
- **New:** an **in-browser WebAssembly sandbox** runs the model live on the
  page, with 13 preset demos (2-call, trigger routing, polarity+exclusion,
  negation, catch-all skipping named devices, enum+door security, relative
  dim, thermostat nudge, 5-change scene, status question).
- **New:** **13 deployment targets** with exact folder names and artifacts
  (`macos-arm64`, `linux-{x86_64,arm64,armv7,riscv64,mipsel}`,
  `windows-{x86_64,arm64}`, `android-{arm64,armv7,riscv64}`,
  `ios-{arm64,sim-arm64}`, `tvos-arm64`, `watchos-arm64`, `wasm`,
  `wasm-component`) — each an engine under 1 MB plus `libneedle.a`/`needle.h`
  (or `needle.js`/`needle.wasm`, or a WASI `.component.wasm` + `.wit`).
- **Strengthened:** *"4L can match DeepSeek V4 Flash when tuned on
  downstream tasks for one epoch"*, and tuned subnetworks *"from 4 layers up
  … starting at 29M parameters"* pass DeepSeek V4 Flash. **One epoch** is the
  headline for cheap customisation — but note the notebook's caution that
  the confidence head is not tuned by this path.
- **Still disputed:** the page repeats *"Extraction generalised well to
  classification problems too"*, which the notebook measured at 1/5.

## Open questions for later analysis

> **Follow-up (2026-09-21):** `039-jev-system-one-decision-layer-analysis.md`
> compares Needle 3 against Jev (TypeSafe System One) and resolves this
> "same shape of decision?" question: the **seam** is the same (a decision
> source feeding `route`), but the **decision classes differ** — Needle is the
> *dispatcher* (text → grounded tool call), Jev is the *classifier/scorer*
> (state → typed judgement with calibrated confidence). Jev does not replace
> Needle; register B6 is unchanged. §3.5 of 039 carries the side-by-side table
> and the per-decision assignment.

- Does grammar-constrained decoding map cleanly onto pii's existing
  `ToolSpec` JSON, or does it need a schema subset (no nested objects/arrays,
  per the ESP32 port's compile-time rejections)?
- Would a Needle router node live inside `pii-graph` as a node kind, or
  outside as an MCP/CLI boundary? (Same question the Chromecast-protocol idea
  raised in `030-OPUS-START-HERE.md` — it may be the same shape of decision.)
- Worth a spike measuring Needle 3 on *our* actual tool schemas (pii-tools'
  bash/fs/worktree/grep/glob) before any design commitment — the BFCL gap
  suggests domain fit is everything here.

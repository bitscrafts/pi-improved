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
| Parameters | 25M–121M, **sliceable at any depth 2–20 layers from ONE weight set** |
| Binary size | **8–29 MB** (single `.cact` file) |
| Quantization | **CQ2, 2.125 bits per weight**, trained in (not post-hoc) |
| Engine | under 1 MB per platform, 13 platform folders |
| Decode speed | up to **4k tok/s** on a Raspberry Pi 5 |
| Context | 256-token sliding window, tools pinned as KV sinks |
| Languages | EN, FR, ES, DE, NL, IT, PL |
| License | Apache-2.0 (weights + code); the 2-bit post-training runs on their paid platform |

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

## Reality check: the ESP32 port, measured

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

Hard limits: 256-token window is the real ceiling (example schema = 104
prompt tokens; a second tool takes it to 182); **2–3 tools fit**, ~5 needs
the retrieval head the port doesn't implement; types supported are `string`
(+`enum`), `integer`, `number`, `boolean` — **nested objects and arrays are
rejected at compile time**; tool names ≤ 39 chars.

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

## The Python that the videos point at

The "valuable Python" in the referenced material is in
`andrisgauracs/needle-2-esp32/tools/`:

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
2. **Take the confidence-band routing design (act / confirm / refuse) as the
   missing "paused, awaiting human input" state** in pidag/pii's graph work
   — with the caveat that a *calibrated* score is the hard part, and that
   Needle's own head goes dark once tuned.
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

## Open questions for later analysis

- Does grammar-constrained decoding map cleanly onto pii's existing
  `ToolSpec` JSON, or does it need a schema subset (no nested objects/arrays,
  per the ESP32 port's compile-time rejections)?
- Would a Needle router node live inside `pii-graph` as a node kind, or
  outside as an MCP/CLI boundary? (Same question the Chromecast-protocol idea
  raised in `030-OPUS-START-HERE.md` — it may be the same shape of decision.)
- Worth a spike measuring Needle 3 on *our* actual tool schemas (pii-tools'
  bash/fs/worktree/grep/glob) before any design commitment — the BFCL gap
  suggests domain fit is everything here.

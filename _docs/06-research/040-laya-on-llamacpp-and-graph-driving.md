# 040 — Laya on llama.cpp, containers, and how a System-1 model drives pii's graph

**Date**: 2026-09-21
**Status**: RESEARCH + MEASUREMENT. Infrastructure change made (container rename,
new Laya instance). No pii code change.
**Trigger**: user — *"does it can run from lamma.cpp? laya is target on decisions
no as a general llm, research on the internet! how can laya can improve our
projects! graph driving? state machine? do the research!"* plus two directives:
*"as lamma.cpp shown as more performant, when dealing with models, use the
approach lamma.cpp first!"* and *"rename the container! it must be a generic
lamma.cpp, laguna is only a model"*.
**Companion**: `039-jev-system-one-decision-layer-analysis.md` (what Jev/Laya are,
where a decision layer fits pii). This doc answers the *how*, on real hardware.

---

## 1. Can Laya run from llama.cpp? — YES, measured (hybrid, not monolithic)

**Answer: yes for the backbone, no for the decision head — but the split is
published and I ran it end-to-end here.**

An independent GGUF conversion exists (`fr0stbit3/laya-gguf`, Apache-2.0,
converted with llama.cpp's `convert_hf_to_gguf.py`). Its own README is explicit:
the GGUF files "hold the ModernBERT **backbone only**", load in llama.cpp as the
`modern-bert` architecture, and "the decision head … must run outside
llama.cpp". The head travels in a sibling `laya-head.safetensors` (the `mmproj`
pattern) and is read by `laya_head.py`.

**Why the split is real and not laziness.** Laya is not a language model: it is
ModernBERT-large + a small *decision head* over a `[MASK]` position, plus
per-question-type temperature calibration. llama.cpp's job is the transformer
backbone (`--embeddings --pooling none` → per-token hidden states); the head
(`logits → temperature scale → softmax → choice/score/noul`) is arithmetic on
top. So llama.cpp serves the expensive part — 421M parameters of encoder — and
the head is a few tensor ops in PyTorch.

**That is still a large win, because the 421M backbone was the entire cost.**

### Measured parity (this container, CPU, identical questions)

`<scratch>/laya-gguf/parity_test.py` — same state, same questions, one run
per path:

| path | dept | p(billing) | confidence | urgency | churn | wall |
|---|---|---|---|---|---|---|
| A — pure PyTorch (HF) | billing | 0.9670 | 0.8476 | 1.0415 | 0.1180 | 220 ms |
| B — **llama.cpp GGUF + PyTorch head** | billing | **0.9664** | 0.8452 | 1.0445 | 0.1158 | 608 ms |
| **delta** | — | **0.00060** | 0.0024 | 0.0030 | 0.0022 | — |

**Parity is real: Δp = 0.0006.** All 36 head tensors mapped into the model with
zero unmatched names. The backbone ran as `modern-bert` in llama-server over
HTTP; the head ran in PyTorch.

**Honest caveat on speed:** in *this* CPU-only container the hybrid is **slower**
(608 ms vs 220 ms) because it pays a per-call HTTP round-trip for the backbone
instead of keeping it in-process — 3 backbone calls = 543 ms of the 608 ms.
**llama.cpp is not automatically faster here; it is faster when it replaces a
model that would otherwise be resident-reloaded, and it is the *only* way to run
Laya outside the `laya` PyTorch stack.** The value is (a) one server serving many
models from one image, (b) quantised weights (Q8_0 = 421 MB vs 843 MB f16), and
(c) no per-model Python deps. Do not claim a speed win for the hybrid on CPU —
claim **format, memory, and packaging** wins, and measure before asserting speed.

### Quantisation tolerance (from the conversion's own card, single example)

| file | size | billing p | urgency | churn |
|---|---|---|---|---|
| HF original (PyTorch) | 843 MB | 0.967 | 1.630 | 0.198 |
| F16 | 933M | 0.966 | 1.631 | 0.201 |
| **Q8_0** | **563M** | 0.967 | 1.637 | 0.198 |
| Q6_K | 485M | 0.970 | 1.650 | 0.198 |
| Q4_K_M | 414M | 0.967 | 1.601 | 0.219 |

Q8_0 is the recommended quant for anything important; Q4_K_M drifts (0.02–0.04).
**I used Q8_0 for every measurement above.**

---

## 2. "llama.cpp first" for models — the approach, now in place

Per the user's directive, llama.cpp is the **default serving path for models on
this host**. Concretely, the host's llama.cpp setup was **not** generic: an image
(`localhost/llamacpp-laguna:local`), a container (`llamacpp-laguna`) and a unit
were all named after one *model*, even though the entrypoint was already
env-driven (`MODEL=`, `LLAMA_CTX=`, `LLAMA_PORT=`). The user's correction —
*"laguna is only a model"* — was exactly right. **Fixed:**

| before | after |
|---|---|
| image `localhost/llamacpp-laguna:local` | **image `localhost/llamacpp:local`** |
| container `llamacpp-laguna` | **container `llamacpp`** |
| unit `llamacpp-laguna.container` | **unit `llamacpp.container`** (old kept as `.retired`) |
| entrypoint hard-wired to chat args | entrypoint gains **`LLAMA_EXTRA_ARGS`** (any mode) |

- **Port 8082 is unchanged**, and the default model is still laguna, so pi's
  `models.json` (`http://<host>/v1`) needed **no change** — verified
  live: `/v1/models` returns the laguna id and a chat completion round-trips.
- The models directory is the **existing read-only mount**
  (`/podman/assistants/hermes/image/models` → `/models:ro`); Laya's GGUF was added
  **there** so nothing is duplicated: `laya-Q8_0.gguf` (421 MB) now sits beside the
  19 GB laguna file.
- **A second instance of the same image serves Laya in embeddings mode:**

  ```
  container  llamacpp-laya     (image localhost/llamacpp:local)
  port       8083 -> 8080
  MODEL      /models/laya-Q8_0.gguf
  LLAMA_EXTRA_ARGS  --embeddings --pooling none -ub 2048 -b 2048
  LLAMA_CTX  2048      LLAMA_JINJA 0
  ```

  Verified live: `/health` → `{"status":"ok"}` and `/embedding` returns 1024-dim
  per-token hidden states. One image, two models, two modes, no second toolchain.

**The generic pattern to reuse (this is the durable part):** one llama.cpp image
+ `MODEL` as the only identity-bearing variable + `LLAMA_EXTRA_ARGS` for mode
(chat / embeddings / reranker). Adding a model is now an env line and a unit
file — not a new image.

---

## 3. How Laya can improve our projects — the research

### 3.1 The pattern has a name in the wild, and it is the one we half-built

Internet research converges on a two-layer split with a consistent vocabulary:

- **"Deterministic shells, probabilistic cores"** — the framework is strictly
  defined, and the LLM is *contained inside it*: "the shell defines the arena,
  the core performs in it" … "the boundary between them is the most important
  design decision in your entire agent."
- **"Why Your AI Agent Needs a System 1"** (dev.to) — an agent paying
  Sonnet-class prices to check *whether anything needs doing* spends ~$15/day on
  silence. The fix is a front layer that decides **skip / quick-check / full
  wake**: "every check costs the same, whether it leads to action or not." Its
  measured result: **~24M tokens skipped in 5 days**, and — the important part —
  a **rules layer was added after the LLM-only phase produced a false negative**.
  The lesson is stated explicitly: *hardcoded rules first, model only for the
  ambiguous middle.*
- **Workflow-state guardrails** (a separate, lower layer) — guards decide whether
  a *state transition* may fire at all; "no model participates in the guarantee,
  which is exactly why the two layers [output guardrails and state guardrails]
  are complementary rather than competing."
- **Tiered routing** — the widely-published 2026 consensus: Tier 1 cheap model
  handles intent/classification for ~80% of traffic at <5% of cost; escalate the
  flagged remainder.

**Mapping our stack onto it — and this is the finding:**

| layer in the literature | what it must be | our asset | Laya's role |
|---|---|---|---|
| hard rules / "pre-attentive" | **deterministic, no model** | pidag's declared rules; `route` on `$.payload.exit_code`; `FailoverReason` | **nothing** — must stay model-free |
| **System 1 decisions** | cheap, calibrated, typed | **(the gap)** | **Laya** — classify/score/gate |
| System 2 work | generative, expensive | pi / Claude / LLM workers | nothing |
| state guardrails | transitions allowed at all | the static graph + `validate.rs` | nothing |

Laya occupies **exactly one row** — and it is the row pii has never had. The
literature also independently reproduces our own hard-won conclusion: the model
layer comes *after* the rules layer, never before it.

### 3.2 Graph driving — the answer is "the graph drives, Laya decides"

**The question "can Laya drive the graph?" should be answered no — and the
question contains the bug.** A probabilistic model that chooses *control flow*
reintroduces the injection failure mode doc 025 called structurally impossible in
a static graph. The correct shape, which our engine already supports:

```
        state ──► [ Laya node ] ──► $.payload{tier, confidence, ...}
                                        │
                     ┌──────────────────┴──────────────────┐
                     ▼                                     ▼
              route on $.payload.tier              route on confidence
              (cheap | mid | strong)               (< τ → escalate/human)
```

This is the **deterministic shell / probabilistic core** pattern realised in the
graph we already have: the shell is the statically-validated graph (edges fixed
at author time, capability-scoped, journaled); the core is Laya, which may
**only select among ports that already exist**. It cannot create an edge, so an
adversarial state cannot argue its way onto the cheap port — it can at most pick
the wrong *existing* port, which the confidence rule catches.

**And the engine already has the exact primitive this needs.** Doc 025 §4.2
established that a node must report *failure as payload*, not as a node error,
because "a graph cannot branch on a reason Pii never produces." Spec 40 shipped
that for provider failures (`FailurePayload{status,reason,provider}`). **A Laya
node needs the identical treatment**: its answer must land in `$.payload` so the
existing `route` node can branch on it unchanged. Nothing new is needed — the
shape is already proven by `graphs/starter/failover.toml`.

**Measured support for this shape:** on a real pii-shaped routing question
("which tier should implement spec 41?") Laya answered `strong` at **confidence
0.067** — it reported its own ignorance. A gate reading that confidence is
therefore *load-bearing*, not decorative (contrast Needle's N7, which emitted a
meaningless call at 0.996).

### 3.3 State machine — Laya is the transition guard, never the transition

The strongest research finding is that **state guardrails are a different layer
from output guardrails, and no model belongs in the guarantee.** So in a state
machine:

- **Laya may answer "may this transition fire?"** — a `Noul` (`is this input a
  prompt injection?`, `is this output consistent with the spec?`, `are these two
  memories the same fact?`) or a `Choice` (`which state is this ticket in?`).
- **Laya may NOT own the transition table.** The legal `source → destination`
  moves stay a static allow-list, exactly as the workflow-state-guardrail
  literature frames it: the guard is a *conditional check on a transition the
  machine already defines*, not a generator of new transitions.
- **Where this bites us concretely:** pii's journal is already an append-only
  state machine (`NodeStarted`/`Finished`/`Failed`/`Dead` + `seq`, replayable).
  Laya's natural fit is the **pre-transition check** on a *suspect* transition —
  the same place pidag's escalation rung lives today.
- **The failure to guard against:** a `Noul` guard is only as good as its
  calibration *on our distribution*, and the hostile-state caveat applies —
  text engineered to argue for its own classification can move the answer. That
  is why the deterministic guard must sit **first** (matching the dev.to finding
  that the rules layer was added *after* an LLM-only false negative).

### 3.4 Ranked, concrete applications (highest value first)

| # | where | Laya primitive | why now | effort |
|---|---|---|---|---|
| L1 | **pidag/pi escalation gate** — "is this task cheap-tier or strong-tier?" | `Choice`(tier) + `Score`(difficulty) + `Noul`(must-not-be-cheap) | pii/pidag already have an escalate-on-verdict rung; this adds the *pre-dispatch* signal the free→account→topup ladder lacks. Directly answers 025 §4.4 item 5. | 1 node + 1 route |
| L2 | **Guardrail on the agent loop** — injection / spec-drift / "does this tool call match intent?" | `Noul` ×N in one call | the literature's most-repeated pattern; ~0.3 s, $0, local | 1 node |
| L3 | **the workspace memory consolidation judgement** — "are these two memories the same fact?" / "is this worth keeping?" | `Noul` + `Score` | the live register defects (C22/C23/C31/C35) are all *judgement* failures currently done by an LLM with a too-small budget | already-shipped LLM seam |
| L4 | **Wake/skip triage** — "does this event need the expensive brain at all?" | `Noul` | the dev.to measured ~24M tokens skipped in 5 days on this exact idea | cron/MQTT front |
| L5 | **Doc/route classification** — "which doc section does this fact belong to?" | `Choice` | supports the SSOT/register discipline | low |

**Start with L1 or L2.** Both are one node, both are measurable against a
deterministic baseline, and L2 is the pattern with the most external evidence.

### 3.5 The five caveats that must travel with any of this

1. **Rules first, model second.** The rules layer is cheaper, deterministic and
   auditable. The dev.to case added it *because* the model-only version produced
   a false negative. Laya handles the ambiguous middle, never the hard rule.
2. **Confidence is the gate — `act_probability` is not.** Measured here: the
   `action.act_probability` head returned exactly `1.0` on every case including
   gibberish and an empty body (doc 039 §6.5). Gate on `confidence`.
3. **State quality, not language, is the main degrader.** A German case failed at
   conf 0.49 on a bare sentence and scored 0.9995 with a full envelope. Feed it
   the same rich state the vendor example uses.
4. **~0.3 s per decision on CPU — batch questions, don't call it per tool call.**
   4 questions cost ~2.3× one (doc 039 §6.2).
5. **Hostile state is unhandled.** Neither license nor locality fixes it; it is
   the caller's threat model and the reason the deterministic guard goes first.

---

## 4. Verdict

- **llama.cpp: yes**, as a published hybrid — backbone in llama.cpp
  (`modern-bert`, Q8_0), head in PyTorch, **Δp = 0.0006** verified here. On CPU
  it is *slower*, not faster; the win is format, quantised memory, one image for
  many models, and no per-model Python stack. **Use llama.cpp first for serving
  models** (now the host's layout), and measure before asserting a speed win.
- **Container: renamed to generic** (`localhost/llamacpp:local` /
  `llamacpp` / `llamacpp.container`), port 8082 and default model unchanged so pi
  is unaffected; Laya added as a **second instance of the same image** on 8083 in
  embeddings mode; model files share the existing read-only mount.
- **Graph: the graph drives, Laya decides.** A Laya answer must land in
  `$.payload` so the existing `route` node branches on it — the same shape spec 40
  proved for provider failures. Laya may only select among existing ports.
- **State machine: Laya is the transition guard, the table stays static.** No
  model in the guarantee.
- **Highest-value first step:** L1 (pre-dispatch tier decision) or L2 (agent-loop
  guardrail) — one node, one route, measurable, and L2 has the most external
  evidence behind it.

**Out of scope:** no pii code, spec, or graph change was made. Infrastructure
only (container rename + a Laya serving instance), plus these measurements.

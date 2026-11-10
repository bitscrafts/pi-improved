# 039 — Jev (TypeSafe AI) as a System-1 decision layer for Pii: analysis

**Date**: 2026-09-21
**Status**: ANALYSIS ONLY. No implementation, no graph/spec/`.rs` change.
**Trigger**: user — *"review our research on jav [Jev] and research the internet
on how it can be used on pii"* → *"no implementation, just analysis!"* →
*"also do it compare to the needle3 model!"* → *"also add to the report the Laya
(open source version of jev)"*.
**Prior art / inputs (in-repo)**
- `_docs/06-research/025-model-routing-and-failover-plan.md` — the routing +
  failover plan (Tracks A/B). **Jev is absent from it**; this doc supplies the
  missing decision-engine half.
- `_docs/06-research/016-ml-applications.md` (classical ML on Pii),
  `014-local-model-spec-implementation.md` (tiny chat models incl. routing).
- Landed code this analysis reads: `crates/pii-abi/src/failover.rs` (spec 37,
  `FailoverReason` + `Recovery`), spec 39 (structured provider failure), spec 40
  (`AgentNode` `fail` port + `FailurePayload{status,reason,provider}`), spec 41
  (`graphs/starter/failover.toml`), spec 42 (reason on the operator surface),
  `graphs/starter/verify-fix.toml` (escalation shape).
**Prior art (the workspace memory)**: `research/typeface-ai-and-jev`,
`insights/jev-system-one-models-primary`, `research/jev-herdr-model-router`,
`insights/jev-herdr-model-router`, `research/laya-system1-alternative-to-jev`,
`research/jev-console-url`, `insights/typeface-ai-and-jev`.

---

## 1. What Jev is (grounded)

Not an LLM. A **System One model**: you send *state* (JSON or text) plus a map
of **typed questions**; it returns typed answers in **one parallel pass**.
It generates **zero text** — it cannot ramble, and there is no parse step.

| | Jev | Frontier LLMs |
|---|---|---|
| Latency | 70–500 ms | 3 s–329 s |
| Input price | $0.042 / M tok | $0.20–$10 / M tok |
| Output price | **free** (no output stream exists) | ~5× input |
| Structured-output errors | **0% by construction** | 0.58%–45.5% |

Three primitives, and that is the entire surface:

| Primitive | Question | Returns |
|---|---|---|
| `Choice` | pick one of a list (**≤255** options) | `choice` + `probabilities` + `confidence` |
| `Score` | rate on an ordinal rubric (2–10 levels) | `score` (may land between levels) + `probabilities` + `confidence` |
| `Noul` | is this statement true? | `noul` ∈ [0,1] — *the number is the belief*, no separate confidence |

- **Endpoint**: `POST https://api.typesafe.ai/v1/systemone`, `Authorization:
  Bearer <key>`. Accessed directly (waitlisted early access), or via OpenRouter,
  Vercel AI Gateway, Netlify AI Gateway, AIMLAPI, or LiteLLM pass-through
  (`/typesafe/v1/systemone` — LiteLLM note: **no streaming**, own evaluate
  endpoint, **not** `/chat/completions`).
- **SDKs**: Python (`typesafe-sdk`) and JS/TS (`@typesafe-ai/sdk`). Community
  .NET SDK exists. The Python SDK's `@jev.fn` decorator turns a function
  signature into state+questions and its Pydantic return annotation into the
  question set.
- **Training**: RLCD (Reinforcement Learning for **Calibrated** Decisions) —
  the deliberate inverse of RLHF. Confidence is therefore *meaningful in
  aggregate* (higher confidence ⇒ higher accuracy), which is what makes
  threshold-based automation defensible. Built by Diogo Almeida (RLHF/InstructGPT
  co-inventor); $40M led by DCVC; announced 2026-09-15.
- **Answers are evaluated in parallel and in isolation** against the whole
  state, so adding questions barely moves latency and there is no context-rot
  between questions. Cookbook: a 13-question batch is **12.2× cheaper and 10.0×
  faster** than asking one at a time, with identical answers.

**The honest boundary (stated by TypeSafe itself):** this is *"not agents."*
Jev cannot generate text, code, explanations, or unbounded tool arguments. It
is a **checker/decider**, never the **maker**. It is useless where the space of
valid answers is not known up front, and it does not write the schema for you.

---

## 2. Common usages on the internet (what people actually ship)

TypeSafe's own use-case map plus independent write-ups converge on a small set:

| Usage | Source pattern name | Shape |
|---|---|---|
| **Intent routing** — classify user intent, route to the handler | *Intent Routing* | `Choice` over intents + code does the dispatch |
| **Confidence-gated routing** — different thresholds per action based on what being wrong costs | *Confidence-Gated Routing* | read `confidence`, compare per-action |
| **Composite scoring** — several dimensions combined by *your* formula | *Composite Scoring* | several `Score` questions, weights in code |
| **Speculative fan-out** — ask everything up front, let code decide what mattered | *Speculative Fan-Out* | one call, N questions, batch discount |
| **Model routing / harness engineering** — pick which LLM gets a prompt; escalate the hard ones | *Harness Engineering* | `Choice`(model) + `Score`(difficulty/risk) |
| **LLM guardrails** — semantic checks on every input, output, and tool call | *Universal Verification* | `Noul` probes, cheap per check |
| **Retrieve-then-judge** — fetch precisely, judge cheaply | *Retrieve, then judge* | retrieval shortlist → `Noul`/`Score` per hit |
| **Extraction from known candidates** — bounded, enum-shaped extraction | (docs) | `Choice` with an explicit `other` |
| **Search/reranking** — score query-to-candidate relevance, pairwise rerank | (docs) | `Score` per candidate |
| **Big-data map-reduce** — classify giant corpora / agent traces | (docs) | bulk `Choice`/`Noul` passes |

**Economics claim driving adoption**: 40–60% of a typical frontier token bill
is decision-shaped work, so moving that slice to Jev cuts the decision slice
~97% and the total bill ~30–70%. The widely-quoted worked example is
100 B tok/month at a $1.50 blended rate → ≈$65.6k/month saved.

**Counter-claims worth carrying (the research is not all promotional):**
- *"Do not migrate anything this week."* Shadow the checker call inside an
  existing loop, pin the version, keep the LLM path warm. The model is days old.
- Accuracy on TypeSafe's 4-workflow benchmark ≈**68%**, *"close to mid-tier
  LLMs like GPT-5.6 Terra"* — i.e. it buys speed and cost, **not** accuracy
  headroom. Benchmarks were built by the vendor and use an average of two
  frontier models as reference labels.
- **State is not treated as hostile.** Text engineered to argue for its own
  classification can move the answer. Anything user-controlled in `state` is
  the caller's threat model to handle.
- It is a *constrained decision model*, not a general language model
  (community pushback on over-generalisation).

---

## 3. Where this lands in Pii (the actual analysis)

Throughout this section "the engine" means **a System-1 decision model of this
class**, and §3.3 shows that class has a local, Apache-2.0 member (Laya) as well
as a cloud one (Jev) — so nothing below is a commitment to a vendor.

Pii already owns the two halves Jev is missing, which is the whole reason this
is a natural fit:

- **Pii has the maker.** `pii-model`'s `ModelProvider` (sync, `crates/pii-model/src/lib.rs:118`)
  with Ollama/llama.cpp providers, the agent node, and the graph engine.
- **Pii has the branchable substrate.** `route` (kind, `on` + `rules[{eq,port}]`),
  `transform`, `loop_budget`, `retry`, an append-only journal, and now a **typed**
  failure taxonomy (`FailoverReason`/`Recovery`, spec 37) that a `route` can read
  via spec 40's `FailurePayload{status,reason,provider}`.
- **Pii does NOT have a decision engine.** Today every branch that needs a
  *judgement* (not a comparison) must call an LLM — expensive, slow, and
  text-shaped. This is exactly the gap doc 025 identified when it said B4
  (tier-0 router) "is the only part of the article's playbook that needs a
  classifier."

Jev is a candidate for that classifier. It is **not** a replacement for any
existing piece.

### 3.1 Candidate placements (ranked by value/effort)

**J1 — System-1 router node behind `route` (the strongest fit).**
Today routing decisions are either static (`task_class` rules in the graph) or
delegated to an LLM. A Jev call gives a *calibrated* `Choice` + confidence that
becomes **`$.payload`**, which the existing `route` node branches on unchanged —
identical in shape to how spec 40 made provider failure branchable. The decision
stays inspectable via `pii graph show`, and because the graph is static and
statically validated, the injection failure mode is *still* structurally
impossible: Jev may only *select among ports that already exist*; it can never
create an edge. That is a materially safer shape than "an LLM decides where to
go", and it is the direct complement to doc 025 §4.4 items 1–3.
*Analysis only — no node kind, no provider, no wiring proposed for the build.*

**J2 — Verification/guardrail on the agent loop (cheapest real win).**
`Noul` is a checker primitive: "does this tool call match the stated intent?",
"does this output contradict the spec?", "is this a prompt-injection attempt?".
Doc 025 §4.4 item 2 already insists the verifier must be **deterministic and
free** — "do not evaluate answers with an expensive model (eats the savings)".
Jev is a third class of verifier: not deterministic like `exit_code`, but
~40–400× cheaper and 20–200× faster than an LLM judge. It fits the same
`gate`-then-`route` shape the `verify-fix` graph already uses.

**J3 — Escalation difficulty/risk scoring.**
Doc 025's Track B wants cheap→strong escalation and safety rules that protect
`spec_authoring`/`architecture`/`security` from the cheap tier. A `Score`
(difficulty) + `Noul` (must-not-be-cheap) pair, both from one parallel call,
feeds the existing `route`. Note this must stay a *preference order / ordering*,
never the sole authority — 025 §4.4 item 3 requires the non-negotiable branch to
be a **static rule that matches first**, so a model verdict can never reach the
cheap port. Jev slots *behind* that rule, not in front of it.

**J4 — Peak-window + provider selection refinement.**
025 §4.4 item 4 computes the peak window from config (deterministic, free) and
should stay deterministic — Jev adds nothing there. Where Jev *could* help is
the fuzzier adjacent question ("is this request actually equivalent, so the
cheaper provider is acceptable?"), but that is speculative; listed for
completeness, not recommended.

**J5 — Memory/consolidation judgement (the workspace memory-adjacent, not pii-core).**
`Score`/`Noul` over a candidate pair — are these two memories the same fact? is
this item worth keeping? — is a bounded decision and a direct fit. Relevant to
the consolidation defects on the register (C22/C23/C31/C35, whose root cause is
a small budget and an unparseable JSON body), but that work lives in **the workspace memory**,
so it is recorded here only as a cross-reference, not a pii proposal. Placement
by subject, per `ssot-doc-organization`.

### 3.2 What Jev must NOT be used for here

- **Not the maker.** No text, no code, no spec authoring. Spec authoring is
  planner-only and already a hard rule.
- **Not a replacement for `pii-abi`'s typed taxonomy.** `FailoverReason` is
  derived in code from real HTTP/provider facts; a probabilistic model must not
  sit in front of deterministic classification that is already correct and free.
- **Not an authority that can create control flow.** It selects among existing
  ports; the static graph remains the only source of edges.
- **Not a substitute for the deterministic gates.** cargo fmt/check/clippy/test
  and `validate-exit-criteria` stay exactly as they are.
- **Not a self-hosted assumption.** Jev is cloud-only and proprietary (unlike
  Laya, which is Apache-2.0 and self-hostable). Adopting Jev means **data
  egress** and a **network dependency** in the decision path — the exact thing
  Pii's local-first posture is built to avoid. This is the single biggest
  tension and it is not resolvable by engineering; see §4.

### 3.3 Laya — the open-source counterpart to Jev (same API, local, $0)

**Laya is the decisive half of this analysis, not a footnote.** `convaiinnovations/laya`
is an open (Apache-2.0) System-1 decision model that is *API-compatible with Jev*:
the same state + typed-questions contract, the same three question types
(`choice` / `score` / `noul`), the same calibrated-probability + confidence
returns. Anything written against the System-1 interface can target either one.

| | **Jev** (TypeSafe) | **Laya** (Convai Innovations) |
|---|---|---|
| Licence | proprietary, cloud API only | **Apache-2.0** (weights + code) |
| Host | vendor cloud | **self-hosted**: commodity GPU, Mac MPS, or **CPU** |
| Cost | $0.042 / M input tok (output free) | **$0 inference** |
| Data egress | **yes — state leaves the machine** | **zero** — local / air-gapped / on-device |
| API | state + typed questions | **same shape** (`laya.load(...)`, `agent.predict(state, questions)`) |
| Questions | `Choice` / `Score` / `Noul` | **identical three** |
| Training | RLCD | **RLCD** (same family) |
| Architecture | not published | ModernBERT-large with a `[MASK]` head |

Training detail worth recording (it is why the confidence is trustworthy rather
than decorative): Laya's policy emits a probability distribution; exploration
adds zero-mean Gaussian logit noise; the reward is a **strictly proper scoring
rule** (log + spherical + ranked probability score for the ordinal case). Max
expected reward is reached **only when the probabilities are truly calibrated**.
Multi-turn support uses temporal-difference learning with Monte Carlo targets
(TD(λ=1.0)) over prefix slices. It claims 100% human-annotated real datasets and
zero synthetic shortcuts.

**Self-reported benchmark vs *published* Jev figures** (treat as directional —
see the caveat immediately below):

| | Laya (self-reported) | Jev (published) |
|---|---|---|
| Latency P50 | **~38.4 ms** (p95 42.1) | ~400 ms avg |
| 10 batched questions | 156 ms | ~1500 ms serial |
| In-task accuracy | **83.8%** | 67.8% |
| Routing / moderation | 99.1% macro (ECE 0.009) / 96.7% (ECE 0.061) | — |
| Email triage / selective automation | 73.2% / 92.2% @50% coverage | — |

**Honest caveats (do not skip):** these figures are **self-reported by Convai
Innovations against Jev's published numbers**, and Jev's own 193×/444× figures
are vendor-claimed by TypeSafe. **Both sides are vendor claims** — treat every
number above as directional, not verified. The repos do ship `eval/`, so the
real test is running Laya locally and computing our own ECE / coverage curves,
which is exactly the "proven, not assumed" rule this workspace already applies
to free/local models.

**Why Laya is the linchpin of the recommendation.** It changes the question from
"do we adopt a cloud vendor for decision-making?" into "do we adopt a *class*
of model?". Three consequences:

1. **It is the local-first fit.** Jev's one irreducible cost is data egress plus
   a network dependency *inside the decision path* — the thing Pii's posture
   exists to avoid. Laya removes both while keeping the same interface, which is
   the same move already made with nvidia→ollama.
2. **It de-risks the vendor.** Writing against the System-1 interface and
   defaulting to Laya means a Jev outage, price change, or waitlist refusal
   never blocks the design — Jev becomes swappable, not load-bearing.
3. **It is testable at $0, today.** A shadow evaluation costs nothing, needs no
   key, and can run on a spare port against a real Pii decision — which is
   precisely the evidence currently missing (§ bottom line).

So the real decision is **not "Jev yes/no"** but **"which decision engine"** —
and it maps onto the workspace's existing free→account→topup ladder exactly:
**Laya as the default (local, $0, no egress), Jev as the cloud cross-check.**
That is the same conclusion the earlier `research/jev-herdr-model-router` +
`insights/laya-system1-alternative-to-jev` work reached independently, and it is
consistent with the user's standing preference for the free/local path first.

**Caveat that survives regardless of engine:** both carry the same
`state is not treated as hostile` exposure — text engineered to argue for its
own classification can move the answer. Neither license nor locality fixes that;
it is handled at the caller (deterministic checks, §4 F7).

### 3.4 Integration-shape notes (if it is ever built)

- Jev is **sync-callable over HTTP**, which matches `ModelProvider`'s deliberate
  sync design — but it is **not** a chat provider: it has no `/chat/completions`,
  no streaming, and returns typed values, not a transcript. Modelling it *as* a
  `ModelProvider` would be a category error. Its natural seam is closer to a
  **tool** node (a call returning structured payload) than a model node.
- Everything it returns must land in **`$.payload`** so `route` can branch on it
  — the same "failure as payload, not as node error" insight that made spec 40
  necessary. A Jev answer that arrives as a node `Err` would be unusable.
- Its question set is **schema-shaped and static**. That is a feature (no
  parsing, no prompt drift) and a constraint (new questions = a code change).
- **Cost/observability**: input-priced, output-free. `ChatResponse` already
  carries `tokens_in`/`micro_usd`; a Jev call's accounting would be trivially
  representable, but the existing field names are chat-shaped.

---

## 3.5 Jev vs Needle 3 — two different decision classes, not two candidates

Doc `034-cactus-needle3-automation-research.md` already analysed Needle 3 as a
possible pii router (and it is register row **B6**: *"Needle 3 as a local
router node"*). These are **not** competing answers to the same question, and
conflating them would produce a wrong design. Both are non-chatting, on the
"decision" side of the LLM divide — but they solve *different* halves of it.

| | **Jev** (TypeSafe) | **Needle 3** (Cactus) |
|---|---|---|
| Class | System One **decision** model (RLCD) | Open **automation foundation** model |
| Job | score/choose/decide over a **given** state | map free text → **tool call** + fill args; extract; embed |
| Primitives | `Choice`(≤255) / `Score`(2–10) / `Noul` | declared function schema; grammar-constrained decode |
| Output guarantee | **0%** structured-output error *by construction* (schema-defined answers) | grammar makes invalid values **unrepresentable** |
| Where the boundary is defined | by the **caller's typed question set** | by the **declared tool schema** |
| Confidence | calibrated, meaningful in aggregate (RLCD) — the core selling point | calibrated head, **min of two signals**; withheld < 0.1 |
| Size / host | cloud-only API, 70–500 ms | **29–121M params, 8–29 MB**, runs on CPU / Pi 5 / MCU |
| Cost / egress | $0.042/M in, output free — **but data leaves the machine** | **$0, zero egress**, Apache-2.0 |
| Maturity | days old (2026-09-15), early access, waitlist | 2026-09-17; **independently measured** in-repo (Colab notebook) |
| Measured weak spot | ~68% on vendor's own benchmark — mid-tier-LLM parity | BFCL v4 **50.2**; classification **1/5** (measured); confidence routing **failed** on vague input (locked a door at 0.989–1.000) |

**The distinction that matters for pii:**

- **Needle answers "what should I DO?"** — text → a structured *action*
  (tool call with grounded arguments), or refusal. It is a *dispatch/assembly*
  layer. Its grammar makes malformed calls impossible, but the notebook's N7
  proved the grammar is satisfied by a *meaningless* call too: `lock_door(door="the thing")`
  at 0.996 confidence. So Needle is strong at **shape** and unproven at
  **judgement**.
- **Jev answers "what IS this?"** — state → a typed *judgement*
  (`billing` vs `technical`, urgency 1–10, is-this-injection true/false). It
  never emits an action, so it cannot execute anything; but its answer is a
  *classification with calibrated confidence*, which is exactly what a `route`
  node needs to select a port.

**Consequence — they are complementary and both belong behind the same seam:**

| pii decision the graph needs today | which one |
|---|---|
| "which port/model should this take?" (intent, difficulty, risk) | **Jev** (`Choice`/`Score`) — 025 §4.4 item 1/5 |
| "is this input/output/tool-call acceptable?" (guardrail) | **Jev** (`Noul`) — or Needle's refusal, if the check is call-shaped |
| "turn this request into a call on an existing tool" | **Needle** (grammar + grounded args) — 034 item 3 |
| "what type is this document / which enum?" | contested: Jev `Choice` (typed, calibrated) vs **Needle measured 1/5** — Jev is the stronger candidate for pure classification |
| "is this call *meaningful*?" (the N7 hole) | **neither** — needs a deterministic check (034 item 2's conclusion stands) |

So the earlier note that 034's Needle router and 025's tier-0 router were
"probably the same shape of decision" resolves as: **the seam is the same (a
decision source feeding `route`), the decision classes are different.** Jev is
the *classifier/scorer*; Needle is the *dispatcher*. A pii graph could use both,
and neither replaces the deterministic gates.

**Cost/egress tiebreak is decisive and unambiguous, and it is the same as
Laya's:** Needle is local, $0, zero-egress and independently measured; Jev is
cloud-only. On the workspace's free→account→topup ladder, that means Needle
**and** Laya sit on the free/local rung and Jev sits on the paid cross-check
rung — for *different* jobs. Register B6 does not change; §3.3's F2 conclusion
does not change; what §3.5 adds is that **Jev is not a substitute for Needle 3
and must not be evaluated as one.**

The one honest caveat shared by both: **neither has been measured on pii's own
decisions.** 034's closing open question — *"worth a spike measuring Needle 3 on
our actual tool schemas before any design commitment"* — applies verbatim to Jev
on our actual routing questions. Neither should be adopted on vendor benchmarks
alone.

---

## 4. Decision forks (chosen vs discarded, per user's standing instruction)

| # | fork | options | recommendation | why the others lose |
|---|---|---|---|---|
| F1 | Is there a decision-engine gap in Pii at all? | (a) yes, build one; (b) no, keep LLM-judges | **(a) yes** — 025 already identified it (B4 needs a classifier) | (b) leaves every judgement on the expensive path and contradicts 025's own "do not judge with an expensive model" |
| F2 | Which engine? | (a) Jev cloud; (b) Laya local; (c) both, Laya default + Jev cross-check; (d) defer | **(c) both, Laya default** | (a) imports egress + a network dependency into the decision path; (b) forfeits the cheap cross-check the user already holds a dev key for; (d) ignores a live gap |
| F2b | Does Jev **replace** Needle 3 (register B6)? | (a) yes, one router engine; (b) no — different decision classes, use both | **(b) no** | (a) discards a local, zero-egress, *independently measured* dispatcher in favour of a cloud classifier that cannot emit tool calls at all (§3.5) |
| F3 | Where does it sit? | (a) new node kind; (b) a `tool`-shaped call feeding `route`; (c) a `ModelProvider` impl; (d) inside `pii-model` | **(b)**, with (a) explicitly *not* required | (c) is a category error (no chat, no stream, no text); (a) is new machinery for something composition already expresses |
| F4 | Can it create control flow? | (a) yes, dynamic edges; (b) no, select among existing ports only | **(b) no** | (a) destroys the property 025 §4.4.3 prizes — that injection cannot reach the cheap port because the edge is static |
| F5 | Does it replace the typed failover taxonomy? | (a) yes; (b) no, it sits beside it | **(b) no** | (a) replaces a free, correct, deterministic classifier with a probabilistic one |
| F6 | Timing | (a) build now; (b) shadow-test only; (c) analysis only (this doc) | **(c) analysis now, (b) shadow next** | (a) bet production on a days-old model whose own reviewers say "do not migrate anything this week"; the vendor benchmark is 68%, i.e. parity with mid-tier LLMs, not superiority |
| F7 | Trust the confidence score as a safety gate? | (a) yes, threshold and act; (b) no — shape only, safety needs a deterministic check | **(b) no** | (a) is directly refuted by two independent measurements: Needle's N7 (door locked at 0.989–1.000 on "handle it") and Jev's own docs warning that *state is not treated as hostile* — text engineered to argue for its own classification can move the answer |

**Discarded and recorded (not silently dropped):** dynamic-edge routing via a
model (F4a); `ModelProvider`-shaped Jev (F3c); Jev as the maker for spec
authoring (hard rule violation); Jev in front of the peak-window computation
(025 §4.4.4 keeps it deterministic); self-hosting assumption for Jev (it is
cloud-only); Jev as the *only* engine with no local counterpart (F2a).

---

## 5. Bottom line

**The finding is architectural, not promotional.** Pii's graph already knows how
to branch; doc 025 already specified the failure-as-payload change that makes
branching possible; what Pii has never had is a **cheap, typed, calibrated
decision source** for the branches that need judgement rather than comparison.
Jev is the first credible shipped instance of that class, and the internet's
common usages (intent routing, confidence-gated routing, guardrails, harness
model-routing, retrieve-then-judge, bounded extraction) are almost a checklist
of the decisions Pii currently delegates to an LLM.

The two constraints that decide the shape:

1. **Local-first.** Jev is cloud-only and proprietary; Laya is Apache-2.0 and
   runs on CPU. The free→account→topup ladder says: **Laya default, Jev as the
   paid cross-check** — the same conclusion the earlier herdr/model-router
   research reached independently.
2. **The graph stays the authority.** A System-1 model may only *choose among
   ports that already exist*; it never creates an edge. That preserves the one
   property doc 025 called structurally valuable, and it is why the correct
   integration shape is a decision **feeding `route`**, not a new router.

**And it is not a Needle 3 replacement (§3.5).** The two models answer different
questions — Needle *dispatches* (text → grounded tool call, locally, $0, grammar
-enforced) while Jev *judges* (state → typed classification with calibrated
confidence). Register B6 stands unchanged; both belong behind the same `route`
seam, for different decision classes. Neither should be adopted on vendor
benchmarks: 034 measured Needle's classification at 1/5 and its confidence
routing locking a door at 1.000 on "handle it", and Jev's own docs warn its state
is not treated as hostile.

**Next step (a decision for the user, not for an agent):** whether to (i) open a
shadow-evaluation task for Laya on a spare port against a real Pii decision
(recommended, zero cost, no egress), (ii) request/keep a Jev key for
cross-checking, or (iii) defer entirely. Registered as a new row in the
workspace OPEN-REGISTER.

**Explicitly out of scope for this doc:** any implementation, spec, node kind,
provider impl, or `.rs` change.

**Tracked in:** `<workspace>/docs/01-overview/OPEN-REGISTER.md` row **A16**
(AWAITING a user decision) — per the `open-item-tracking` procedure, so the
next-step fork is findable and does not rot with this session.

---

## 6. MEASURED — Laya downloaded and run for real (2026-09-21)

The shared caveat above ("neither has been measured on pii's own decisions")
was closed for Laya by actually downloading and running it in this container.
**Everything in this section is measured here, not quoted from a vendor.**

**Environment:** container `the host`, **no GPU** (`torch.cuda.is_available()` →
`False`), 32 vCPU, 125 GB RAM. This is the CPU path — the worst case for a
421M-parameter encoder, and the honest one to report.

**Setup:** `uv venv` (CPython 3.12.14) → `uv pip install laya` → **laya 0.3.4**,
torch 2.14.0, transformers 5.17.0. Install succeeded (no packaging defects, unlike
Needle 3's documented missing `pydantic`/`numpy`/`sentencepiece`). Disk: venv
5.4 GB, HF cache 2.8 GB. Scripts kept at `<scratch>/laya-test/`.

### 6.1 The documented quickstart runs and gives the documented answers

`laya.load("convaiinnovations/laya")` + the email-triage example from the model
card produced: **department `billing` (confidence 0.89)**, urgency **1.51/2**,
churn risk **43.1%**, phishing **3.1%** — matching the card's shape. It really
does return typed answers with **`output_tokens: 0`**: there is no generation
step to hallucinate in.

### 6.2 Latency, measured (CPU, no GPU)

| what | measured | vendor claim (T4 GPU) |
|---|---|---|
| cold load (first build) | **23.0 s** | seconds |
| 1 question | **140 ms** | 39.5 ms |
| 4 questions, one call | **328 ms** | — |
| steady median (4 questions, 6 runs) | **307.9 ms** (min 305.9, max 324.9) | 158.6 ms for 10 on GPU |
| `Router(preload=True)` startup | **63.4 s** (3 checkpoints) | — |
| Router, 3-4 questions | **83–365 ms** | 32.8 ms |

**Reading:** the claims are directionally right but GPU-flattered. On CPU this is
a **~0.3 s decision**, not a 40 ms one — still ~10-25× faster than a frontier LLM
call and ~1000× cheaper, but it is *not* free-latency, and a graph node calling it
per step would notice. Batching is real and cheap: 4 questions cost barely more
than 1.

### 6.3 The headline measured result — confidence tracks correctness

A 4-case refund spot-check with known labels: **4/4 correct**, and the confidence
separated cleanly — `noul=0.948` (conf 0.948) and `noul=0.958` (conf 0.958) on the
two genuine refund requests vs `0.442` and `0.035` on the two negatives.
Calibration is *observable in a handful of calls*, which is the property that
makes threshold gating meaningful.

**And it fails the way it says it fails.** The pii-shaped routing question
("which tier should implement spec 41?") answered `strong` at **confidence 0.067**
with the distribution `cheap 0.18 / mid 0.33 / strong 0.49` — i.e. it correctly
reported that it did not know. That is the opposite of Needle's measured N7
failure (a *meaningless* call emitted at 0.996 confidence), and it is the single
most important contrast between the two systems.

### 6.4 Language handling — the Router works, and the caveat is subtler than documented

The docs warn the English checkpoint collapses on non-Latin scripts (Khmer at
**0.000 accuracy / 0.952 confidence**) and that the `Router` fixes it by detecting
script in <0.5 ms. Measured here:

- Script detection works: Portuguese routed to `english`, a Devanagari-shaped case
  would go to `multilingual`, and the routing metadata explains *why*.
- **A German case "failed" (`dept=other`, confidence 0.49) and the cause was NOT
  language.** With the *same* short German sentence the model was unsure; with a
  richer state (from/subject/body, as the docs' own example uses) the identical
  German request scored **billing at 0.9995 confidence**. So thin state, not
  foreign language, is what degrades it — a **state-quality** caveat that the
  vendor material does not state as clearly as the script caveat.
- Portuguese with a full envelope → `billing`, confidence 0.82-0.89.
- One residual oddity: routing metadata picked `english` for the Portuguese case
  while still answering correctly; the multilingual checkpoint answered a second
  Portuguese case at **0.99 confidence in 90 ms** (the fastest call measured).

### 6.5 A real finding — the `action` head is inert

Every answer carries `action.act_probability`, and across all cases measured
(meaningful, irrelevant, **gibberish `"asdkjh qweoiu zxcvb"`, and an empty
`body: ""`) it returned **exactly 1.0**. Read against the installed source
(`laya/agent.py:310`: `ext = {"act_probability": round(float(act[r, 0]), 4)}`,
a raw softmax over 2 logits), this is a separate head that is not trained to
discriminate on this checkpoint.

**Consequence for the integration design:** do **not** gate on `act_probability`
— in the installed build it carries no signal. Gate on **`confidence`**, which is
derived from the probability distribution's shape and *did* vary meaningfully
(1.000 → 0.067) in the measurements above. This is a concrete, measured refinement
of §4 F7: "adopt the shape, not the score" now has a named field to trust and a
named field to ignore.

### 6.6 What this changes in the analysis

- **§3.3's core claim is now evidence-backed:** Laya runs locally on CPU, $0,
  zero egress, returns calibrated typed answers, and **correctly reports its own
  uncertainty** — the property Jev is sold on, confirmed on our own hardware.
- **§4 F2's "testable at $0 today" is done, not proposed.** The remaining work is
  an *evaluation*, not an installation: run Laya against real pii decisions
  (a routing/tier judgement, a guardrail `Noul`, a consolidation "same fact?"
  `Noul`) and compute our own ECE / risk-coverage curves from its `eval/` shape.
- **A new cost constraint for the design:** at ~0.3 s/decision on CPU it is a good
  *per-task* gate and a poor *per-tool-call* gate. Batch questions (4 questions
  cost ~2.3× one) rather than calling it repeatedly.
- **Still unmeasured:** accuracy on *pii's* decision distribution, and behaviour
  under the hostile-state condition (§4 F7) — a state engineered to argue for its
  own classification has not been tested here.

**Nothing in this section changes the recommendation's shape** (§5): Laya on the
free/local rung, Jev on the paid cross-check rung, the graph still the sole
authority over edges. It upgrades the recommendation from *reasoned* to *measured*,
and it names the one field (`confidence`) and the one anti-field
(`act_probability`) the integration must use.

**Reproduce:** `<scratch>/laya-test/` — `run_laya.py` (quickstart),
`bench_laya.py` (latency + calibration), `router_laya.py` (language routing),
`de_check.py` (thin-vs-rich state), `action_check.py` (`act_probability` head).

---

## 7. llama.cpp and integration shape — see doc 040

**Follow-up (2026-09-21):** `040-laya-on-llamacpp-and-graph-driving.md` answers
the two questions this doc leaves open and was requested alongside it:

1. **"Does it run from llama.cpp?"** — YES, as a published **hybrid**: the
   ModernBERT backbone in llama.cpp as `modern-bert` (Q8_0), the decision head in
   PyTorch. Verified here end-to-end with **Δp = 0.0006** against pure PyTorch.
   Important honest caveat measured there: on CPU the hybrid is **slower**, not
   faster (HTTP round-trip per backbone call) — the win is format, quantised
   memory and one-image-many-models, not speed. The host's llama.cpp setup was
   also renamed to be generic (it was named after laguna, which is only a model),
   and Laya now runs as a second instance on 8083.
2. **"How can it improve our projects — graph driving? state machine?"** — the
   answer to both is the same principle, now with external literature behind it
   (deterministic shells / probabilistic cores; rules-before-model; state
   guardrails as a *separate* layer with no model in the guarantee):
   **the graph drives and Laya decides** (its answer lands in `$.payload` so the
   existing `route` node branches on it — the exact shape spec 40 proved for
   provider failures), and **Laya is a transition guard that never owns the
   transition table**. Ranked applications L1–L5 and the five caveats are in 040.

This section 6 is the *measurement* half for Laya; 040 is the *deployment and
integration* half.

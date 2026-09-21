# 025 — Model routing + provider failover in Pii: plan

**Date**: 2026-09-11
**Status**: PLAN (design + spec breakdown). No implementation.
**Trigger**: user directive — "how Hermes deals with provider/model switching when
it became exhausted … this model must be followed in pii", plus the ByteByteGo
routing research already in agent-memory
(`research/model-routing-costs`, `insights/model-routing-maps-onto-our-chain`).

**Prior art / inputs**
- Hermes source (local): `agent/error_classifier.py` (2187 lines),
  `agent/chat_completion_helpers.py`, `agent/auxiliary_client.py`,
  `tests/run_agent/test_provider_fallback.py`.
- Article: <https://blog.bytebytego.com/p/how-smart-model-routing-can-cut-llm>
  (ByteByteGo, 2026-09-09).
- Pii: `crates/pii-abi/src/error.rs`, `crates/pii-model/src/lib.rs`.
- Cost windows: `/projects/docs/03-reference/deepseek-pricing.md` and
  `/projects/docs/03-reference/ollama-pricing.md` (workspace-level, SSOT).

---

## 1. Two capabilities, not one

The directive conflates two mechanisms that must be designed separately, because
they optimise different things and fail differently:

| | **A. Failover** | **B. Smart routing** |
|---|---|---|
| Optimises | resilience / availability | cost |
| Trigger | an error at call time | the request's difficulty, before the call |
| Question answered | "this provider just failed — who else can serve it?" | "which model is the cheapest that can do this?" |
| Failure mode | chain burns through every provider on a non-provider error | router misjudges difficulty → quality drop |
| Hermes has it | **yes**, mature | partially (fallback chain doubles as a cheap-tier ladder) |

Pii has **neither** today. The shell harness has a coarse version of both
(`ORCH_CHAIN=empero ollama deepseek`, cooldowns), but that lives in bash around
`pi`, not inside Pii — so nothing in Pii's own runtime benefits.

**Both are first-class goals of equal weight.** Routing is *not* a follow-on to
failover: failover keeps us alive when something breaks, routing decides whether
we should be paying that price at all. They share one substrate (the ordered
provider chain + the reason taxonomy) but optimise different things, and routing
has the larger steady-state payoff — failover only pays out on failures, routing
pays out on *every request*. Treat B as co-equal in scope and priority, not as
"phase 2".

---

## 2. What Hermes does (the model to follow)

### 2.1 A typed error taxonomy — the core idea

`agent/error_classifier.py` classifies every API failure into a `FailoverReason`
(≈16 reasons), and **the reason decides the recovery strategy**:

```
auth                 -> refresh/rotate credentials
auth_permanent       -> abort (refresh already failed)
billing              -> rotate immediately (402 / confirmed credit exhaustion)
rate_limit           -> backoff, then rotate
upstream_rate_limit  -> switch MODEL, not credentials (aggregator 429; our key is fine)
overloaded           -> 503/529 backoff
server_error         -> 500/502 retry
timeout              -> rebuild client + retry
ssl_cert_verification-> fail fast (deterministic; retrying is waste)
context_overflow     -> COMPRESS, do not failover
payload_too_large    -> COMPRESS
image_too_large      -> shrink and retry
model_not_found      -> fall over to the next entry
unknown              -> fall over
```

**The load-bearing insight**: *not every error should advance the provider
chain.* A context overflow or a 413 is a property of the **request**, not the
provider — failing over to another provider reproduces the identical failure
and burns the whole chain for nothing. Hermes encodes this by giving each reason
its own strategy rather than a boolean "retryable".

The second insight is the `rate_limit` vs `upstream_rate_limit` split: a 429 from
an *aggregator* means your key is healthy and the upstream model is throttled, so
the correct move is a different **model**; a 429 from the provider itself means
rotate credentials. Getting these two confused either rotates good keys or
retries a throttled model forever.

### 2.2 The chain

- Config: ordered `fallback_providers` list (provider, model, base_url, api_key,
  api_mode). Invalid entries are filtered at construction.
- Runtime: `_fallback_chain` + `_fallback_index` on the agent; advancement walks
  the list in order.
- Reason text is surfaced to the operator (`_fallback_reason_text`) so a silent
  switch is never silent — you always learn *why* it moved.
- Credential rotation happens *inside* a provider before leaving it
  (`_pool_may_recover_from_rate_limit`), so a second API key is tried before the
  whole provider is abandoned.
- A distinct **payment** fallback path exists for billing failures.

### 2.3 What the article adds on top (for B)

- **Cascading**: try the cheap model, verify the result, escalate only on a
  failed check. Requires a *cheap, deterministic* verifier — running an
  expensive model to judge every answer eats the savings.
- **Small-model router**: classify difficulty cheaply; must be combined with
  fixed **safety rules** for task classes that always go to the strong model.
- **Semantic routing**: good for intent, weak for reasoning-difficulty.
- **Learned routing**: needs volume; not for us yet.
- **Ten-x savings are conditional**: large price gap, mostly-simple requests, a
  router that reliably identifies them. If any is absent, the savings vanish.
- **Failure mode that matters most for Pii**: *prompt-injection of routing
  instructions.* If the router reads difficulty hints out of untrusted text, a
  spec or a dependency README can argue itself onto the cheap model. Routing
  must use trusted metadata + app rules, never prompt/spec text.

---

## 3. Gap analysis — Pii today

| capability | Hermes | Pii today |
|---|---|---|
| error taxonomy | ~16 typed reasons, each with a strategy | `PiiError` 8 variants; `is_retryable()` = `Timeout \| Unavailable`, `is_terminal()` = 4 variants — a **2-way split** |
| provider chain | ordered list + index, filtered at build | none — `provider_from_spec()` returns a single `Arc<dyn ModelProvider>` |
| per-reason recovery | explicit (rotate / backoff / compress / abort / failover) | implicit: caller decides, nowhere centralised |
| credential rotation | pools, rotate before leaving provider | none (single key per provider) |
| reason surfaced to operator | yes, operator-friendly text | no |
| routing by difficulty | partial (chain as cheap-tier ladder) | none in-process; coarse `ORCH_CHAIN` in the bash harness |
| peak/off-peak awareness | n/a | none (and Ollama/DeepSeek have *different* peak windows) |
| primitives for chain / router / observability | n/a (not a graph engine) | **already have them** — `route`, joins, `loop_budget`, `retry`, journal, per-node caps (`crates/pii-graph`) |

Relevant Pii anchors:
- `crates/pii-abi/src/error.rs` — `PiiError` + `is_retryable` / `is_terminal`.
  Note the doc comment on `Unavailable`: *"Worker death and HTTP 429/5xx are its
  producers"* — the raw material for A is already flowing, it just has nowhere
  to be classified.
- `crates/pii-model/src/lib.rs` — `ModelProvider` trait (`chat`,
  `chat_streaming`), `provider_from_spec` (`scripted:`/`llamacpp:`/`ollama:`),
  `resolve_with_transport` (the single construction entry point a chain would
  replace or wrap).
- `crates/pii-model/src/ollama.rs` — already maps `429 || >=500` to a transient
  error at two sites. That mapping is the only current "classification".

---

## 4. Design for Pii — graph-native

### 4.1 The engine already has the primitives (verified against source)

Pii is graph-first, so neither the chain nor the router should be a new opaque
Rust struct bolted onto the model layer. Almost everything this plan needs is
**already in `crates/pii-graph`** — the one genuine gap is the classification
that lets a graph branch on a failure.

| concept needed | Pii primitive | anchor |
|---|---|---|
| ordered provider chain | nodes + typed edges `Edge { from, port, to, kind }` | `crates/pii-graph/src/spec.rs` |
| **branch on a failure reason** | `route` node: `on` expr + `rules = [{eq, port}]`; `eq` is a `serde_json::Value`, so **string equality works** | `crates/pii-graph/src/nodes.rs` |
| retry the same provider | `NodeSpec.retry` (`RetryPolicy`) | `spec.rs` |
| bound the ladder | `loop_budget = N` on a cycle member | `sched/loop_helpers.rs` |
| escalate cheap → strong | the `verify-fix` shape: model → gate → `route` on `$.payload.exit_code` | `graphs/starter/verify-fix.toml` |
| decision input | tiny expr language, `$.path.member` access only | `crates/pii-graph/src/expr.rs` |
| observability | append-only journal: `NodeStarted`/`Finished`/`Failed`/`Dead` + `seq`, replayable | `crates/pii-graph/src/journal.rs` |
| per-hop authority | three-layer cap intersection (run ∩ graph ∩ node) | `validate.rs`, `sched/dispatch.rs` |
| per-hop budget | `Limits { max_steps, max_wall_ms, max_tokens, max_parallel }` | `spec.rs` |

Consequence: **B1 (deterministic escalation) and B2 (safety rules) are already
expressible as graph shapes today** — `verify-fix.toml` is a working escalation
ladder with a loop budget.

### 4.2 The one genuine gap: the failure must be *branchable*

`route` can compare any JSON value, but it can only branch on something that
**appears in the payload**. Today a node that hits a provider error returns
`Err`; the scheduler marks it failed and its downstream **dead** (the run
fails) — so there is nothing for a `route` to read. The load-bearing change is
therefore twofold:

1. **A1 — classify** (`FailoverReason` + `classify()`), because a graph cannot
   branch on a reason Pii never produces. This is the linchpin; everything else
   is composition of existing parts.
2. **Surface the failure as payload, not as a node error.** The model-calling
   node must return a *successful* `Outcome` carrying e.g.
   `{ "status": "error", "reason": "rate_limit", "provider": "ollama-cloud" }`
   so a `route` on `$.payload.reason` can select the next hop. This mirrors what
   `gate` already does with `$.payload.exit_code`.

**This — not the chain — is the real implementation work.**

### 4.3 A — failover as a graph

```
model_a --(out)--> route_reason --(rate_limit|server_error)--> model_b --(out)--> route_reason2 --(ok)--> done
                                   (ok)--> done                                        (fallback)--> fail
```

- Sequential edges, one node per provider entry ⇒ **no wasted spend**.
- Explicitly **not** `JoinPolicy::Any { cancel_losers }`: that is a *parallel
  race* — correct for "first of N wins" latency work, but **wrong for provider
  failover**, because every attempt bills before the losers are cancelled.
  Failover must be sequential.
- `route` rules carry the reason strings (`eq = "rate_limit"`), with the last
  rule as a catch-all fallback (no `eq`).
- `ContextOverflow` / `PayloadTooLarge` route to a **compress** branch, never to
  the next provider.
- `NodeSpec.retry` handles same-provider retry/backoff *before* the ladder moves
  on, so "retry" and "failover" stay distinguishable — the same separation
  Hermes makes.
- Observability is free: the journal already records which hop ran, which
  failed, and the sequence; the reason rides in the payload.
- Accepted trade-off: a 5-entry ladder is ~10 nodes of TOML — verbose, but
  *visible*, per-hop capability-scoped, and journaled. A model node's `cfg` may
  optionally carry a compact `chain = [...]` for the purely mechanical case
  (failover only, identical caps, no policy variation). Any ladder with real
  policy — compress branches, different tools, per-hop caps — belongs in the
  graph, not hidden in a cfg blob.

### 4.4 B — routing as a graph

1. **The router is the existing `route` node.** Difficulty / peak / safety
   decisions are `rules` over a `transform`-computed value; each port wires to a
   different model node. No new "router" abstraction is needed, and the decision
   stays inspectable with `pii graph show`.
2. **Escalation is the `verify-fix` pattern — already built and verified.**
   `pii graph show graphs/starter/verify-fix.toml` renders exactly this ladder:

   ```
   fix [agent] (entry) (loop_budget=3)
    |-- ok --> test

   test [tool: bash]
    |-- ok --> gate

   gate [route: $.payload.exit_code]
    |-- pass --> done
    |-- fail --> retry

   retry [transform: $.payload]
    ^-- loops back to "fix" (budget=3)
   ```

   The verifier is a **`tool` node running the tests** plus a **`route` node
   branching on `$.payload.exit_code`** — deterministic and free, which is
   exactly the article's "do not judge with an expensive model". Note the
   overloading: that node is *named* `gate` but its **kind is `route`**; the
   server-registered `gate` kind is the separate human-approval gate.
   Crucially, this proves §4.2: routing works here **because the `tool` node
   reports `exit_code` as payload** rather than failing the node. The model node
   needs the identical treatment for `reason`.
3. **Safety rules are rule ORDER, plus config.** The graph is statically
   validated, so the non-negotiable branch is a rule that matches first
   (`task_class in {spec_authoring, architecture, security}` → strong-model
   port). Because that is a **static graph edge rather than a model's
   judgement, no text inside a spec or a repo README can argue its way onto the
   cheap port** — the article's injection failure mode becomes *structurally
   impossible* here, not merely discouraged.
4. **Peak-aware provider choice** (verified 2026-09-11): `deepseek-v4.1-flash`
   costs *identically* on Ollama Cloud and DeepSeek direct (0.15/0.003/0.60
   off-peak; 0.30/0.006/1.20 peak) — only the **windows** differ (DeepSeek
   01–04 & 06–10 UTC, Ollama 12–18 UTC, Mon–Fri). Two equal-price entries for
   the same model therefore take a `route` rule on the current window,
   preferring whichever is off-peak. A `transform` node computes the window from
   config (never hardcoded); the fallback rule keeps the peak provider usable
   when nothing else is available.
5. **Tier-0 router = an `agent` node on the local model.** Pii already runs
   granite in-process (spec 35/36, `llama-cpp-2`, real tool calls). As a graph
   node it is a *free* classifier feeding the `route` node — the article's
   "small model as router" at zero marginal cost. Defer until 1–4 are proven.
6. **Learned routing**: out of scope; needs request volume Pii does not have.

### 4.5 What NOT to do

- Do not failover on `ContextOverflow` / `PayloadTooLarge` — route to compress.
- Do not use `Any { cancel_losers }` for failover — it is a parallel race that
  bills every attempt.
- Do not let the router read difficulty from prompt/spec text (injection).
- Do not evaluate answers with an expensive model (eats the savings).
- Do not switch hops silently — the reason goes in the payload and the journal.
- Do not hide a policy-bearing ladder inside a `cfg` blob; put it in the graph.
- Do not route spec authoring to the cheap tier (existing hard rule).

---

## 5. Spec breakdown (for the PLANNER to author)

`specs/` is planner-only; this section is a *brief for the planner*, not specs.
Two tracks, both first-class. Track A builds the substrate; Track B is the
routing layer and is co-equal in priority — B3 in particular is the best
value-per-effort item in the whole plan.

### Track A — make failure branchable, then express failover as a graph

| # | spec | scope | depends on |
|---|---|---|---|
| A1 | `FailoverReason` + `classify()` in `pii-abi` | typed taxonomy; classify from `PiiError` + HTTP status/provider context; a unit test per reason | — |
| A2 | Provider-side error context | `ollama`/`llamacpp`/http providers attach status + origin so `upstream_rate_limit` is decidable | A1 |
| A3 | **Model node reports failure as payload** | the model-calling node returns a successful `Outcome` carrying `{status, reason, provider}` instead of `Err`, so `route` can branch on `$.payload.reason`; non-provider faults keep node-error semantics | A1, A2 |
| A4 | Failover ladder as a graph | a reference graph (`graphs/starter/`): sequential model nodes + `route` on reason + a compress branch for `ContextOverflow`/`PayloadTooLarge`; `retry` for same-provider backoff; validated by `pii graph lint` | A3 |
| A5 | Reason on the operator surface | confirm the reason reaches the journal/payload a human can read; document the ladder's journal signature | A4 |

### Track B — routing (co-equal, not a follow-on)

| # | spec | scope | depends on |
|---|---|---|---|
| B1 | Escalation ladder, generalised | the `verify-fix` shape as a reusable pattern: cheap model → deterministic `gate` → `route` on pass/fail → escalate; bounded by `loop_budget`. **No LLM judge.** | A4 |
| B2 | Safety rules as rule ORDER | non-negotiable `task_class` rule matching **first**, declared in config, so no router verdict **or document text** can reach the cheap port | A4 |
| B3 | Peak-aware window route | a `transform` node computes the current peak window from config (never hardcoded); an equal-price `route` prefers the off-peak provider, with the peak provider as fallback | A4 |
| B4 | Local tier-0 router node (research) | granite in-process as a free difficulty classifier feeding a `route` | B1, B2, B3 |

### Sequencing

- **A1 first** — small, pure, no runtime behaviour change, and everything else
  depends on it. Non-vacuously verifiable: one test per reason mapping from a
  concrete error.
- **A3 is the real work.** A1+A2 produce a reason; A3 makes it *branchable*.
  Until a `route` can see the reason in a payload, no graph can fail over —
  which is why the chain is not a struct to write but a shape to unlock.
- **A4 is mostly composition**, not new machinery: edges, `route` rules, and a
  `retry` policy over primitives that already exist and are already tested.
- **B1/B2/B3 each depend only on A4**, so routing does not wait for the whole
  failover track. **B3 is the standout** — a `transform` plus one `route` rule,
  near-zero risk, savings on every equal-price call, and it composes with A
  because it is a *preference order*, not a hard filter (off-peak preferred,
  peak still usable when nothing else is available).
- **B4 is research** and should not start until B1–B3 are proven in production.

---

## 6. Risks

- **Burning the chain on a request-shaped error.** Mitigated by the `Recovery`
  table and an explicit test that `ContextOverflow` does *not* advance the chain.
- **Losing the reason.** A silent switch makes cost off-peak/failover analysis
  impossible; **A5 is mandatory, not polish**.
- **Config drift between the daemon chain and `.pidag`/`ORCH_CHAIN`.** Two
  chains already exist in the shell world; document which layer owns which.
- **Peak data going stale.** Prices and windows change; the docs cite sources
  and `/root/.local/bin/llm-peak` is the runtime check. **B3 must read the
  windows from config, not hardcode them.**
- **Over-engineering before need.** A 16-reason taxonomy earned by Hermes's
  hundreds of provider quirks need not be ported wholesale; start with the
  classes Pii can actually produce, and grow the enum as new failures appear.

---

## 7. Bottom line

> **Follow-up (2026-09-21):** `039-jev-system-one-decision-layer-analysis.md`
> supplies the **decision-engine** half this plan leaves open. §4.4 item 5 here
> names a tier-0 classifier ("granite in-process") but does not settle what the
> classifier *is*; 039 analyses a System-1 decision source (Jev/Laya-class) for
> exactly that slot, and §3.5 there distinguishes it from Needle 3 (a
> *dispatcher*, not a classifier). 039 is analysis only, proposes no
> implementation, and keeps every constraint below intact — notably §4.4 item 3's
> rule that the non-negotiable branch must be a static rule matching first.

Two co-equal tracks, one substrate.

**Failover (A)**: Hermes's edge is not "it has a fallback list" — it is that
**every failure is typed, and the type selects the recovery**. Pii already
produces the raw failures (429/5xx → `Unavailable`) but throws the type away.
A1 recovers the type; A2 sharpens it; **A3 makes it branchable**, which is the
real work; A4 expresses the ladder as the graph shape Pii already knows how to
run.

**Routing (B)**: the larger steady-state payoff, because it applies to *every*
request rather than only to failures. It needs the same graph primitives, then
three independent, low-risk additions: escalate on a deterministic check (B1),
protect the task classes that must never go cheap (B2), and prefer the off-peak
provider whenever two entries cost the same (B3). B4 — granite in-process as a
free tier-0 router — is the natural next step once those are proven, and is the
only part of the article's playbook that needs a classifier.

**The architectural point**: because Pii is graph-first, this is mostly *unlocking
what already exists* rather than building a router. `route` is the router, the
journal is the observability, `loop_budget`/`retry` are the bounds, and the
static graph makes the article's injection failure mode structurally impossible
instead of merely advised against.

Ship A1 first; B3 is the highest value per unit of effort in the plan.

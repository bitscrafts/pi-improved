# Pii — Pass 3: Tier 0 on the CPU, and the harness/DAG/pipeline substrate

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

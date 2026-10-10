# 057 — The direction is the vision: sequence for implementing ALL remaining pillars (2026-10-10)

**Owner decision (verbatim, 2026-10-10)**: *"the direction is what it was, we proceed in order. all must be
implemented!"* — so P4 is **BUILD, not re-scope**. `_docs/01-overview/001-pii-vision.md` stands as written:
30+ channels, ~20 providers, SOP engine, WASM plugin host, hardware HAL, entity/relationship memory with
tiering, and the self-improvement loop (skill extraction, procedural templates, trajectories,
`Skill.graph` promotion after shadow-run agreement).

**How this doc is used**: it is the ORDER. Each row becomes one spec, implemented on the claude+sonnet lane,
verified by the orchestrator, committed between specs. A row may be split when it proves too large for one
run (the 100/101 precedent) — never merged.

## Where the vision actually stands (measured 2026-10-10)

| vision phase | state |
|---|---|
| 0 toolchain + workspace + CI gates | **DONE** |
| 1 core engine (Ctx, graph, journal, resume, cold start 3-4 ms) | **DONE** |
| 2 coding harness (tools, sandbox, verify-fix) | **DONE** |
| 3 assistant/gateway (daemon, UDS, channels) | **PARTIAL** — daemon + UDS + 2 channels (`cli`, `discord`) + 3 provider kinds (`scripted`, `ollama`, `llamacpp` off by default); no SOP engine |
| 4 IoT/reactive (MQTT, hardware HAL, WASM plugins) | **ABSENT** |
| 5 self-improvement (skills, distillation, trajectories) | **SUBSTRATE ONLY** — spec 103 landed trajectories (correlation, verbatim outcome, redaction at the sink, default OFF); no extraction, no verifier, no promotion |
| pillar 3 extras (entities/relationships, tier-aware tool selection) | **ABSENT** — BM25+vector+RRF+decay+consolidation exist |

Roadmap phases P0-P3 and P5 of `052` are the *production readiness* axis and are closed except P5 (running);
this doc is the *vision* axis that P4 was gating.

## The order (each row = one spec, landable and gated on its own)

**Stage A — finish the gateway (vision phase 3), because everything else is reached through it**

| # | spec | why first | done when |
|---|---|---|---|
| 111 | **providers**: provider kinds beyond the three — an OpenAI-compatible kind (covers most vendors by config), plus `anthropic` and `gemini` as explicit kinds | the smallest step that makes pii useful with the models the owner actually has; no new transport, the existing `provider_from_spec` seam already dispatches | each new kind resolves, streams (or returns a clear "streaming unsupported"), reports errors through `pii-observe`, has a recorded-trace test and a config sample |
| 112 | **channels**: 2 → the vision's list, started with the ones reachable here (HTTP webhook in + out, a generic chat-webhook channel, then matrix/slack/telegram as thin adapters) with a channel trait that a third party can implement | channels are how a user meets the assistant; the trait + two workhorses beat ten stubs | a channel trait documented + one new working channel end-to-end (message in → run → reply out), plus an adapter template |
| 113 | **SOP engine**: declarative standard-operating procedures (steps, preconditions, approvals) compiled into the existing graph | the vision's SOP engine is the gateway's structured-work surface and reuses the DAG, not a new engine | an SOP file loads, lints, runs, suspends for approval and resumes; a failing precondition refuses loudly |
| 114 | **providers tiering hook**: tier-aware model selection (pillar 3's second half) — map a task class to a model tier with the ladder already in `models.json` | depends on 111 having enough kinds to tier | a config maps tiers to models; a run records which tier it used; a wrong-tier request is refused rather than silently downgraded |

**Stage A2 — the defects P5's measurement found (spec 110 findings F1-F8), because a product that lies or
grows without bound is not usable**

| # | spec | why | done when |
|---|---|---|---|
| 115 | **operator surfaces under load**: (F1) `run.get` answers NotFound for a KNOWN run that is still in flight (128 054 such polls for 12 452 runs) so a client cannot tell "not yet" from "unknown id"; (F4) trajectories have per-run caps but NO aggregate cap or retention (85 019 files / 196 MB in 33 min, ~160k inodes/hour at the ceiling); (F5) the store never shrinks (216.6 MB / 85k runs, `pii_store_bytes` the only visibility); (F6) `pii metrics` exposes no latency histogram and no RSS, so the numbers an operator needs under load are invisible | a run in flight is distinguishable from an unknown id (test + paste); trajectory retention is bounded by a documented policy and the bound is proven; store growth is visible and has a documented compaction/vacuum path (or an honest statement of why not); metrics carry latency + RSS |
| 116 | **cost findings**: (F2) RSS grows ~155 B/run with NO plateau over 85 018 runs (+25.9 MB), baseline config included — investigate by bisecting the run path (client turn / journal / sink state / run table) and fix what is identified, or state precisely what was ruled out; (F3) write amplification ~125-131 KB written per run to retain 2.5 KB (~50x, ~3 GB per 8 min at 44 runs/s); (F8) there is no `NullObserver` switch and no way to disable the audit log, so the instrumentation A/B could not be run as specified | each item either fixed with a mutation control, or closed with the measurement that shows it is not the cause; the A/B switches exist (or the report says why not) |

**Stage B — IoT/reactive (vision phase 4), honestly bounded by what this host can prove**

| # | spec | why | done when |
|---|---|---|---|
| 117 | **MQTT node(s)**: subscribe/publish as graph nodes (native client, not WASM) | the reactive pillar's front door; testable against a local broker | a graph subscribes to a topic and fires a run; a publish is observable; a broker outage degrades loudly, not silently |
| 118 | **hardware HAL**: an abstraction with a SIMULATED backend first, so the contract is testable here, plus one real backend behind a feature | no hardware in this container — the contract must be provable without it | the HAL trait + simulated backend + tests; the real backend feature-gated and documented as unverified on this host |
| 119 | **WASM plugin host**: `wasmtime` + WASI component model, epoch interruption, fuel opt-in, pooling allocator | the vision's plugin surface; enables third-party nodes without rebuilding | a trivial WASM node runs under the host's limits; an infinite loop is interrupted by epoch; a plugin without fuel is bounded |

**Stage C — self-improvement (vision phase 5), on the substrate that already exists**

| # | spec | why | done when |
|---|---|---|---|
| 120 | **extraction**: turn trajectories (spec 103) into candidate skills/procedures, with the corpus-quality guards (no fake wins — the laya lesson) | the substrate records; this is what reads it | a real trajectory set yields ≥1 candidate, each citing its source runs; a failed run can never produce a success claim |
| 121 | **verifier + shadow runs**: `outcome.verifier` as a programmatic pass/fail, and shadow-run agreement before any promotion | a skill that is not verified is a liability, and promotion without agreement is how bad procedural memory spreads | a candidate is accepted only on verifier pass + shadow agreement; both negatives (no verifier, disagreement) refuse |
| 122 | **promotion + lifecycle**: `Skill.graph` zero-token promotion, progressive disclosure, never-delete lifecycle | the vision's mechanism, gated by 119 | a promoted skill is selectable without spending tokens; deleting is never destructive; the lifecycle is exercised end to end |

**Stage D — pillar 3 completion**

| # | spec | why | done when |
|---|---|---|---|
| 123 | **entities/relationships**: an entity model over the existing memory stores, with extraction from documents and a graph view | the vision's cognitive-memory pillar names it explicitly | entities and relationships persist, are searchable, and survive consolidation; a document yields entities with provenance |
| 124 | **memory tiering**: tier-aware retention/recall (hot/warm/cold) on top of decay + consolidation | the vision pairs tiering with model tiers | a tier policy is configurable and observable; a recall across tiers is measured |

## Rules this order inherits (they are not optional here)

- One spec per run, claude+sonnet lane, **orchestrator reproduces every headline claim**, mutation control
  for every new assertion, both gates green (with `QG_SKIP_COVERAGE=1` only when disk demands it, said aloud),
  workers never commit, `specs/` is planner-only.
- **No capability lands dormant by accident**: if a stage ends with something unwired on purpose, it goes in
  `_docs/05-operations/dormant-capabilities.md` with its check (the `pii-http-fetch` precedent).
- **A vision claim is only closed when the artefact exists**: `001`'s "30+ channels / ~20 providers" is a
  promise; each stage updates the doc's status lines as it lands, so the document and the code agree at every
  point (the 2026-10-09 static-binary correction is the precedent).
- **Order is load-bearing**: 111 before 112 (a channel needs a provider to answer); Stage A2 (115-116) is
  independent of the vision stages but must run before Stage B, because a surface that lies or grows without
  bound makes every later measurement untrustworthy; 120-122 do not require 123-124 (memory stands alone), but
  121 must land before 122 (no promotion without a verifier).

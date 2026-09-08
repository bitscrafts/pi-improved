# Pii — P4.7: The Self-Improving "Boss" Module (deep design)

**Date**: 2026-09-07 · **Author**: orchestrator (grounded in web research + this repo's actual substrate) · **Status**: DESIGN — not yet a spec

## 0. The user's ask (verbatim intent)

> "the P4.7 is a tough one, we need internet research, agent-memory research, this module should be the boss, checking logs from time to time, intersect problems fix them, deploy the app (in parallel) pass all the work to the new version until its production ready (never mix development and production) and keep improving, researching the internet for new on the area, trying new tech. lets have a deep inspection on this!"
> "it should manage its own context! using specific models!"

So P4.7 is NOT the narrow opus "Slice 2" (durable patch artifacts + ledger evolution). It is a **self-aware, self-managing, self-improving deployment loop** — a "boss" that:
1. **Manages its own context** — compacts/summarizes its own session, keeps a durable `state.md`, uses external memory as the source of truth (not the compaction summary).
2. **Uses specific models per role** — a model hierarchy (scout/planner/worker/reviewer), not one model for everything.
3. **Checks logs periodically** — a cron-driven loop that watches the daemon's logs/journal for problems.
4. **Intersects problems and fixes them** — detect → localize → patch → validate → deploy.
5. **Deploys in parallel** — a dev build and a prod build run side by side; work is passed to the new version until it's production-ready; **never mix dev and production**.
6. **Keeps improving** — researches the internet for new tech, tries new things, and feeds learnings back.

## 1. Research grounding (2026-09-07 web + agent-memory)

### Context management (the "manage its own context" part)
- **Anthropic "Effective harnesses for long-running agents"** — the best way to elicit incremental progress is to have the agent **commit progress to git with descriptive messages** and **write summaries to a progress file**. This is the "state.md" pattern.
- **Zylos "Agent Context Compaction"** — set compaction threshold at ~70-75% of the context window (not 95-98%); design for **compaction chains** (a session over 4h compacts 10+ times); **external memory must be the source of truth for durable facts, not the compaction summary**; active goal state should be written to a structured `state.md` at each milestone.
- **Context engineering 2026** — 70%+ of AI coding failures trace to poor context design, not model capability. Conversation summarization that captures **decisions and outcomes** (not dialogue verbatim) reduces history tokens 80%+ while preserving what matters. **Sub-agent isolation** is the most powerful context-management pillar.
- **agent-memory (our own)**: `insights/declarative-attention-relevance` — Declarative Attention's "reversible context compaction" (arXiv:2609.02737) directly addresses the destructive-compaction pain we hit. `pi-usage/diagnosis/looping-back-to-prompt-context-exhaustion` — the recurring "pi loops back to the prompt" symptom is context exhaustion; the fix is bounded context + explicit state.

### Model routing (the "using specific models" part)
- **Model routing 2026** — treat "which model handles this step" as a per-request decision, not a one-time setting. Three tiers: fast/cheap, balanced, frontier. Route by task complexity, cost-quality tradeoff, and provider health. **A single routing error cascades** into downstream cost/latency/quality failures.
- **KissAPI session management** — "a scout does not need the same model as a reviewer. A reviewer should not necessarily be the same model that wrote the patch. **Model diversity catches weird mistakes.**" Routing table: repo search → fast model; patch generation → strong coding model; security review → different strong model; docs → mid-tier; long failing-debug loops → **escalate after 2 attempts**.
- **Hierarchical multi-agent** — an orchestrator routes sub-prompts to specialized models (HuggingGPT pattern). The orchestrator plans and delegates; it does not execute coding itself.
- **Our model hierarchy** (from `~/.pi/agent/models.json`): planners = `glm-5.2[nvidia]` / `kimi-k3[moonshotai]` (free), `claude-sonnet-5[anthropic]` (paid); workers = `deepseek-v4-flash-0731[nvidia]` / `gemini-3.6-flash[google]` (free), `deepseek-v4-pro[deepseek]` (paid). This is the substrate for the boss's model routing.

### Self-healing / self-improving (the "boss" part)
- **Addy Osmani "Self-Improving Coding Agents"** — the "continuous coding loop": one capable agent loops longer (overnight/multi-day) on a complex project rather than a swarm. **Live logs** (tail the agent's output), **reflective improvement** (add usage examples to AGENTS.md when the agent struggles with an API — "pair-programming retrospectives").
- **The 5-role pipeline** (from the existing self-healing concept doc, grounded in SWE-agent → AutoCodeRover → RepairAgent): Detect → Localize → Patch → Validate → Deploy/PR + learn.
- **AIOps "AI Oops"** (arXiv 2508.06394) — never act on a single unverified signal; require reproduction before patching.
- **Opus's P4.7 review** (in `_tmp/phase3/opus-heal-and-rss-response.md`): the strongest self-improvement machinery is NOT the heal module — it's the **deterministic no-model gates** (`reachability-check`, `doc-drift-check`) that convert regressions into machine-checkable facts. The highest-leverage work is **extending those gates to the silent-no-op class** (a declared config key with no reader; an event variant with no producer; an RPC method with no dispatch arm). That would have caught `memory={enabled}`, `AssistantDelta`, and `session.list` automatically.

## 2. What Pii already has (the substrate — not duplicated)

| Capability | Where | Reuse |
|---|---|---|
| Journal + event bus | `pii-store` journal, `pii-server::Bus` | the boss's "check logs" source — subscribe to `event.subscribe` or read the journal |
| `pii graph trace` | renders every model/tool/routing decision | the root-cause substrate |
| `xtask` gates | quality-gate, lint-files, deps-check, net-check, rss-gate | the Validate step |
| `agent` node + graphs | the graph engine | the boss loop IS a pii graph (dogfooding) |
| `gate` node + approvals | F5.9/F5.10 | "never auto-merge without a human" |
| `cron`/timer wheel | daemon scheduler | the periodic "check logs" trigger |
| Budget system | B6 `max_tokens` | bound each heal attempt |
| `heal_detect` node + `HealStore` | S0.1/S0.2 | the detect step + ledger |
| `pii web` viewer | P4.6 | human observability of the boss's work |
| `pii-memory` | P4.3 | the boss's external memory (source of truth) |
| Model hierarchy | `models.json` | the boss's model routing |

## 3. The architecture: a self-aware "boss" loop

The boss is a **cron-driven pii graph** (dogfooding — it runs on the substrate it heals) plus a **new `pii-boss` crate** for the context-management and model-routing logic that graphs can't express.

```
┌─────────────────────────────────────────────────────────────┐
│  BOSS LOOP (cron graph, runs every N minutes)               │
│                                                             │
│  [1] CHECK LOGS  ── read journal/bus, run xtask gates        │
│       │  (deterministic: reachability-check, doc-drift,     │
│       │   quality-gate, rss-gate)                           │
│       ▼                                                     │
│  [2] DETECT  ── a gate is red, or a silent-no-op is found    │
│       │  (reproduce once before recording — AIOps guardrail)│
│       ▼                                                     │
│  [3] LOCALIZE ── agent node (scout model) narrows to file:line│
│       │  (AutoCodeRover pattern)                            │
│       ▼                                                     │
│  [4] PATCH ── agent node (worker model) in a DISPOSABLE      │
│       │  worktree, fs_write scoped to the worktree only      │
│       ▼                                                     │
│  [5] VALIDATE ── run the SAME gate that failed, in the       │
│       │  worktree, against the patch. Green? → proceed.      │
│       ▼                                                     │
│  [6] DEPLOY (parallel) ── build the new version, run it      │
│       │  side-by-side with prod, pass work to it until       │
│       │  production-ready. NEVER mix dev and prod.           │
│       ▼                                                     │
│  [7] LEARN ── write the resolution to pii-memory + the       │
│       │  fixing-analysis ledger; update AGENTS.md with the   │
│       │  lesson (reflective improvement)                     │
│       ▼                                                     │
│  [8] RESEARCH ── periodically search the web for new tech,    │
│       │  try it in the dev build, feed learnings back         │
│       ▼                                                     │
│  [9] CONTEXT MGMT ── compact/summarize the boss's own        │
│       session; write state.md; external memory = source of   │
│       truth (NOT the compaction summary)                      │
└─────────────────────────────────────────────────────────────┘
```

### 3.1 Context management (the "manage its own context" part)

The boss must not die of context rot. Three mechanisms, per the research:

1. **Durable `state.md`** — at each milestone, the boss writes its active goal, current phase, decisions, and next step to a structured `state.md` (in the worktree or a boss state dir). This is the source of truth for "where am I", NOT inferred from history.
2. **External memory as source of truth** — durable facts (resolutions, lessons, model-routing learnings) go to `pii-memory` (P4.3), never to the compaction summary. The compaction summary is ephemeral; memory is permanent.
3. **Compaction chains** — the boss's session compacts at ~70-75% of context (not 95%), and it's designed to compact 10+ times over a long run. Each compaction preserves decisions/outcomes, not dialogue.

### 3.2 Model routing (the "using specific models" part)

The boss routes each step to a specific model tier, per the research:

| Step | Model tier | Why |
|---|---|---|
| Check logs / detect | deterministic (no model) | gates are machine-checkable; no model needed |
| Localize | scout: `gemini-3.6-flash[google]` (fast, cheap) | high volume, low risk |
| Patch | worker: `deepseek-v4-flash-0731[nvidia]` (strong coding) | quality matters |
| Review | reviewer: `claude-sonnet-5[anthropic]` (different strong model) | model diversity catches mistakes |
| Research | planner: `glm-5.2[nvidia]` / `kimi-k3[moonshotai]` | complex reasoning |
| Escalate | after 2 failed fix attempts → human | stops token burn |

The routing is a **per-request decision** (a `route` node in the graph), not a one-time setting. A single misrouted decision cascades, so the routing policy is explicit and auditable.

### 3.3 The dev/prod split (the "never mix development and production" part)

This is the user's hard requirement. The boss maintains **two parallel deployments**:

- **DEV build** — where the boss experiments, patches, and tries new tech. Runs on a disposable worktree / a dev runtime dir (`/root/.pii-dev/`). Anything can break here.
- **PROD build** — the stable, user-facing daemon (`/root/.pii/`). The boss NEVER touches prod directly.

The promotion rule: **work is passed to the new version until it's production-ready** — i.e. the dev build must pass all gates + a soak + the boss's own validation before it's promoted to prod. Promotion is a deliberate, gated step (a `gate` node + human approval), never automatic. This is the "pass all the work to the new version until its production ready" requirement.

### 3.4 The research loop (the "keep improving, researching the internet" part)

A periodic (e.g. daily) research step:
1. Search the web for new tech in the agent/self-healing/model-routing space.
2. Store findings in `pii-memory` (research + insights, per the agent-memory convention).
3. Try promising tech in the DEV build (never prod).
4. If it works and passes gates, promote it (gated).

## 4. What's genuinely novel here

- **The boss runs on the substrate it heals** — a pii graph, using pii's own journal, gates, approvals, budget, memory, and model hierarchy. Not a bolt-on service.
- **Self-managing context** — the boss compacts its own session, writes `state.md`, and uses external memory as the source of truth. This is the "manage its own context" ask, grounded in the Anthropic/Zylos research.
- **Model-routed, not one-model** — each step uses a specific model tier, per the routing research.
- **Parallel dev/prod with gated promotion** — the "never mix development and production" requirement, made concrete.

## 5. Build order (spec-sized, per the opus review + this design)

- **Slice A (deterministic, no AI)**: extend `reachability-check`/`doc-drift-check` to the silent-no-op class (a declared config key with no reader; an event variant with no producer; an RPC method with no dispatch arm). This is the highest-leverage self-improvement work and gives the boss something real to detect.
- **Slice B (the boss loop skeleton)**: a cron graph that runs the gates, records failures to the heal ledger, reproduces once, and suspends on a `gate` node for human acknowledgement. No patching, no write authority. Proves the dogfooding claim.
- **Slice C (context management)**: the `pii-boss` crate — `state.md` writer, compaction-chain logic, external-memory-as-source-of-truth wiring.
- **Slice D (model routing)**: a `route` node that dispatches each step to the right model tier.
- **Slice E (localize + patch + validate)**: in a disposable worktree, against a deliberately broken unit test. Write capability scoped to the worktree.
- **Slice F (parallel dev/prod + gated promotion)**: the dev/prod split, the promotion gate.
- **Slice G (research loop)**: the periodic web-research step feeding pii-memory.

## 6. Guardrails (non-negotiable)

- **Never operate on the live working tree or prod.** Every heal attempt runs in a disposable worktree; prod is never touched directly.
- **Never auto-merge or auto-promote.** Promotion suspends on a `gate` node + human approval.
- **Require reproduction before patching.** Never patch off a single unverified log line.
- **Require validation before promotion.** The patch must pass the same gate it was trying to fix, in the worktree, before promotion.
- **Budget-bound every attempt.** Cap tokens/model-calls per heal attempt.
- **Do not let the boss edit `specs/`.** Specs are planner-only.
- **Rate-limit heal attempts per failure signature.** Escalate after 2 failed attempts on the same signature.
- **The boss may build and test its own patch, but must never replace the running daemon binary.** Deploy stays human, permanently.
- **Provenance.** Every boss-authored commit carries a machine-identifiable trailer + branch prefix so a human can revert a class of changes at once.
- **Worktree lifecycle.** Creation, cleanup (mv to trash, never rm -rf), and a disk ceiling.

## 7. Open questions for the user

1. **Scope of the boss's autonomy**: should it auto-promote dev→prod after passing all gates + a soak, or always require a human `pii approve`? (The user said "pass all the work to the new version until its production ready" — suggests auto-promotion after validation, but the opus review says deploy stays human. Need a decision.)
2. **Which model for the "boss" orchestrator itself** (the one that decides what to do next)? The research says a fast model for routing/classification; the user said "using specific models" — is the boss a fast router (gemini-3.6-flash) or a strong planner (glm-5.2/kimi-k3)?
3. **How aggressive should the research loop be** — daily web searches, or only when the boss is idle?
4. **First target**: the opus review says use a deliberately-broken unit test (not the RSS bug). Confirm.
5. **Dev/prod split location**: a separate runtime dir (`/root/.pii-dev/`) + separate store, or a separate container/VM?

---

### One-paragraph honest summary
P4.7 is the capstone: a self-aware "boss" that runs on Pii's own substrate, manages its own context (state.md + external memory + compaction chains), routes each step to a specific model tier, checks logs periodically, detects/localizes/patches/validates problems in disposable worktrees, maintains a parallel dev/prod split with gated promotion, and keeps improving by researching the web and feeding learnings back. The research is grounded (Anthropic harnesses, Zylos compaction, model-routing 2026, Addy Osmani self-improving agents, the existing opus heal review). The build order starts with the deterministic silent-no-op gates (highest leverage), then the boss loop skeleton, then context management, model routing, patch/validate, dev/prod, and the research loop.

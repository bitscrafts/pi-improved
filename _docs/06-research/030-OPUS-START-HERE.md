# pi-improved — OPUS REVIEW ENTRY POINT (2026-09-17)

**Read this file first. It is the only file you need to open to give a
decision on every open question. Everything else is optional
deep-dive material, linked below, not required reading.**

## What changed tonight (one paragraph)

Phase 5.2 (structural debt: F-12/F-13/F-14/D4/D7) and Phase 5.3
(test-coverage gaps: F-16/F-18/F-19/F-20, F-22 verified) are both
CLOSED — 17 pidag specs (43-59) ran entirely on the free `ollama-cloud/
glm-5.3-flash:cloud` tier, zero paid escalations, $0 cost, 1.5%
weekly-quota consumed. Two real bugs were caught during independent
re-verification (not self-reported by the implementer): a same-thread
mutex re-lock deadlock, and a dropped crash-loop-cap early-return. A
separate pidag bug (unbounded output-drain hang) was found and fixed
in the pidag repo itself. Repo hygiene: 18G of stale `_tmp/` scratch
archived off-disk, hardcoded absolute paths and orphaned docs found and
partially fixed (spec-61 queued for the rest).

## USER-MANDATED (not optional): unify the DAG/graph format across pii and pidag

**User (verbatim, 2026-09-18): "using the same dag format in both could
be a must!"** This is stronger than the 4 decisions below — it's a
stated requirement, not a design preference up for debate. Ground
truth already gathered, in `/projects/pidag/docs/`:

- `pii-vs-pidag-dag-format-notes.md` — internal comparison: pii uses
  port-based graph routing (declarative pattern-match routes, not
  arbitrary code — see pii's `graphs/starter/verify-fix.toml`,
  `failover.toml`); pidag uses a flat DAG with `depends_on` +
  `retry_attempts`/`retry_backoff_ms`. Neither is a strict superset of
  the other today.
- `sota-orchestrator-research-2026-09.md` — external validation:
  Temporal's declarative retry taxonomy and Argo's exit handlers are
  compatible with BOTH formats without contradicting pidag's own
  static-graph thesis (`ARCHITECTURE.md`: "does not reason during the
  run"); LangGraph's arbitrary-code conditional routing was explicitly
  rejected as incompatible with that thesis — this sets a hard
  constraint on what "unified" can mean: whatever shape is chosen must
  stay declarative-only, no runtime-code routing in either project.

**Opus is asked to decide the unification SHAPE**, not whether to do
it: (a) pidag adopts pii's port-based routing wholesale, (b) pii
adopts pidag's flatter model, (c) a new shared crate/schema both
projects depend on, or (d) a translation layer (each keeps its native
format, a converter maps between them for tooling/analysis purposes
only). Option (c) is the only one that structurally prevents drift
long-term; options (a)/(b) require a real migration in the losing
project; option (d) is cheapest but doesn't satisfy "same format" as
literally stated by the user.

**Status**: spec-129 (a small, additive, non-retryable-error-
classification field — NOT format unification, just the highest-
confidence cross-format-compatible pattern from the research) is
currently being implemented via a sonnet subagent as a first concrete
step. It does not preempt or substitute for the unification decision —
it was scoped deliberately small so it wouldn't need to wait on the
bigger architectural call.

## The 4 decisions you're being asked to make

Full grounding for each is in `_docs/06-research/029-...md` (linked
per-section below) — this list is the DECISION, not the reasoning; open
the linked section only if you want the reasoning before deciding.

1. **Coding-harness graph shape.** Concrete proposal: fork
   `graphs/starter/verify-fix.toml`, add multi-criteria validation
   (reuse `validate-exit-criteria.sh` directly) and model-escalation-
   on-retry. → [§1 in 029](029-coding-harness-design-questions-for-opus.md#1-the-core-question-one-implementing-dag-not-an-abstract-loop)
   **Decide**: approve this shape, or a different one?

2. **F-13 scope** (already implemented as module-reorg, not full
   3-crate split). → [§2](029-coding-harness-design-questions-for-opus.md#2-f-13-scope-options-recorded-2026-09-17-deferred-until-now)
   **Decide**: was this the right call, retroactively, or should it be
   redone as a full split?

3. **Modularity idea** ("microservices in a single binary" — memory/
   MQTT/harness as swappable modules via the graph engine's uniform
   node interface). → [§3](029-coding-harness-design-questions-for-opus.md#3-modularity-idea-recorded-2026-09-17-microservices-in-a-single-binary)
   **Decide**: worth a real design doc + prototype, or too early?

4. **Chromecast-protocol module** (new tonight — house-wide Google
   Home/Cast control as a standalone daemon or pii companion crate).
   → [§5](029-coding-harness-design-questions-for-opus.md#5-new-idea-2026-09-17-mid-session-chromecast-protocol-as-a-standalone-daemon--pii-companion-module)
   **Decide**: scope it as a real next project, or defer?

Plus one small housekeeping question, not architectural:
**`_docs/06-research/` reorganization** (28 files, flat-by-number, no
thematic grouping) → [§4](029-coding-harness-design-questions-for-opus.md#4-separate-smaller-ask-reorganize-_docsresearch)
— flat-by-date vs. SSOT subfolders, your call, low stakes either way.

## What does NOT need your review

- Everything closed tonight (specs 43-59) is independently gate-
  verified and committed — re-litigating implementation details is not
  a good use of your time unless something in the summary below looks
  wrong to you.
- The remaining mechanical backlog (specs 57-61, in flight as of this
  writing) is routine test-coverage/hygiene work, same pattern as
  43-59 — no design judgment needed, just flagging it exists.

## Full commit list tonight (for provenance, not review)

```
34ad977 test: add pii-channels discord.rs coverage; fix hardcoded tmp path
70d00ca docs: record F-19/F-20 closure — Phase 5.3 backlog cleared
d69f415 test: add pii-tools coverage for bash/fs/worktree (F-19)
b5ded1b docs: record F-18 closure (spec-55)
8b4b13d test: add pii-proto wire round-trip tests (F-18)
bed68b0 docs: add Chromecast-protocol module idea to opus review package
c27bce3 docs: record F16/F-22 closures and F-18/19 partial progress
9d9a330 fix(pii-net): bound the accept loop to a fixed worker pool (F16)
69ddf80 docs: bundle coding-harness design questions for opus review
ca1856a test: add direct MemStore contract tests (F-18/F-19)
76f4e6c docs: record spec-52 (D4/D7) completion in HANDOFF.md
b48b497 fix: D4/D7 sync-blocking seam -- channel poll off the tokio executor, worker kill off the state lock
5c9e13e docs: mark Phase 5.2 complete in HANDOFF.md
a8b9af4 refactor: extract xtask/src/main.rs's argv parsers into cli_parse.rs
b98a581 refactor: split pii-tui/src/lib.rs into {state,render,input,driver}.rs
11df84d refactor: split pii-graph/src/run.rs into run/{mod,engine,resume}.rs
aaee53c refactor: split pii-store/src/mem.rs into mem/{mod,write,read}.rs
39db2e1 refactor: reorganize pii-server into engine/, node_kinds/, daemon/ boundaries
839ae38 refactor: move dead HealAttempt product fields into a generic extra bag
fc773ce docs: update HANDOFF.md for spec-45 (service.rs split)
629ac9f refactor: split pii-net/src/service.rs into service/{mod,conn,http,dechunk}.rs
8c365ad docs: update HANDOFF.md for spec-44 (silent_noop.rs split)
8988f5f refactor: split xtask/src/silent_noop.rs into silent_noop/{mod,scans,helpers}.rs
a337ab2 refactor: split agent.rs into agent/{mod,turn,tools,memorize,tests}.rs
eab311f feat: Phase 5.0 re-baseline + Phase 5.1 Track A (failover becomes real)
```

(Specs 57-61 will add a handful more commits after this doc was
written — check `git log` directly for the current tip if reviewing
later than the session this was written in.)

## Where to go for more detail (optional, in order of usefulness)

1. `_docs/06-research/029-coding-harness-design-questions-for-opus.md`
   — the 5 decision sections above, full reasoning + ground truth.
2. `HANDOFF.md` — full session ledger, spec-by-spec evidence, commit
   SHAs, every finding. Read this only if you want to audit HOW a
   decision was reached, not just WHAT the decision options are.
3. `_docs/06-research/026-codebase-analysis-and-phase5-plan.md` — the
   original Phase 5 punch list (F-11 through F-22) tonight's work
   closed most of.

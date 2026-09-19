# pi-improved — OPUS REVIEW ENTRY POINT (updated 2026-09-19)

**Read this file first. It is the only file you need to open to give a
decision on every open question. Everything else is optional
deep-dive material, linked below, not required reading.**

**Decision count: 4 original (coding-harness graph shape, F-13 scope,
modularity idea, Chromecast module) + 3 new (2026-09-19, code-knowledge graph:
the `reachability-check` upgrade, native-service shape/placement, and whether
the coding harness should consume the graph) + 1 housekeeping
(`_docs/06-research/` reorg).**

**Newest section: "NEW (2026-09-19): code-knowledge graph — TWO SEPARATE
THINGS" — jump there if you have already ruled on the original four.** It is
split into **Track A** (graphify as a tool for our own implementation work —
already applied, *not* a decision) and **Track B** (pii maintaining a code
graph inside its future coding harness — the three decisions).

## What changed since this package was opened (one paragraph)

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

**Since then (2026-09-18/19), still awaiting your review:**

- **A full fable-model codebase review was run and implemented** — 8 specs
  (62-69) covering a security fix (worktree signature validation), a broken
  feature (pii-web memory search key), a dead config knob (worker restart
  backoff), persistence of the doc layer, F-11 pass 2, gate hygiene, and
  `pii-cli` test gaps. All gate-verified and committed. Review doc:
  `032-fable-pii-review-2026-09-18.md`. **Read it if you want an independent
  second opinion on the codebase before ruling on the four original decisions
  — it was produced by a different model with different blind spots.**
- **The `reachability-check` upgrade (Decision 5) was derived from that
  review**, as was the measured `PiiError`-dominance evidence above.
- **Two pidag defects were documented** in `pidag/docs/FINDINGS.md`
  (`26c1ff7`, `2db967c`), one of which is the source of the failover
  constraint quoted in the new section.

## USER-MANDATED (staged, not immediate): unify the DAG/graph format across pii and pidag

**User (verbatim, 2026-09-18): "using the same dag format in both could
be a must!"** Then clarified on follow-up: **"'identical file format'
is a good option for future, 'Compatible principles' is good for
now!"** — so this is a TWO-PHASE requirement, not an immediate merge:

- **Phase now**: compatible principles across both formats (already
  satisfied by the constraint below — declarative-only, no
  runtime-code routing in either project). No migration needed today.
- **Phase later**: a single identical shared schema/format, once
  there's a concrete need driving it (e.g. the coding-harness graph in
  §1 below, which would live in whichever project ends up hosting it).

Ground truth already gathered, in `/projects/pidag/docs/`:

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

**Direction (assistant's recommendation, pending opus confirmation):
pidag should end up as a constrained SUBSET of pii's schema, not the
reverse.** Reasoning: pii's port-based routing is a strict
superset in expressiveness (a fixed pass/fail edge is just a port with
one rule; the reverse doesn't hold — pidag's flat depends_on cannot
express pii's branching/pattern-match routes without new machinery).
pii is also the general-purpose orchestration engine (memory, channels,
tool calls, heals); pidag is a specialized single-workflow consumer
(implement → validate → gate → retry/iterate). A specialized tool
naturally profiles down from a general one; going the other direction
would regress pii's existing branching capability that `failover.toml`/
`heal-fix.toml` already rely on. Concretely: pidag's SDD DAG would
become expressible as pii's schema restricted to just the port
patterns pidag actually needs (a `pass` port and a `fail` port,
nothing more exotic) — one shared crate/schema, pii as the superset,
pidag as the constrained profile.

**Opus is asked to confirm or override this direction** and pick the
concrete unification vehicle once Phase-later is actually triggered:
(a) pidag adopts pii's port-based routing wholesale as a constrained
subset (assistant's recommendation), (b) pii adopts pidag's flatter
model (assistant's view: a regression, not recommended), (c) a new
third shared crate/schema neither currently has (higher cost, no
clear advantage over (a) given pii's format is already sufficient),
or (d) a translation layer only, no shared format (cheapest, but
doesn't satisfy the user's "identical format, future" goal).

**Status**: spec-129 (a small, additive, non-retryable-error-
classification field — NOT format unification, just the highest-
confidence cross-format-compatible pattern from the research) is
DONE — implemented via a sonnet subagent, gate-verified, committed
(`39e81a2`), and the resulting pidag binary is now installed and in
use. It does not preempt or substitute for the unification decision —
it was scoped deliberately small so it wouldn't need to wait on the
bigger architectural call.

## NEW (2026-09-19): code-knowledge graph — TWO SEPARATE THINGS

**Added to this package after the original four decisions. Same rule as
above: the DECISION is here, the reasoning is in the linked docs.**

**User clarification (2026-09-19, verbatim): "2 things here graphify as a
tool for our direct implementation and pii maintaining a graph like this in
its future coding harness!"**

These are **two distinct tracks** and must not be conflated. Track A is
about *us* (this session, and Claude Code subagents) working on the code.
Track B is about *pii at runtime* doing coding work. They share a tool and an
idea; they share no decisions.

Ground truth (three docs, all committed):

- `035-graphify-code-knowledge-graph-research.md` — what graphify is, and a
  **measured** trial on pii (269 code files → 4,187 nodes, 9,646 edges, 216
  communities, fully local, zero LLM calls, no API key).
- `036-graphify-integration-design-discussion.md` — the wiring mechanics, each
  one tested (uv isolation, `GRAPHIFY_OUT`, dual CLAUDE/AGENTS, hooks, skills).
- `037-pii-native-graph-service-design.md` — the native-service design.

---

## TRACK A — graphify as a tool for OUR direct implementation work

**Status: DONE and applied. Not a decision — review only if you think the
approach is wrong.**

graphify external tooling is now wired in **both** pii (`22152e7`) and pidag
(`d93a6a4`): `graphify-out/` gitignored, always-on rules in `CLAUDE.md` +
`AGENTS.md`, a PreToolUse soft nudge in `.claude/`, Hermes + pi skills. Git
hooks deliberately **not** installed (a post-commit rebuild changes what
"clean tree" means, and the pidag effect-verification seam depends on tree
state). Both graphs built locally, $0.

**What Track A buys, concretely — and its limits:**

- `graphify query` / `path` / `explain` answer *"what does this touch?"* and
  *"how do these connect?"* without a grep-and-read sweep. Free, local.
- The god-node table is **evidence for architecture judgements** (below).
- **Limit, measured:** it is for *orientation on unfamiliar ground*. A
  published head-to-head found it uses **more** tokens than semantic search on
  targeted work in a known codebase (BFS ~1,500 tok vs 100-350 tok per hit).
  On a codebase we know well, it is not automatically cheaper than reading.
- **Limit:** community labels need an LLM, so the local path yields the graph
  but not readable community names.

### Evidence Track A produces for the EXISTING decisions (machine-checked)

graphify's god-node list on pii — the most-connected concepts, by edge count:

```
PiiError 406   Ctx 168   GraphSpec 60   Store 44   RunEvent 42
Response 42    GraphNode 42   Engine 37   NodeRegistry 30   CapKind 29
```

- **`PiiError` at 406 edges is 2.4× the runner-up.** The most load-bearing
  abstraction in the codebase is the error type. This independently supports
  the F-11 direction (structured error taxonomy) that motivated specs 60/66 —
  a judgement originally made by hand, now confirmed by a tool that knew
  nothing about our reasoning.
- **`GraphSpec` / `GraphNode` / `NodeRegistry` / `Engine` clustering high** is
  direct input to **§1 (coding-harness graph shape)** and **§3 (modularity
  idea)**: the graph engine's own abstractions are among the most-connected
  things in the tree, which is evidence for *and* a caution about extending
  them.
- Use it as evidence, not proof: these are co-occurrence/call edges with
  `EXTRACTED` vs `INFERRED` tags, and a high edge count can mean "central" or
  merely "widely referenced in signatures".

---

## TRACK B — pii maintaining a code graph inside its future coding harness

**Status: proposed. Decisions 5-7 below. This is where your judgement is
actually needed.**

The link to §1 is direct and it is the reason this track exists: **§1 asks for
one implementing DAG** (fork `verify-fix.toml`, add multi-criteria validation
and model escalation). A coding harness that *implements specs* is a graph
that edits a codebase — and a code graph is precisely the index such a graph
would want to consult. The gap §1 identifies is that nothing composes the
three existing patterns into one graph; the question here is whether a
**fourth thing** — a maintained view of the code's own structure — belongs
inside that harness or alongside it.

Two shapes, and I recommend rejecting the first:

- **Shape 1 (reject): wrap graphify's output.** A node kind over
  `graphify-out/graph.json`. Imports a foreign artifact schema, a foreign
  extractor's measured quirks (one source file produced **zero** nodes; two
  files produced duplicate nodes), and 15 MB into our verification seams.
  Also drags a Python tool into pii's runtime dependency story.
- **Shape 2 (recommend): a pii-native code graph.** In Rust, scoped to what
  our gates need, maintained by the engine like `memory` is.

**Why this is cheap: the seam already exists, and `memory` is the precedent.**
`pii-server`'s own module doc states the registration rule — node kinds live
in `engine/registry.rs` because they *"need a resource the engine owns … that
`pii-graph` itself must not know about."* That is exactly a code graph's
situation. The five-part pattern is already exercised end to end by `memory`:
a `NodeFactory` in the registry, `effects()`/`required_caps()`, named
`ports()`, a method in the **closed wire vocabulary**
(`pii-proto/src/method.rs`), and **one shared tool**
(`pii-tools/src/memory.rs`) so that `pii-mcp` and the server's agent loop share
*"one code path, one capability gate, one bound."* Nothing in that list is new
work.

**Three graphs are in play and must not be conflated:** the **execution**
graph (`pii-graph` — the DAG being run), the **concept** graph
(`pii-memory/src/graph_lane.rs`, F4.8, over memory docs), and the **code**
graph, which does not exist today. pii already hosts two; Track B is about the
third.

### Decision 5 — upgrade `xtask reachability-check` from string search to a real call graph

The smallest, most defensible native step, and **the only one I would do
first.** `xtask reachability-check` exists for the B1 class
(`ollama::provider_from_env` with zero non-test callers) and the M3 class
(`Supervisor::invoke` called only from tests). Its own doc admits the
weakness verbatim: *"Pure text search is sufficient: we check 'does this
string appear as a call outside tests', **not graph structure**."*

A string search cannot distinguish a **call** from a string literal, a doc
comment, or a same-named field — which is the **"check that looks rigorous
while testing the wrong surface"** failure mode **already documented twice in
`pidag/docs/FINDINGS.md`**. A tree-sitter call graph turns the gate into: *"is
there an `EXTRACTED` `calls` edge from a non-test file to this symbol?"* Same
living-list interface, strictly stronger oracle. This is **the same class of
upgrade as F-11**: a typed structural fact replacing a string-matching
approximation.

**Decide**: approve this as the first native step? Reasoning:
[§ Shape 2 in 037](037-pii-native-graph-service-design.md).

### Decision 6 — should pii host a code-knowledge graph as a native SERVICE, and where?

**Decide (a)**: Shape 1 or Shape 2?
**Decide (b) placement**: a new lane in `pii-memory`, a sibling store in
`pii-store`, or parser-in-`xtask` only?
**Decide (c) dependencies**: a Rust-only tree-sitter parser drags in grammar
crates, against a strict `deps-check`, a **450-line effective-size gate**, and
workspace-wide `#![forbid(unsafe_code)]` (`Cargo.toml:54`). Recommend scope
**Rust-only** (not graphify's ~40 languages) and prefer reusing **`petgraph`**
(already in-workspace via amem). If the grammar crates still fail policy, keep
the parser in **`xtask`** — outside `crates/`, and it already hosts every gate.
**Decide (d)**: expose to agents at **runtime** (a tool — more useful, risks
becoming a toy) or **gate-time only** (`xtask` — cheaper, cannot drift)?

### Decision 7 — should the coding harness (§1) CONSUME this graph?

This is the Track B question that matters most, and the one I am least sure
of — which is why it belongs in your review rather than mine.

If pii maintains a code graph, then a coding-harness DAG could **use** it
rather than merely coexist with it. Three candidate uses, each a design
choice:

1. **Pre-edit orientation.** Before the `agent` node in a `verify-fix`-derived
   harness writes anything, a `graph.code.query` call tells it what the
   target symbol touches — turning the harness's context from "read the files
   you guess are relevant" into "read the subgraph the graph says is
   relevant." This is the direct analogue of what Track A does for *us*.
2. **Post-edit blast-radius check.** After an edit, ask which
   `EXTRACTED` edges now point at changed symbols. Potentially a **stronger
   gate input** than the current single `test_command` exit code — it can
   catch "this passed the tests but broke an unreferenced caller."
3. **Tool-set narrowing for the harness's own agent.** Today `pii-mcp`
   registers `ToolSet::builtin` wholesale (fable Finding 6 / security
   Finding, spec-62's neighbourhood) — every tool visible every turn. A
   queryable code graph makes it possible to narrow the admissible set
   per turn.

**Decide**: is any of these worth designing into the harness now, or is the
harness graph (§1) a *separate* decision that the code graph should not
complicate? Honest counter-argument I want you to weigh: §1's gap is
already well-defined (compose three existing patterns into one DAG), and
adding a fourth concern could delay a decision that is otherwise ready.

### Does Track B interact with the DAG-format-unification mandate?

A code-knowledge graph **is a graph**, and this package already carries a
**user-mandated** requirement to unify the DAG/graph format across pii and
pidag (*"using the same dag format in both could be a must!"*, staged as
compatible-now / identical-later).

- Is a code graph a **third format** to reconcile, or orthogonal (a data
  structure, not an execution plan)?
- Does it **change the unification direction**? The current recommendation is
  pidag becomes a constrained subset of pii's port-based routing. If pii gains
  a native graph *service* over a code corpus, does that strengthen the
  pii-as-superset case or muddy it?
- Does it **subsume the coding-harness graph (§1)**, or sit beside it?
  **Decide**: same thing, related, or unrelated?

### Constraint to confirm before any of 5-7 is built

From the stored failover architectural correction: *"a graph cannot branch on
a reason `Pii` never produces"* — the model node had to return a **successful
`Outcome`** carrying `{status, reason, provider}` instead of `Err`, and the
proof this pattern works is that the escalation ladder **already functions**
because the **tool node reports `exit_code` as payload** rather than failing.

So a code-graph node whose finding the harness **routes on** must return a
successful `Outcome` with the finding in the payload, **not `Err`**. Corollary:
a node that merely **answers** is a **tool** (effect-free, read-only); a node
whose judgement the graph **routes on** is a **route payload**. Those are
different designs and should be picked deliberately — note that Decision 7's
use (2) above (blast-radius as gate input) is exactly the *route payload*
case.

**Decide**: confirm this constraint, or say why it does not apply here.

### Veto condition (please hold me to it)

If the code graph cannot make a **gate** measurably stronger, it does not
belong in pii. pii's value is its verification seams, not its browsing.

## The 4 original decisions you're being asked to make

Full grounding for each is in `_docs/06-research/029-...md` (linked
per-section below) — this list is the DECISION, not the reasoning; open
the linked section only if you want the reasoning before deciding.

1. **Coding-harness graph shape.** Concrete proposal: fork
   `graphs/starter/verify-fix.toml`, add multi-criteria validation
   (reuse `validate-exit-criteria.sh` directly) and model-escalation-
   on-retry. → [§1 in 029](029-coding-harness-design-questions-for-opus.md#1-the-core-question-one-implementing-dag-not-an-abstract-loop)
   **Decide**: approve this shape, or a different one?
   **NOTE (2026-09-19): see Track B, Decision 7 — a code-knowledge graph
   could *feed* this harness (pre-edit orientation, post-edit blast radius,
   tool-set narrowing). Decide whether that belongs in the harness or is a
   separate concern that should not delay this decision.**

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

- Everything closed in the sessions this package covers (specs 43-69) is
  independently gate-verified and committed — re-litigating implementation
  details is not a good use of your time unless something in the summary
  below looks wrong to you.
- The graphify tooling wiring (`22152e7`, `d93a6a4`) is a mechanical
  integration, already tested and applied. Not a decision — flagged in the
  new section only so you are not surprised by it.
- The remaining mechanical items in the pi-improved backlog are routine
  test-coverage/hygiene work, same pattern as 43-59 — no design judgment
  needed.

## Full commit list (for provenance, not review)

```
# --- 2026-09-18/19: fable review implemented (specs 62-69) + graphify research ---
8c45d3e docs: Graphify research -- tooling vs a native code-knowledge-graph feature
22152e7 chore: wire graphify tooling in pii (local AST, gitignored output)
c2e2d12 docs: native pii graph service design -- the memory seam
552922b docs: Graphify integration design discussion (mechanics tested)
(corresponding pidag commit for the same wiring: d93a6a4)
55a8328 feat(xtask): lint-files --report mode; correct the 031 size list (spec-69)
75606c4 test(pii-cli): close pure-logic test gaps; hygiene batch (spec-68)
c47de9d fix: gate hygiene -- wire lint-files, clippy.toml, line-counter (spec-67)
6871616 refactor(pii-store): add PiiError::store_op constructor (spec-66)
2e47219 fix(pii-memory): persist + read the doc layer (spec-65)
fe35c37 fix(pii-worker): apply restart_backoff_ms in respawn (spec-64)
890c285 fix(pii-web): send the correct memory-search param key (spec-63)
883d768 fix(pii-tools): validate worktree signature, block path escape (spec-62)
1fc2d00 fix: replace hardcoded /projects/pi-improved/_tmp paths (spec-61)
35c4353 docs: mark Phase 5.x complete in HANDOFF.md
(plus the fable review doc itself: 032-fable-pii-review-2026-09-18.md)

# --- earlier session: Phase 5.2/5.3 closure ---
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
2. `_docs/06-research/032-fable-pii-review-2026-09-18.md` — the independent
   fable-model review of the whole codebase (10 findings, several
   security/behavioural), all implemented as specs 62-69. A different model's
   second opinion; useful context for the four original decisions.
3. `_docs/06-research/035/036/037-graphify-*.md` — the code-knowledge-graph
   chain: what the tool is (measured on pii), how it was wired, and the
   native-service design behind Decisions 5-7.
4. `HANDOFF.md` — full session ledger, spec-by-spec evidence, commit
   SHAs, every finding. Read this only if you want to audit HOW a
   decision was reached, not just WHAT the decision options are.
5. `_docs/06-research/026-codebase-analysis-and-phase5-plan.md` — the
   original Phase 5 punch list (F-11 through F-22) tonight's work
   closed most of.

# 029 — pii coding-harness graph: design questions for opus review

**Status**: RESEARCH / PRE-SPEC — no code written, this is a design
package for opus's judgment before anything gets built.

**Ask**: three related architecture questions bundled into one review,
since they're not independent — the harness's shape affects whether/how
the modularity idea applies to it, and the F-13 scope decision affects
what the harness's own crate boundaries should look like.

---

## 1. The core question: one implementing DAG, not an abstract loop

The user's framing (verbatim): **"the 'implement → gate → validate →
iterate' should be associated with ONE implementing DAG! lots to come!"**

This reframes the ask correctly — not "design a general concept," but:
**does a concrete `graphs/starter/implement.toml` (or similarly named)
exist that pii could run TODAY to take a spec + a failing test and
produce a validated patch, the way `pidag sdd --run` does externally
via `pi`?** If not yet, what's the smallest real graph that gets there,
built from primitives that already exist?

### What already exists (grounded by reading the actual files tonight)

Three starter graphs already implement variations of exactly this loop:

- **`graphs/starter/verify-fix.toml`** (64 lines) — the closest existing
  precedent. `agent` node (tools: `fs_read/fs_write/edit/multi_edit/
  bash`, `loop_budget = 3`) → `test` (bash tool node) → `route` on exit
  code → `pass`→done / `fail`→`retry`→back to `agent`. This is
  structurally IDENTICAL to pidag's `implement → validate` cycle, minus
  multi-criteria validation (pidag's exit criteria are a list of shell
  commands, this graph only checks one `test_command`'s exit code) and
  model escalation on retry (pidag's iter2 switches to a paid model;
  this graph's `loop_budget` just re-runs the same agent node, same
  model, no escalation).

- **`graphs/starter/heal-fix.toml`** (196 lines) — a more elaborate
  variant: `probe` (localize) → `route` → `fix` (agent, confined to a
  disposable worktree via `scope_root`, NO bash tool — only
  `fs_read/fs_write/edit/multi_edit`) → `retest` → `route` → `summarize`
  (git diff) → `record_fixed` (a `heal_record` node — ledger evidence
  BEFORE the human gate) → `escalate` (a `gate` node — human approval is
  the ONLY way past this point, nothing auto-merges).

- **`graphs/starter/failover.toml`** (Phase 5.1 Track A's own graph) —
  shows how `route` + `loop_budget` + a dead-end port (`compress`) express
  a retry/escalate ladder driven by a real classified error taxonomy
  (`FailoverReason`), not just a bash exit code. This is the shape a
  real "iter1 free model fails → iter2 paid model" escalation would need
  to borrow from, if the harness graph wants pidag's model-escalation
  behavior rather than verify-fix.toml's same-model retry.

### The real gap: nothing composes these three patterns into ONE graph

No existing `graphs/starter/*.toml` combines:
1. `heal-fix.toml`'s worktree confinement + human-gate-before-commit
   discipline (safety),
2. `verify-fix.toml`'s multi-turn agent-fixes-a-failing-test loop
   (the actual coding work), and
3. `failover.toml`'s classified-failure-driven model escalation
   (cost discipline — try free first, escalate only on real failure,
   per AGENTS.md's own cost policy already governing the pidag/pi
   worker chain).

**Concrete pre-spec ask for opus**: should the FIRST implementing DAG
be a fork of `verify-fix.toml` (simplest, already close), extended with:
- Multi-criteria validation: replace the single `test` node with either
  (a) N sequential `tool` nodes each gating on their own exit code, or
  (b) one `tool` node running a wrapper script that runs all exit
  criteria and reports pass/fail/total like pidag's own
  `validate-exit-criteria.sh` already does (reuse that script directly
  — it already exists in `~/.pi/agent/skills/validate-exit-criteria/`)?
- Model escalation on retry: does `route_step`'s model-resolution
  mechanism (already used by `heal-fix.toml`'s `fix` node, spec 33)
  support switching models between loop iterations, the way pidag's
  `[sdd].models_for_iter` config does? If not, is that a real gap in
  `pii-graph`'s node/route primitives, or achievable today via two
  `agent` nodes (one per model) chained by a `route` on attempt count?
- Human gate placement: `heal-fix.toml`'s "gate before anything leaves
  the sandbox" discipline is clearly right for pii's own self-repair use
  case — does a general "implement a spec" harness need the SAME gate,
  or is that gate specifically about the SELF-modifying nature of
  heal-fix (pii patching itself) rather than a property every
  implementing DAG needs?

## 2. F-13 scope options (recorded 2026-09-17, deferred until now)

Already stored in amem (`pii/phase5.2/f13-scope-options-for-opus`) —
summarizing here so it's in one document with the rest:

- **Option 1 (CHOSEN, implemented as spec-47)**: module reorganization
  INSIDE `pii-server` (`engine/`, `node_kinds/`, `daemon/` as
  directories, not crates) — lower risk, keeps `dispatch.rs`'s existing
  `daemon::methods` coupling as an accepted, documented constraint.
- **Option 2 (NOT chosen)**: full 3-crate split (`pii-engine`,
  `pii-nodes`, `pii-daemon`) — would require either moving
  `dispatch.rs`'s routing logic or accepting a real crate dependency
  edge from "engine" to "daemon," a bigger design decision deferred to
  a session with more runway.
- **Option 3 (NOT chosen)**: skip F-13 entirely, do only F-12 (the
  warn-band file splits) — rejected as too conservative given F-13 was
  the largest single remaining item in the 026 report's own punch list.

**Ask**: was Option 1 the right call, or does the harness design above
change the calculus (e.g. if the coding-harness graph needs its own
crate-level isolation from the daemon, Option 2's split might now be
worth revisiting)?

## 3. Modularity idea (recorded 2026-09-17, "microservices in a single binary")

Already stored in amem (`pii/architecture/microservices-in-single-binary-
idea-for-opus-review`) — the user's own framing, explicitly flagged for
opus feasibility analysis, not a committed direction:

> "the hard idea is like having microservices in a single binary" —
> memory, MQTT, coding harness, etc. as modules that COULD be pulled out
> as standalone external processes later, communicating over a common
> protocol, with the graph engine's `GraphNode` trait as the candidate
> uniform interface (in-process vs. remote becomes a config knob on the
> node, using `pii-worker`'s existing UDS JSON-RPC protocol — already
> built for hardware/untrusted isolation, currently underused).

**Ask**: does this connect to the coding-harness question above? If the
harness becomes a real, load-bearing subsystem (not just a starter
graph), is IT a candidate for this same in-process-vs-worker-isolated
treatment, or is that premature until the harness proves itself
in-process first?

## 4. Separate, smaller ask: reorganize `_docs/06-research/`

Not architecture — pure documentation hygiene, but flagged now since
opus is already reviewing this session's output.

**Current state** (verified by listing the directory): 28 research docs
(001 through 028), numbered sequentially with NO thematic grouping —
architecture plans (001-003, 011), feature probes (004, 014-023),
phase-plans (007, 025, 026), reviews (009, 010, 012, 028), and status
docs (024, 027) are all interleaved by number only. `_docs/INDEX.md` and
`_docs/TOC.md` exist workspace-wide but the research folder itself has
no internal grouping the way the workspace-level `docs/01-overview/`
through `docs/99-archive/` SSOT structure does (per this workspace's own
`AGENTS.md`/`ssot-doc-organization` convention).

**Ask**: should `_docs/06-research/` get the same SSOT treatment (e.g.
subfolders by theme: `architecture/`, `phase-plans/`, `feature-probes/`,
`reviews/`), or is flat-numbered-by-date actually the right shape for a
RESEARCH folder specifically (where chronological order of discovery
matters more than topic grouping)? Either answer is fine — just needs a
decision recorded so the folder doesn't keep growing unstructured.

---

## 5. New idea (2026-09-17, mid-session): Chromecast-protocol as a standalone daemon / pii companion module

User's own framing (verbatim): **"'Chromecast-protocol' could be a
great standalone daemon project as well a pii companion module!
Chromecast-protocol could be the link to all google home devices at
home!"**

Concrete origin: researching how to use the user's existing Google Home
speaker as Hermes/pii voice OUTPUT (alarms, announcements, message
readout) surfaced that Google Home speakers are plain Chromecast-
protocol devices — the exact mechanism `chromecast-tv-mirror` (a
separate, still-research-only project in this workspace) already uses
for the TV. `pychromecast` (Python, HA-maintained, 2.7k stars) discovers
and casts to ANY Cast device on the local network, not just one
pre-configured TV.

**Why this is a real architecture fit, not just a feature request**:
this is a clean, concrete instance of the modularity idea in section 3
above — "graphs as universal infra, worker-isolation as config knob."
A Chromecast-protocol module is:
- Naturally standalone (its own local-network discovery/session-
  management concerns, unrelated to pii's coding/memory/graph domains).
- A genuine "house-wide" capability multiplier if it becomes a `pii`
  node kind (or `pii-worker`-isolated companion process) rather than
  a one-off script tied to a single hardcoded TV — the existing
  `mcp__chromecast__*` MCP tools ARE hardcoded to "the configured
  Chromecast" today; a real module would need device-targeting
  (by name/room) to reach "any Google Home in the house," not just one.
- A candidate for the SAME in-process-vs-worker-isolated treatment
  discussed in section 3, using `pii-worker`'s existing UDS protocol —
  or, per the user's own framing, potentially simpler as a fully
  independent standalone daemon that pii/Hermes both talk to, rather
  than living inside either.

**Ask for opus**: is this genuinely a good candidate MODULE to build
(house-wide Cast-protocol control: TTS announcements, media casting,
device discovery, potentially the voice-INPUT bridge too if IFTTT/
Wyoming gets built later) — worth scoping as an actual next project
(new dir, e.g. `/projects/chromecast-hub` or a `pii-cast` crate), or is
it premature relative to the harness/modularity work in sections 1-3
above? If worth building, standalone daemon vs. `pii` companion crate
vs. `pii-worker`-isolated node — which shape, and why?

## Ground truth sources (for opus to verify independently, not trust blindly)

- `graphs/starter/verify-fix.toml`, `heal-fix.toml`, `failover.toml` —
  read in full during this session, quoted accurately above.
- `~/.pi/agent/skills/validate-exit-criteria/run.sh` — exists, parses
  `## Exit Criteria` checkbox items the same way pidag's spec format
  does (both projects independently converged on this shape).
- amem keys: `pii/phase5.2/f13-scope-options-for-opus`,
  `pii/architecture/microservices-in-single-binary-idea-for-opus-review`,
  `pii/roadmap/conformance-audit-2026-09-17` (the phase-by-phase
  planned-vs-built audit from earlier this session).
- `_docs/06-research/` directory listing (28 files, verified via `ls`).

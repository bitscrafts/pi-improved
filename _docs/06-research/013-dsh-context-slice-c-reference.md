# dsh-context — reference design for pi-improved Slice C (context management)

**Date**: 2026-09-10 · **Author**: orchestrator (research) ·
**Status**: REFERENCE ONLY — Slice C is deferred behind specs 30-36 + B; this
doc records the design to steal when we build it. Do NOT implement from this
doc yet.

## 1. Why this exists

The user asked to record, for future pi-improved implementation, the insight
that **dsh-context is the design to steal for Slice C** (context management).
This doc is the SSOT for that reference. It builds on:

- `_docs/06-research/012-p47-remaining-opus-plan.md` — Slice C / "manage its own context"
  (compaction + `state.md` + external memory as source of truth).
- `_docs/06-research/012-p47-remaining-opus-plan.md` — **D9**: Slice C is
  unbuilt; `crates/pii-server/src/compaction.rs:69` counts a turn as a
  `Role::User` message so `compact` is a no-op on any agent transcript
  (011 A17d, unfixed); `state.md` is written by the launcher, not a node.
- `_docs/06-research/005-dsh-vs-pii-harness-analysis.md` — the append-only
  journal + derived-projection pattern (dsh validates it; pi already has it).
- agent-memory `workspace/research/dsh-context` (facts) and
  `workspace/insights/dsh-context-relevance` (why it matters).

## 2. What dsh-context is (facts)

A DeepSeek Harness plugin (github.com/bowenliang123/dsh-context, Apache-2.0)
for context insight + management. Install: `dsh plugin --profile web add
dsh-context`; no build/restart. Gives a **Context tab** and an in-session
**`/context`** command.

**Key architectural anchor**: it reads the SAME official token-meter
projections the chat composer's context ring reads (`contextPressure` /
`contextBreakdown`) — i.e. context insight is **derived from the harness's own
token accounting**, not re-invented.

**What it shows** (all projections over the harness's event log):

1. **Current composition** — six-color stacked bar scaled to the model's full
   window: system prompt, tool schemas, user messages, injected context,
   assistant replies, tool results; gray track = remaining headroom; top-5
   most expensive tool schemas. "When a conversation starts degrading, this
   is where you find out which part ate the budget."
2. **Context stats** — turns, steps, injections, compactions, prunes.
3. **Context trend** — one stacked bar per model *request* (finer than
   per-message); **Total vs Delta** mode. Delta shows signed per-category
   change per request (e.g. Turn 5 step 1 grew +1.6k: injected +803, user
   +649, assistant +178). Real example: ~563k tokens across 48 turns, then
   compaction recycled −535.5k in one step.
4. **Context events** — every compaction, tool-output prune, skill/plugin
   injection, model switch, plan-mode toggle; labeled with producer source
   (instruction file paths, plugin id, skill name), net token delta
   (compactions/prunes show NET reclaimed), turn/step, timestamp; filterable
   by Inject/Compact/Prune/Switch/Mode.
5. **File activity** — one row per touched file (read/written/searched),
   aggregated to the step picked on the trend chart.
6. **Agent network** — current agent, parents, every subagent; one node per
   agent, colored parent→child edges (`parentId`).
7. **Context browser** — open the box of ANY request (Live next request or any
   retained step): six collapsible sections expand into per-element token
   prices, then into actual content (full system prompt, each tool's
   description + JSON schema, message text, reasoning, tool-call args, tool
   outputs). Tool results show OK/error status, Raw/Markdown toggle, image
   thumbnails with estimated token cost.
8. **Multimodal** (DSH 0.1.1+) — image attachments in full view, priced by
   DeepSeek's official image-size→token conversion (117–384 tokens/image).

## 3. Why it matters to pi-improved (the "steal" list)

Slice C is the one P4.7 slice that is **unbuilt** (012 D9). dsh-context is the
concrete reference design for it. The transferable lessons, in priority order:

1. **Context insight = a projection over the existing journal, not a new
   subsystem.** pi-improved already has the raw material: the append-only
   `RunEvent` journal (per-request token deltas, compaction events, tool
   results). A context-trend/composition view is a pure projection over it —
   exactly the "history derived by projection" pattern 005 already validated.
   Do NOT build a parallel context store.
2. **Read the harness's own token accounting, don't re-invent it.** dsh-context
   reads `contextPressure`/`contextBreakdown`. pi-improved should derive
   composition from its own per-request token metering (the same numbers the
   budget/`MicroUsd`/`WallMs` charging already produces), not a separate
   estimator.
3. **Delta mode is the diagnostic that fixes our compaction no-op.** The signed
   per-category change per request tells you WHICH part grew the window. This
   is directly actionable for `compaction.rs:69` (counts a turn as a
   `Role::User` message → `compact` never fires on an agent transcript).
4. **File Activity + Agent Network are natural graph-engine projections.**
   pi-improved has a real graph engine (nodes/edges/ports); dsh approximates
   these with plugins. When we build Slice C, these are cheap projections over
   the journal + the run's node/edge structure.

## 4. What NOT to copy

- The plugin-metaphysics (Cordis reversible registrations) — 005 already
  concluded pi's static single binary is the better fit for lnx.
- The web-UI chrome. pi-improved's context insight should be a `pii graph
  trace`-style CLI projection + the journal, not a browser tab.

## 5. When to act

Slice C is deferred behind specs 30-36 (P4.7 remaining) and B (residual
defects). When we pick it up, spec-first: a `context` projection over the
existing journal, plus the `compaction.rs:69` fix (turn counting) and the
`state_write` node kind (011 proposed, never built). This doc is the reference
design for that spec.

## 6. Record of where this lives

- This file: `_docs/06-research/013-dsh-context-slice-c-reference.md` (SSOT).
- Condensed facts + insight: agent-memory `workspace/research/dsh-context` and
  `workspace/insights/dsh-context-relevance`.
- Cross-links: 008 §3.1 (Slice C), 012 D9 (Slice C unbuilt).

# Spec: Pii Global/Cross-Session Memory (search + write policy)

**Project**: /projects/pi-improved
**Phase**: post-3.6 — memory activation (session memory already landed,
commit `58438cd`; this spec covers the SEPARATE cross-session layer)
**Status**: DRAFT — spec only, no implementation authorized yet (user
directive, 2026-09-05: "write a real spec first ... before any code").

## Overview

Pii ships a complete, working memory SUBSTRATE
(`pii-memory`: BM25 + int8 vector + graph-lane search, fused by RRF;
`pii-store::MemStore`; the `memory.search`/`memory.put`/
`memory.consolidate` RPCs in `crates/pii-server/src/daemon/methods.rs`)
that is fully functional but **never invoked automatically**. A graph's
`memory = { enabled = true, top_k = 6 }` node config (see
`graphs/starter/assistant.toml`) is parsed by nobody — the field exists,
is silently ignored, and gives a false impression the feature is wired.

This is a DIFFERENT problem from session memory (fixed `58438cd`):
session memory is "remember this conversation, within this conversation."
Global memory is "remember a fact ACROSS conversations/sessions/days" —
e.g. a user preference stated once, a recurring project convention, a
correction the user had to make twice. It needs its own read policy
(when to search, what to inject) and write policy (what's worth keeping
forever, at what confidence, with what dedup), because unlike session
history (bounded, ephemeral, cheap to keep in full) a global store grows
without bound and pollutes every future prompt if it's too permissive.

## Requirements

### Functional

#### G1 — Read path: when and what to search
- Triggered per-turn, ONLY when the owning graph's `agent` node has
  `memory = { enabled = true, top_k = N }` in its TOML config (the
  field ALREADY EXISTS and is parsed nowhere — G1 is "start reading it,"
  not "add a new field"). Graphs that don't opt in (e.g. heal-fix,
  verify-fix) get zero behavior change.
- Query text = the current turn's `user_text` verbatim (no query
  rewriting in v1 — LLM-based query expansion is a real technique but
  adds a model call per turn before the real model call; out of scope
  until v1's simple form is proven useful).
- Call `MemorySearcher::search(ctx, user_text, top_k)` (already exists,
  `crates/pii-memory/src/search.rs`) via the SAME `Ctx` the run already
  carries — no new capability grant needed, `memory` node kind already
  has `required_caps() == CapKind::empty()`.
- Inject hits as a SEPARATE, clearly-labeled system message BEFORE the
  session-history messages (not mixed into `history`, not mixed into
  the main `system` prompt) — e.g. `role: System, content: "Relevant
  memory:\n- <doc 1 text>\n- <doc 2 text>"`. Rationale: keeps the
  provenance visible for debugging (`pii graph trace` / the TUI's
  hex-seq indicator can show which nodes fired) and keeps memory
  swappable/disableable without touching history logic.
- Degraded search (embedder unavailable, `MemorySearchResult.degraded
  == true`) must NOT fail the turn — proceed with whatever lanes
  succeeded (matches `MemorySearcher::search`'s existing contract) or
  with zero memory context if all lanes failed.
- A search that returns zero hits injects NO system message at all
  (not an empty "Relevant memory:" header) — avoids training the model
  to expect a section that's usually empty noise.

#### G2 — Write path: what gets remembered, and when
- **Trigger**: only after a run reaches `Terminal::Success` (never on
  failure/suspend — a failed turn has nothing worth remembering).
- **What counts as memorable** (the actual design question — this is
  where "importance/dedup rules" from the user's ask lives): v1 scope
  is deliberately narrow, NOT "remember every turn" (that's just
  session memory duplicated into a slower, fuzzier store):
  - A memory candidate is proposed only when the assistant's own reply
    contains an explicit self-flagged marker, e.g. the system prompt
    instructs: "if the user states a lasting preference, correction, or
    fact worth remembering across sessions, end your reply with a line
    `MEMORIZE: <one-sentence fact>`." The agent decides, not a separate
    classifier call (no extra model round-trip).
  - Rationale for self-flagging over an always-on heuristic: a
    keyword/regex heuristic ("remember", "always", "prefer") is exactly
    the kind of thing that either over-fires (every casual mention) or
    under-fires (misses phrasings); asking the model that already has
    full context to flag it explicitly is cheaper and more accurate
    than a second classification pass.
  - `MEMORIZE:` lines are stripped from the text shown to the user (the
    TUI/`pii -p` output) — they are a side-channel, not part of the
    conversational answer.
- **Dedup**: before writing, run `MemorySearcher::search` for the
  candidate fact's own text at `k=3`; if the top hit's fused score
  exceeds a configurable threshold (default 0.85 — needs empirical
  tuning once real usage exists, NOT hardcoded blind), skip the write
  and instead bump the existing doc's `last_used_ms`/`importance`
  (`pii_memory::doc::Doc` already has both fields) — this is a refresh,
  not a new fact.
- **Importance**: v1 always writes at `importance = 1.0` (the
  `Doc::new` default already used by the existing `memory.put` RPC) —
  no per-fact importance scoring in v1; consolidate's existing
  `archive_after_days` decay (already implemented,
  `crates/pii-memory/src/consolidate.rs`) is the only aging mechanism.
  A smarter importance model (recency-weighted, frequency-weighted) is
  explicitly OUT OF SCOPE for v1.
- **Layer/kind**: v1 always writes `Layer::L1`, `DocKind::Atom` (the
  existing `memory.put` defaults) — no classification into
  Scene/Persona/Skill in v1.
- Uses the EXISTING write path (`Doc::new` + `pii_memory::tokenize` +
  `PutRows` + `MemStore::put`, currently duplicated inline in
  `daemon/methods.rs::memory_put`) — extract that into a small shared
  helper (`pii_memory::doc::write_fact` or similar) so the agent's
  write path and the `memory.put` RPC don't duplicate the
  tokenize-then-PutRows logic a third time.

#### G3 — Consolidate stays a manual/cron operation
- No change to `consolidate` — it already exists as a `memory` node kind
  (`op = "consolidate"`) usable in a cron graph. G2's writes just feed
  it more real data than the RPC-only path currently does. Not adding
  an "auto-consolidate after N writes" trigger in v1 — keep write and
  consolidate decoupled until real growth data justifies coupling them.

### Non-Functional
- **No new model calls per turn** beyond the search (search is a pure
  BM25/vector/graph-lane lookup, zero model calls per `pii-memory`'s
  own crate-level invariant — see its doc comment "Zero model calls,
  zero socket code").
- **Backward compatible**: any graph without `memory = {enabled=true}`
  sees zero behavior change (this is the same guarantee session memory
  already gave to `history`-absent runs).
- **Bounded per-turn cost**: G1's search is `top_k` (config-controlled,
  default suggestion 6, matching the ALREADY-DECLARED-BUT-IGNORED value
  in `graphs/starter/assistant.toml`) — not unbounded.

## Architecture

```
Turn N (memory-enabled graph):
  user_text
     |
     v
  [G1] MemorySearcher::search(ctx, user_text, top_k)  --(existing, unused today)
     |
     v (hits, possibly empty/degraded)
  System("Relevant memory:\n- ...")  --(NEW, only if hits non-empty)
     |
     v
  [existing] history messages (session memory, 58438cd)
     |
     v
  [existing] current user_text message
     |
     v
  model call -> reply (may contain trailing "MEMORIZE: <fact>")
     |
     v (on Success only)
  [G2] strip MEMORIZE line from user-visible text
     |
     v
  dedup search (MemorySearcher::search(fact_text, k=3))
     |
    / \
  hit>0.85   no strong hit
   |            |
 bump doc     write_fact() -> Doc::new + tokenize + PutRows + MemStore::put
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `search_injects_a_labeled_system_message_when_hits_exist` | graph with `memory.enabled=true`, a pre-seeded memory doc matching the prompt | sent `ChatRequest.messages` contains a `System` message with "Relevant memory:" and the doc text, positioned before `history` |
| `search_injects_nothing_on_zero_hits` | `memory.enabled=true`, empty store | no extra `System` message beyond the base `self.system` one |
| `search_degraded_lane_does_not_fail_the_turn` | embedder forced to error | run still reaches `Success`, uses whatever lanes succeeded |
| `memorize_marker_is_written_and_stripped` | scripted reply ending in `MEMORIZE: user prefers dark mode` | (a) user-visible `output.text` has the marker line removed (b) a new `Doc` exists in the store with that fact text |
| `memorize_dedup_skips_near_duplicate_writes` | a fact already stored, then the same fact re-flagged | no second `Doc` created; existing doc's `last_used_ms` advances |
| `graphs_without_memory_enabled_are_unaffected` | `assistant.toml`-style graph with `memory` config OMITTED | no search call, no write path invoked, byte-identical behavior to pre-G1/G2 |

## Exit Criteria
- [ ] `cargo test -p pii-server memory_g1:: memory_g2::` (or equivalent
      module names) — all pass
- [ ] `cargo test --workspace` — 0 failures
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors
- [ ] Manual live verification (same pty-driven method used for session
      memory): state a preference in one `pii tui` session, quit, start
      a FRESH session (new `session.create`), ask a question that
      should surface the remembered preference — confirm it does.
- [ ] `graphs/starter/*.toml` other than `assistant.toml` show zero
      behavior change (their `pii graph lint`/existing tests still pass
      unmodified)

## Guardrails
- Do NOT add a second, competing "remember this" heuristic (e.g. a
  keyword regex) alongside the `MEMORIZE:` marker — pick one mechanism
  (the marker) and commit to it for v1; evaluate alternatives only
  after real usage data exists.
- Do NOT let this spec's implementation touch `pii-memory`'s core
  search/store crate's "zero model calls" invariant — all model-call
  surface area is in `pii-server::agent`, not `pii-memory`.
- Do NOT auto-tune the dedup threshold (0.85) or `top_k` via any
  runtime heuristic in v1 — these are static config values a human
  adjusts after observing real behavior, not something the agent
  self-modifies (that would be "self-improving," a separate, explicitly
  out-of-scope track per the earlier self-healing spec review).
- Do NOT wire this into the self-healing module (Slice 0/1) in this
  spec — global memory and self-healing are independent features that
  happen to share the `pii-memory` substrate; keep them decoupled.

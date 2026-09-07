# Spec: Pii Global/Cross-Session Memory v2 (search + write policy)

**Project**: /projects/pi-improved
**Phase**: 4.3 — global memory v2 (opus Phase-4 plan: "REWRITE spec 09 first — 3 concrete defects")
**Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
commits and updates HANDOFF.md)*
**Source**: Phase 4.3 (2026-09-07) — rewrite of the DRAFT spec 09 to fix the
three defects the opus review identified.

---

## Verified Premises

- `crates/pii-memory/src/rrf.rs:18` — RRF score is `1/(60 + rank)` per lane. A doc ranked #1 in all three lanes scores ≈ `3/61` ≈ **0.049**, then × decay ≤ 1. **Spec 09's "fused score exceeds 0.85 → skip the write" can NEVER fire** — every re-flagged fact writes a new duplicate forever.
- `crates/pii-server/src/daemon/methods.rs:285` — `memory_put` writes `vector: None`. The vector lane has NO data for any doc the product writes.
- `crates/pii-memory/src/embed.rs:127` — `OllamaEmbedder::dims()` returns `0` ("unknown until the first response"), so `search.rs:65` (`Some(e) if e.dims() > 0`) always skips the vector lane → `degraded = true`.
- `crates/pii-memory/src/embed.rs:45-84` — `TrigramEmbedder` is a working offline embedder (`dims() == 256`, deterministic, no model call). This is the fix for the vector lane.
- `crates/pii-memory/src/search.rs:17-26` — `MemoryHit { doc, score, lanes }` — `doc` is the doc **id only**, no text. `MemorySearchResult.hits` carries no doc text, so the read path cannot inject "Relevant memory:\n- <doc text>".
- `crates/pii-memory/src/doc.rs:34-55` — `Doc` has `text`, `concepts`, `importance`, `created_ms`, `last_used_ms`, `evidence`. `Doc::new` (doc.rs:60) computes the content id from text (idempotent put).
- `crates/pii-store/src/mem.rs:27-35` — `PutRows { id, doc, terms, vector: Option<(Vec<i8>, f32)>, edges, dl }` — the vector field exists and is persisted when set.
- `crates/pii-server/src/daemon/methods.rs:231-295` — `memory_put` RPC builds `Doc::new` + tokenize + `PutRows` inline. This is the write path to extract into a shared helper.
- `crates/pii-server/src/agent.rs` — the agent turn loop (spec 13/14/15) is where the read-path injection and MEMORIZE-marker write hook land.

---

## Overview

Pii ships a complete, working memory SUBSTRATE (`pii-memory`: BM25 + vector + graph-lane search fused by RRF; `pii-store::MemStore`; the `memory.search`/`memory.put`/`memory.consolidate` RPCs) that is **never invoked automatically** and, worse, **lies about itself**:

1. **The dedup threshold cannot fire.** Spec 09's write policy says "skip the write if the fused score exceeds 0.85" — but RRF scores max out at ~0.049, so the threshold is unreachable. Every re-flagged fact writes a new duplicate, forever.
2. **The vector lane has no data.** `memory_put` writes `vector: None`, and `OllamaEmbedder::dims()` returns 0, so the vector lane is always skipped. The "BM25 + vector + graph" story is really "BM25 + graph with a false `lanes` report."
3. **The read path can't read.** `memory.search` returns doc **ids only** — no text. Injecting "Relevant memory:\n- <doc text>" is impossible without a `memory.get` or text in the hit payload.

This spec fixes all three and wires the substrate into the agent loop (read injection + MEMORIZE-marker write), per the original spec 09's design but with the defects corrected.

---

## Requirements

### Functional

#### G1 — Read path: when and what to search
- **G1.1**: Triggered per-turn, ONLY when the owning graph's `agent` node has `memory = { enabled = true, top_k = N }` in its TOML config (the field ALREADY EXISTS and is parsed nowhere — G1.1 is "start reading it," not "add a new field"). Graphs that don't opt in get zero behavior change.
- **G1.2**: Query text = the current turn's `user_text` verbatim (no query rewriting in v1).
- **G1.3**: Call `MemorySearcher::search(ctx, user_text, top_k)` via the SAME `Ctx` the run already carries.
- **G1.4**: Inject hits as a SEPARATE, clearly-labeled system message BEFORE the session-history messages — `role: System, content: "Relevant memory:\n- <doc 1 text>\n- <doc 2 text>"`. **This requires the hit payload to carry doc text** (defect 3 fix).
- **G1.5**: Degraded search (embedder unavailable) must NOT fail the turn — proceed with whatever lanes succeeded or zero memory context.
- **G1.6**: Zero hits injects NO system message (not an empty "Relevant memory:" header).

#### G2 — Write path: what gets remembered, and when
- **G2.1**: Trigger only after a run reaches `Terminal::Success` (never on failure/suspend).
- **G2.2**: A memory candidate is proposed only when the assistant's reply contains an explicit `MEMORIZE: <one-sentence fact>` marker (the system prompt instructs the model to emit it for lasting preferences/corrections/facts). The agent decides, not a separate classifier.
- **G2.3**: `MEMORIZE:` lines are stripped from the user-visible text (side-channel, not part of the answer).
- **G2.4 (defect 1 fix — dedup)**: Before writing, run `MemorySearcher::search` for the candidate fact's own text at `k=3`. **Dedup uses an ABSOLUTE similarity signal, not the fused RRF score**: (a) exact-repeat → the content id (`Doc::new`'s blake3-of-text) already collides, so `MemStore::put` is idempotent — no duplicate is created; (b) near-duplicate → compare the candidate's trigram vector (via `TrigramEmbedder`) against the top hit's stored vector with a cosine threshold (default 0.9). If either fires, skip the write and bump the existing doc's `last_used_ms`/`importance` (a refresh, not a new fact).
- **G2.5**: Importance: v1 always writes at `importance = 1.0` (the `Doc::new` default). No per-fact importance scoring in v1.
- **G2.6**: Layer/kind: v1 always writes `Layer::L1`, `DocKind::Atom`.
- **G2.7**: Extract the write path (`Doc::new` + tokenize + `PutRows` + `MemStore::put`) into a shared helper `pii_memory::doc::write_fact` (or `pii-server`'s memory module) so the agent's write path and the `memory.put` RPC don't duplicate it a third time.
- **G2.8 (defect 2 fix — vector lane)**: `write_fact` computes and stores the embedding via `TrigramEmbedder` (offline, deterministic, no model call) so the vector lane has real data. `memory_put` uses the same helper. `OllamaEmbedder` is either fixed (report real dims after a probe) or left as the degraded path — the default is `TrigramEmbedder`.

#### G3 — Consolidate stays a manual/cron operation
- No change to `consolidate` — it already exists as a `memory` node kind (`op = "consolidate"`). G2's writes just feed it more real data.

### Non-Functional
- **N1**: No new model calls per turn beyond the search (search is pure BM25/vector/graph lookup; `TrigramEmbedder` is offline).
- **N2**: Backward compatible — any graph without `memory = {enabled=true}` sees zero behavior change.
- **N3**: Bounded per-turn cost — search is `top_k` (config-controlled, default 6).
- **N4**: `pii-memory` keeps its "zero model calls, zero socket code" invariant — all model-call surface is in `pii-server::agent`.

---

## Architecture

```
Turn N (memory-enabled graph):
  user_text
     |
     v
  [G1] MemorySearcher::search(ctx, user_text, top_k)  --(now returns doc TEXT)
     |
     v (hits, possibly empty/degraded)
  System("Relevant memory:\n- <doc text>...")  --(NEW, only if hits non-empty)
     |
     v
  [existing] history messages (session memory)
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
  write_fact(fact_text)  --(shared helper, computes trigram vector)
     |
     v
  dedup: exact id collision? OR cosine(trigram) > 0.9?
     |                        |
   yes (refresh)            no (write new Doc with vector)
```

**Key decision — dedup via content-id + trigram cosine, NOT the fused RRF score.** The RRF score is a *ranking* signal, not a *similarity* measure — it maxes at ~0.049 and can never be a dedup threshold. Exact repeats already collide on the content id (idempotent put). Near-duplicates are caught by cosine over the trigram vector (the same offline embedder the vector lane uses). Rejected: the spec-09 "fused score > 0.85" rule (unreachable) and a separate LLM similarity call (adds a model round-trip per write).

**Key decision — `TrigramEmbedder` as the default vector lane.** It's offline, deterministic, dims 256, and already implemented. `OllamaEmbedder` is broken (dims()==0) and needs a network call; it stays as an opt-in alternative but is NOT the default. This makes the vector lane real without a model dependency.

**Key decision — `MemoryHit` gains `text`.** The read path needs doc text to inject. Add `text: String` to `MemoryHit` (populated from `MemStore::doc`), so `memory.search` returns text, not just ids.

**What this spec is not**: memory v3 (importance scoring, query rewriting, auto-consolidate), the web viewer (P4.6), TLS (P4.4).

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| G1.1 | `search_injects_labeled_system_message_when_hits_exist` | graph with `memory.enabled=true`, a pre-seeded doc matching the prompt | `ChatRequest.messages` contains a `System` message with "Relevant memory:" and the doc TEXT, positioned before `history` |
| G1.2 | `search_injects_nothing_on_zero_hits` | `memory.enabled=true`, empty store | no extra `System` message beyond the base `self.system` |
| G1.3 | `search_degraded_lane_does_not_fail_the_turn` | embedder forced to error | run still reaches `Success`, uses whatever lanes succeeded |
| G1.4 | `memory_hit_carries_doc_text` (pii-memory unit) | a doc stored, `search` returns it | `MemoryHit.text == <doc text>` (not just the id) |
| G2.1 | `memorize_marker_is_written_and_stripped` | scripted reply ending in `MEMORIZE: user prefers dark mode` | (a) user-visible `output.text` has the marker removed (b) a new `Doc` exists with that fact text AND a non-None vector |
| G2.2 | `memorize_exact_repeat_is_idempotent` | the same fact written twice | only ONE `Doc` (content-id collision); second write is a no-op/refresh |
| G2.3 | `memorize_near_duplicate_skips_write` | a fact stored, then a near-identical fact re-flagged | no second `Doc`; existing doc's `last_used_ms` advances |
| G2.4 | `write_fact_stores_vector` (pii-memory unit) | `write_fact("...")` | the stored doc's vector is `Some` (trigram, dims 256) |
| G2.5 | `memory_put_uses_shared_helper` | `memory.put` RPC | writes via `write_fact` (vector populated), not the inline `vector: None` path |
| G2.6 | `graphs_without_memory_enabled_are_unaffected` | graph with `memory` config OMITTED | no search call, no write path, byte-identical behavior |

**G2.3 (`memorize_near_duplicate_skips_write`) is the acceptance test.** The requirement a plausible implementation gets wrong: it keeps the spec-09 "fused score > 0.85" rule (which can never fire, so every re-flag writes a duplicate) or it dedups only exact text (missing near-duplicates). The test asserts a near-identical fact does NOT create a second doc.

---

## Exit Criteria

- [ ] `cargo build --workspace --all-targets` — compiles (G1-G3)
- [ ] `cargo test -p pii-memory memory_hit_carries_doc_text` — hit text fix (G1.4)
- [ ] `cargo test -p pii-memory write_fact_stores_vector` — vector lane fix (G2.4)
- [ ] `cargo test -p pii-server memory_g1::` — read-path tests pass (G1.1-G1.3)
- [ ] `cargo test -p pii-server memory_g2::` — write-path tests pass (G2.1-G2.3, G2.5)
- [ ] `cargo test -p pii-server graphs_without_memory` — backward compat (G2.6)
- [ ] `cargo test --workspace` — 0 failures (N3)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N3)
- [ ] `cargo fmt --all --check` — clean (N3)
- [ ] `cargo xtask lint-files` — 0 hard errors (N3)
- [ ] `cargo xtask deps-check` — pass (N4)
- [ ] `cargo xtask net-check` — pass (N4)

**Prose criteria**:

1. Test counts pasted raw, one line per binary, **unsummed**.
2. The dedup must use content-id + trigram cosine, NOT the fused RRF score — a test that asserts "fused score > 0.85 skips" is testing an unreachable branch and is a FAIL.
3. `memory_put` must write a non-None vector (via `write_fact`), not the old `vector: None` path.

---

## Guardrails

- **G1 — do NOT edit this spec.** If it is wrong, STOP and report it.
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT regenerate a pinned fixture.**
- **G5 — no hardcoded absolute paths.** Test artefacts under `_tmp/`.
- **G6 — report raw output, never summed totals.**
- **G7 — do NOT add a second "remember this" heuristic** (e.g. keyword regex) alongside the `MEMORIZE:` marker — pick the marker and commit to it for v1.
- **G8 — do NOT touch `pii-memory`'s "zero model calls" invariant** — all model-call surface is in `pii-server::agent`.
- **G9 — do NOT auto-tune the dedup threshold or `top_k` at runtime** — static config values a human adjusts.
- **G10 — do NOT wire this into the self-healing module** — global memory and self-healing are independent.

### Error handling expectations

- A degraded search (embedder error) must not fail the turn — proceed with whatever lanes succeeded.
- A `MEMORIZE:` marker with no fact text (empty) must be ignored, not written as an empty doc.
- `write_fact` must never panic on a store failure — return `PiiError::Internal`.

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/pii-memory/src/search.rs` | `MemoryHit` gains `text`; populate from `MemStore::doc` (G1.4) |
| `crates/pii-memory/src/doc.rs` | add `write_fact` shared helper (G2.7) |
| `crates/pii-server/src/daemon/methods.rs` | `memory_put` uses `write_fact` (vector populated) (G2.5, G2.8) |
| `crates/pii-server/src/agent.rs` | read-path injection (G1.1-G1.6); MEMORIZE-marker write hook (G2.1-G2.3) |
| `crates/pii-server/src/registry.rs` | `DEFAULT_TOML`/`assistant.toml` memory config parsing (G1.1) |

**Not modified**: `crates/pii-memory/src/embed.rs` (TrigramEmbedder already works; OllamaEmbedder stays opt-in), `crates/pii-store/src/mem.rs` (PutRows already supports vectors), `crates/pii-proto/` (no wire change needed — `memory.search` result shape can carry text).

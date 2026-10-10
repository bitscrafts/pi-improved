# Spec: 126 — entities and relationships: the memory pillar's structure (and what the engine may read)

**Project**: /projects/pi-improved
**Phase**: **Stage D** of `057` — vision pillar 3 (long-term memory). Spec 127 adds tiering; this spec gives
memory its **shape**: typed entities, typed relationships, provenance, and a bounded query surface the engine can
use. Until now `pii-memory` holds notes/documents; there is no way to say *"this run was about the ESP32 board, and
that board is in the office"*.

## What exists (measured 2026-10-10)
- `pii-memory` with a store-backed document area (`memory_docs` appears in health) and `pii memory` surfaces.
- `pii-learn` (123/124) established the discipline this spec must reuse: **evidence/provenance mandatory**, typed
  records, deterministic ids, byte-identical reruns, and the substrate treated as **data, never instructions**.
- The trajectory/run substrate records what happened per run; nothing connects a run to a subject.
- No entity or relationship type exists anywhere in the tree.

## Requirements
R1. **Types, in a new module (or crate) with its own seam:** `Entity { id, kind, name, attributes, provenance }`
  and `Relationship { id, from, to, kind, attributes, provenance }`. `id` is **content-derived and stable** (the same
  entity described twice yields the same id — say what is hashed and prove reruns are byte-identical).
R2. **Provenance is mandatory**: an entity or relationship with no source (a run id, a file path, or a named actor)
  is **refused and counted**, exactly as candidates are in 123. An attribute set is data; the id never encodes it
  alone.
R3. **Upsert is idempotent and merges provenance**: describing the same entity twice yields one entity whose
  provenance lists both sources (sorted, deduped, bounded — state the bound and what happens when it is reached).
R4. **Referential integrity, stated and enforced**: a relationship naming an unknown entity is either refused with
  that reason, or creates a marked stub (`kind = "unknown"`, provenance = the relationship that implied it) — pick
  one, do it consistently, and test it. A dangling relationship must never be silently written.
R5. **A bounded query surface the engine can use**: by id, by kind, by name (exact; substring only if you state the
  cost), and **neighbours of an entity** with a depth cap (default 1, configurable) and a result cap. No unbounded
  scan, no arbitrary traversal without caps. Returns are **sorted deterministically**.
R6. **Retention and size are bounded** (the lesson of spec 115): a configured cap on entities, relationships and
  provenance entries per record, with a documented policy when a cap is reached (refuse the write with a counted
  reason, or evict oldest-first — pick one and prove it); `pii memory compact` (or the existing equivalent) reclaims
  space and refuses while a daemon holds the store (the behaviour spec 115 established).
R7. **Engine integration — the point of the pillar**: a graph node kind (e.g. `remember`) that records an entity
  and/or a relationship from a run's data, and a read path the engine can call (e.g. a `recall` node) — both
  attenuated by the capability model like everything else (`capability denied: memory: …` when not granted), both
  emitting named events. Paste a run that writes a fact and a later run that reads it back.
R8. **Observability**: events `memory.entity_written`, `memory.relationship_written`, `memory.write_refused`,
  `memory.query_refused` (with the reason), registered in `ALL`; counters in `pii metrics` **rendered through a live
  daemon** (the spec-123 lesson); `pii health` reporting the memory group's counts; and a **determinism** check:
  the same writes in a different order produce the same stored state (paste `cmp`).
R9. **Injection stance**: entity names/attributes come from run data and are **data**. A fixture whose attribute
  contains *"ignore previous instructions"* must not change what is written or how it is classified — the same
  control 123 carries.
R10. **CLI**: `pii memory entity add|show|list`, `pii memory link`, `pii memory neighbours <id> [--depth N]`.
R11. **Docs**: `_docs/0X-guides/memory-entities.md` (the types, provenance, caps, the node kinds and their
  capability, what memory is for); `001`'s pillar-3 status updated to what exists (entities/relationships done;
  tiering still absent).

## Exit Criteria
- [ ] the tests exit 0 with: idempotent upsert + merged provenance; refusal without provenance; the chosen R4
      behaviour (both directions if you created stubs); the caps (each direction); the neighbours depth and result
      caps; the injection control; the determinism `cmp`
- [ ] a mutation per rule — provenance check removed; id made non-content-derived; depth cap removed; cap-reached
      policy inverted; capability check bypassed — each pasted red, then restored
- [ ] the end-to-end paste: a run writes an entity through the `remember` node, a later run reads it through
      `recall`, with the events and the journal lines
- [ ] the counters move in `pii metrics` through a **live daemon**, and health reports the memory group (paste)
- [ ] `deps-check`, `net-check`, `doc-drift-check`, `lint-observability` exit 0
- [ ] `bash quality-gate.sh` exits 0; plain `fresh-gate` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the guide exists; `001`'s pillar-3 line updated (paste it)
- [ ] the report lists files, names, caps, config keys, and what was NOT verified

## Guardrails
- Reuse the discipline of 123/124 (provenance, determinism, data-not-instructions, spans/caps) rather than
  inventing a parallel mechanism; where a helper exists in `pii-learn`, sharing it is preferred.
- No model in the loop: entities are written by code from run data, not inferred by a model (an inference layer, if
  ever wanted, is a later spec with its own evidence rules).
- Never let a query scan the whole store: every query path is capped, and the cap is a named refusal, not a silent
  truncation.
- Do not touch the trajectory substrate or the ledger schema; memory references them, it does not extend them.
- Scratch under `/projects/_tmp/`, delete only your own, no daemon left running. Do NOT commit.

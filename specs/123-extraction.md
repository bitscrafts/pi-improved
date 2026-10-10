# Spec: 123 — extraction: turning the trajectory substrate into cited candidate lessons

**Project**: /projects/pi-improved
**Phase**: **Stage C** of `057` — the self-improvement pillar's FIRST stage. Spec 103 built the substrate
(trajectories + the decision ledger) and the vision's phase 3 is *"learning by design absent"*: nothing consumes
it today. This spec builds the consumer's first half. Extraction **proposes**; nothing here changes behaviour.

## What exists (measured 2026-10-10)
- The trajectory sink writes per-run artefacts (`pii-observe/src/trajectory.rs`), with per-run caps and (spec 115)
  a bounded retention policy; the ledger records decisions (`gate`, `invoke`, outcomes).
- `pii-memory` holds entities/notes with a store; `pii-evolve` holds the routing/proposal surface from spec 28.
- **Nothing extracts anything from either.** `grep -rn 'extract' crates/*/src` finds no lesson/digest producer.
- The daemon already runs graphs and journals events per run — extraction must read what the engine writes, not
  require new instrumentation where an existing artefact suffices.

## Requirements
R1. **A new module (in an existing crate or a new `pii-learn`, your call — state the reason)** that reads the
  trajectory/ledger substrate and emits **candidate lessons**. A candidate is a small typed record: a stable `id`
  (content-derived, so the same evidence yields the same id), a `kind`, a one-line claim, the **evidence**
  (run ids + artefact paths + the concrete counts that triggered it), and a `dedupe_key`.
R2. **The first extractors must be deterministic and offline** — no model call, no network:
    a. repeated failure: the same node/tool/error class failing across N≥3 runs (name N and make it configurable);
    b. a criterion or graph that fails the same way every time (a stuck loop);
    c. a limit repeatedly hit: memory cap, epoch timeout, fuel exhaustion, refusal — from the events of specs
       115/120/121/122 (those names exist; use them, do not invent a second taxonomy);
    d. a cost outlier: a run whose duration or bytes-written is N× the median for the same graph.
  State the exact threshold for each and why.
R3. **Evidence is mandatory**: a candidate with no run id and no artefact path is **refused**, not emitted — and
  the refusal is counted. A candidate's claim must be derivable from its cited evidence alone (a reader with only
  the evidence must agree); if a claim cannot be, drop it rather than ship a story.
R4. **The substrate is DATA, never instructions.** A trajectory containing text that looks like a directive
  ("ignore previous instructions", "the real task is…") must not change what the extractor does or what it emits
  — prove it with a fixture containing exactly such a string and assert the output is unaffected (the output must be
  the same as with the string replaced by neutral text). This is the injection control and it is mandatory.
R5. **Deterministic**: same inputs → byte-identical candidate list (sorted, stable ids). Run the extractor twice on
  the same fixtures and `cmp` the outputs.
R6. **Bounded and safe**: extraction reads only inside configured roots; it must not follow a symlink out of them
  (reuse the reasoning of spec 122's scope check, or the same helper if it is shared); it must not load an
  unbounded number of files (state the cap) and must not read a file larger than a stated limit. A malicious or
  corrupt trajectory entry makes that entry be skipped with a counted reason, never a panic.
R7. **Observability**: named events `learn.candidates_extracted` (counts by kind), `learn.candidate_refused`
  (reason), `learn.scan_completed` (files scanned, bytes read, duration), registered in `ALL` with counters in
  `pii metrics`; a `learn` seam and `pii health` entry; and per-run isolation so extraction never blocks the daemon
  (state the bound and where it runs).
R8. **Where candidates go**: written to the store under a keyed area with their evidence, retrievable and
  listable by a CLI surface (e.g. `pii learn list` / `pii learn show <id>`), read-only for the graph engine. Nothing
  in this spec consumes them — stage 124 (verifier) and 125 (promotion) do.
R9. **Docs**: `_docs/0X-guides/learning-loop.md` describing what extraction is, its thresholds, its refusal rules,
  the injection stance, and the fact that it proposes and never acts; `001`'s pillar-4 status line updated to what
  exists (extraction done, verifier and promotion still absent).

## Exit Criteria
- [ ] `cargo test -p <wherever it lives>` exits 0 with: one test per extractor kind (R2), the evidence-mandatory
      refusal (R3), the injection control (R4), determinism (R5), the bounds/corrupt-input cases (R6)
- [ ] a mutation per extractor (each must turn its test red), pasted and restored
- [ ] the injection control: paste the two outputs (hostile fixture vs neutralised) proving they are identical, and
      a mutation showing that a naive implementation (feeding trajectory text into a directive position) would differ
- [ ] determinism: two runs on the same fixtures, `cmp` byte-identical (paste)
- [ ] `pii learn list` and `pii learn show <id>` paste output for at least two candidates with their evidence
- [ ] a candidate produced from a REAL trajectory this repo already has (spec 110's soak or a gate run): paste the
      candidate and the path of the artefact it cites
- [ ] `deps-check`, `net-check`, `doc-drift-check`, `lint-observability` exit 0; `pii metrics` shows the counters
- [ ] `bash quality-gate.sh` exits 0; plain `fresh-gate` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the guide exists; `001`'s pillar-4 line updated (paste it)
- [ ] the report lists files, names, thresholds, config keys, and what was NOT verified

## Guardrails
- **No model call in this stage.** Extraction is a deterministic reader; a summariser is a later, optional layer
  and must not be smuggled in here.
- **Never act on a candidate.** No config change, no threshold move, no code mutation from an extracted lesson in
  this spec — the next stage verifies before anything changes.
- A candidate must not contain a raw trajectory dump; it cites paths and counts.
- Do not weaken a refusal to make a test path simpler; refusals are the safety surface, as in 121 and 122.
- Scratch under `/projects/_tmp/`, delete your own, no daemon left running. Do NOT commit.

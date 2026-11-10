# Spec: 75 — documentation correction pass (043 §9, corrections 1–8)

**Project**: <repo>
**Phase**: batch C — the docs debt from the 043 opus review, corrections 1-8 verbatim.

## Context
043 (`_docs/06-research/043-opus-full-review-2026-10-06.md`) §9 audited ~85 doc claims;
its "Highest-value corrections" list (items 1-8) is the contract for this spec. Batches
A/B already landed several changes (boss_watch, quality-gate wiring, specs 73/74) — do
not undo those; the corrections below are additive.

## Requirements (verbatim from 043 §9, tracked to today's tree)
R1. HANDOFF.md: put a `> **HISTORICAL SNAPSHOT (2026-09-16), superseded by CURRENT STATE
    above**` banner above the line carrying "Nothing is committed" (near :188). Add one
    row per landed spec 57-61 batch. Repoint :32 to `_docs/05-operations/033-...`.
R2. AGENTS.md: "18 crates" → "19 crates plus the xtask member" (verify with
    `grep -E '^members' -A 25 Cargo.toml | grep -c 'crates/'`); `:8` research pointer to
    `002-pii-architecture-plan-pass2.md`; `Cargo.toml:54` → `:56`.
R3. `_docs/06-research/030-OPUS-START-HERE.md`: collapse the triple 037 bullets
    (`:140-145`) to exactly one; replace the dead `035/036` bullet at `:485` with 037;
    update the "28 files" decision text; append one pointer line to 043.
R4. `_docs/06-research/000-TOPIC-MAP.md`: replace the stale plan text with the current
    map; mark "Still to do" items done; resolve the 041 collision (appendix = 043-referenced,
    report entry points at 043).
R5. `_docs/06-research/{025,024,015,012,032}.md` + `05-operations/033`: each gets ONE
    status banner citing the landing commit (find it with `git log --oneline -- <file>`
    and the spec's own commit message).
R6. `_docs/INDEX.md` + `_docs/TOC.md`: replace copied research lists with a link to
    000-TOPIC-MAP; fix "future src/" + "18 crates" + "40+ specs" lines; move 023 to
    02-guides, 033 to 05-operations (update INDEX lines after).
R7. `02-guides`: `pidag-way-of-work.md:91` → "2 iterations; iteration 2 swaps to paid,
    then stop" (cross-check `.pidag/config.toml:12` — it says 2); `pii-mcp-bridge.md:23`
    → document the `pii-mcp` binary (not a `pii mcp` subcommand).
R8. `_docs/manuals/technical-manual.tex`: add `run.resume` + `run.trace` to the method
    table (grep the RPC dispatch in `crates/pii-server/src/engine/conn.rs` for exact
    method names first); add `pii-uds` + `pii-http-fetch` to the crate table; change
    "feature-gated" → "separate binary"; repoint 008 → 012.

## Exit Criteria
- [ ] `grep -n "Nothing is committed" HANDOFF.md | head -1` shows the "historical snapshot" banner line number directly above
- [ ] `[ "$(grep -c '18 crates' AGENTS.md _docs/INDEX.md | grep -v ':0' | wc -l)" = 0 ]`
- [ ] `[ "$(grep -c '035/036' _docs/06-research/030-OPUS-START-HERE.md)" = 0 ]`
- [ ] `grep -c 'run.resume' _docs/manuals/technical-manual.tex | grep -v '^0'`
- [ ] `grep -c 'pii-uds' _docs/manuals/technical-manual.tex | grep -v '^0'`
- [ ] `python3 _docs/../../scripts/regen-index.py 2>&1 | tail -1` reports 0 broken links (adjust path: run from repo root, script is `scripts/regen-index.py` — confirm it exists first; if the pi-improved repo has no regen script, the INDEX/TOC link check is manual: grep every relative link target exists)
- [ ] `grep -c 'run.resume\|pii-uds' _docs/manuals/technical-manual.tex`
- [ ] `git diff --stat -- '*.md' '*.tex'` shows only docs files touched

## Guardrails
- Do NOT touch any `crates/` or `xtask/` file — docs only.
- Do NOT rename or delete any 0NN doc — banners and pointers only.
- Do NOT commit; leave the tree for the orchestrator.
- Do NOT edit specs/ or the 043 report itself (it is the contract, not a target).
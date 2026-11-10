# Spec: pii indexes its own codebase

## Overview
Measured 2026-10-19: `grep -rln graphify` over the repository finds nothing outside `_tmp`, and `pii memory tiers` reported entities=0 — pii has entity/link/neighbours verbs and a graph lane in search, and had never indexed one line of its own source. Meanwhile the workspace HAS a graphify tool for other codebases (laya, the workspace memory), so the product could be interrogated about other people's code and not about its own. User directive: "the graphify feature! initially indexing its own codebase" and "the respective graphs must also be modified accordingly".

**Project**: <repo>

## Requirements
### Functional
- An indexer walks every crate under `crates/*/src`, emitting one entity per module, struct, trait, enum and fn, with `--source file:<path>:<line>` and attributes (crate, lines, pub).
- Entities are created through `pii memory entity add`, so the graph lane can traverse them — a document that merely lists symbols does not satisfy this.
- RELATIONSHIPS must exist: `module contains <symbol>`, `crate depends_on <crate>`. This is the half that is NOT wired: `pii memory link` accepts no arguments in this build and prints the entity usage, so `relationships` stayed 0 after 1,415 entities.
- The graph lane in `pii memory search` must report "graph" among its lanes for a query that names an indexed symbol.
- The index runs as a GRAPH (`graphs/code-index.toml`): timer → worker(index) → gate(counts stated, relationships explicit) → memory(record the snapshot).

### Non-Functional
- Idempotent: re-indexing the same tree must not multiply entities.
- `--max` bounds the work so a scheduled pass cannot become a stampede.
- A path longer than 256 bytes is truncated rather than failing the whole file.

## Exit Criteria
- [ ] `uv run scripts/pii-index-code.py --max 3000` exits 0 and prints `entities=<n> failed=0` with n>0
- [ ] `pii memory tiers` reports `entities` > 1000
- [ ] `pii memory search "<an indexed symbol>"` returns `"graph"` in at least one hit's `lanes`  ← BLOCKED on links
- [ ] `pii memory entity list --name "pii-graph::journal"` returns exactly one entity with provenance `file:crates/pii-graph/src/journal.rs:1`
- [ ] `graphs/code-index.toml` loads: the daemon either arms one run for it or refuses it BY NAME (176/177)

## Guardrails
- Do NOT index `_tmp/`, `target/` or any vendored tree: the index is of pii's own source.
- Do NOT fabricate relationships: if `link` is unwired, say relationships=0 rather than inferring edges from naming.
- Do NOT run the indexer on the live daemon's hot path.

## Measured today (evidence this spec rests on)
- entities 1415 after 23 crates; relationships 0; tiers hot=1415; memory.redb ~6.6 MB
- `pii memory entity list --name "pii-graph::journal"` → one entity, provenance file:crates/pii-graph/src/journal.rs:1
- `pii memory link <id> <id> --rel contains` → prints the ENTITY usage: the verb is grouped in cmd_ctl.rs:191 but has no argument parser

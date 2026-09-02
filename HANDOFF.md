# Pii — Session Handoff

**Status: GREEN** (toolchain + SDD pipeline validated)

## What exists
- 3-pass architecture research in `_docs/06-research/` (passes 1-3, ~48k words total).
- 5 reference repos cloned in `_repos/` (read-only).
- pidag initialized (`.pidag/config.toml`) with `ollama-cloud/glm-5.3-flash:cloud` workhorse.
- SDD smoke test `specs/01-smoke-test.md` PASSED the pipeline (files created, iter2/3 recovery loop ran).

## pidag bugs FIXED this session (2026-09-02)
1. **Session:** `pi --session <file>` needs a pre-existing file. Fixed `src/worker/pi_print.rs`
   → `--session-dir <per-node-dir>` (auto-creates + `-c` continues, parallel-workers isolated).
2. **Model split:** `split_provider_model` (TWO copies: `src/lib.rs:48` + `src/backend/pi.rs`)
   mis-split `ollama-cloud/glm-5.3-flash:cloud`. Both fixed to split on earliest `:`/`/`.
Build + install to both `/root/.local/bin/pidag` and `/projects/.local/bin/pidag`.

## Next steps
1. Write Phase 0 spec (workspace skeleton + CI gates) with Claude Code / opus.
2. `pidag sdd specs/02-phase0-workspace.md --run --concurrency 2` (tests 2 parallel workers).
3. Iterate into Phase 1 (core engine: Ctx, graph, journal, cold-start).

## Key files
- `.pidag/config.toml` — model chain (slash-first form required).
- `AGENTS.md` — SDD/TDD workflow + hard rules.
- `specs/` — numbered specs.

## Memory keys
- `pii/architecture/plan` (pass 1), `pii/architecture/plan-pass2`, `pii/architecture/plan-pass3`
- `pii/research/repo-findings`, `pii/research/pass2-findings`
- `pii/implementation/pidag-fixes`

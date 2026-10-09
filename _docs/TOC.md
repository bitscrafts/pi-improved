# Pii Docs — Table of Contents

## 01-overview
- `001-pii-vision.md` — the unified vision and architecture overview

## 02-guides
- `pidag-way-of-work.md` — the SDD pipeline: static-graph orchestration, how pidag drives pi, the free→account→topup model ladder, the implement→quality-gate→validate loop, hard rules, and operational traps
- `feature-gating.md` — how to make the monolith compilable-down: Cargo features gate surfaces (tui/web/evolve/channels/tls/memory/sandbox/github), never the engine substrate; trims vs service-izes
- `pii-mcp-bridge.md` — the Hermes↔pii integration: pii-mcp crate (MCP server + client, separate binary, rmcp quarantine); MCP = integration surface, UDS JSON-RPC = control plane
- `023-llamacpp-granite-on-pi-process.md` — the full process of making llama.cpp + granite work on pi: router mode, the built-in `llamacpp` provider, flag order, ds4-bench results, and the in-process provider (spec 35)
- `pi-local-llamacpp.md` — how to call pi with local llama.cpp models (granite/laguna): the built-in `llamacpp` provider, router port 8082, and the critical `--provider`/`--model`-before-`-p` flag order
- `quality-gate-and-fresh-tree.md` — **the gate manual**: the eight gate steps and why each exists, the two-tree model (`quality-gate.sh` in your tree vs `fresh-gate` in a `git archive` export — believe the export), the `tmp_scratch` rule, and the free-space/retention guards

## 03-reference
- (empty)

## 04-development
- (empty)

## 05-operations
- `033-spec65-guardrail-violation-doc-format.md` — spec-65 guardrail violation: the worker changed the doc storage format (resolved)
- `105-runbook.md` — operator runbook: start/stop, health, metrics, logs, backup/restore, upgrade, wedged daemon

## 06-research
- [000-TOPIC-MAP.md](06-research/000-TOPIC-MAP.md) — the SSOT topic map: canonical / evidence / superseded per subject (replaces the per-file list formerly kept here)
- [000-CANDIDATES-INDEX.md](06-research/000-CANDIDATES-INDEX.md) — the researched-not-scheduled candidate index
- [030-OPUS-START-HERE.md](06-research/030-OPUS-START-HERE.md) — entry point for an opus review pass
- [043-opus-full-review-2026-10-06.md](06-research/043-opus-full-review-2026-10-06.md) — the full opus review (2026-10-06)
- [045-opus-rereview-2026-10-09.md](06-research/045-opus-rereview-2026-10-09.md) — the re-review (2026-10-09) and its ranked findings
- 046-051 — the 045 work-through evidence (mutation verification, five green mutants, the replay defect measured and fixed, the inline-handler measurements, the scratch-helper sweep) — see [06-research/](06-research/) and INDEX.md
- [052-production-readiness-2026-10-09.md](06-research/052-production-readiness-2026-10-09.md) — honest production readiness + the road to production (P0-P5)
- 041-BRIEF-full-review.md / 042-PII-REVIEW-STAGED-STATE-2026-09-28.md — brief and staged state for that review (historical)

## 99-archive
- (empty)
- 041-BRIEF-full-review.md — brief for the full opus review (derives from 031/032)
- 042-PII-REVIEW-STAGED-STATE-2026-09-28.md — staged state + exact resume command for that review
- **Consolidated (2026-09-28):** `002` now carries passes 1+3 · `012` carries `008`+`009` · `026` carries `010`+`011` · `037` carries `035`+`036` · `014` carries `004`+`017` (absorbed verbatim as `## [merged]` sections; originals in `99-archive/06-research-merged-2026-09-28/`).

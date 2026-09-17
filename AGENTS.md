# Pii — Project Agents Guide

**Pii** ("Pi Improved") — a Rust workspace unifying a terminal coding
agent, a personal assistant daemon, a reactive IoT/automation engine, and a
self-improving cognitive layer around one event-sourced daemon substrate.
Four binaries ship today (`pii`, `pii-mcp`, `pii-web`, `pii-evolve`); `pii`
(from `pii-cli`) is the primary entry point and embeds the daemon in-process.
This implements the architecture researched in `_docs/06-research/001-003-*.md`.

## SDD/TDD workflow (mandatory)

**Spec-driven, test-driven.** Every feature starts as a numbered spec in
`specs/`, implemented via pidag, validated by exit criteria (TDD).

```bash
# 1. Write the spec (Claude Code / opus = the planner). TDD Contract + Exit Criteria required.
# 2. Generate + validate DAG
pidag lint specs/01-xxx.md
# 3. Execute (pi + glm-5.3-flash workhorse on ollama-cloud)
pidag sdd specs/01-xxx.md --run --concurrency N
# 4. Review + commit. Update HANDOFF.md same-turn.
```

- **Model chain:** `[worker] default_model = "ollama-cloud/glm-5.3-flash:cloud"` in
  `.pidag/config.toml`. Slash-first form is REQUIRED (pidag split on `:` first broke
  the `:cloud` suffix — fixed in pidag).
- **MANDATORY — thinking OFF on the workhorse (verified 2026-09-02):** glm-5.3-flash and
  deepseek-v4-flash on ollama-cloud are 1M-context **REASONING** models with thinking ON by
  default. On a large one-shot implement prompt they burn the entire output budget on the
  reasoning trace and emit **zero content** (verified: glm 62,019 reasoning tokens, content 0).
  They must be called with `reasoning_effort="none"` on the `/v1/chat/completions` wire
  (verified fix: glm content 22,669 / reasoning 0; deepseek 21,899 / 0 at `max_tokens=6000`).
  NOTE: `pi --thinking off` does **NOT** forward `reasoning_effort` to ollama-cloud — the
  request builder must send it explicitly. See ollama/ollama#18121 (low/high mapping is
  model-fragile for glm; only "none" is reliable).
- **2 parallel workers:** `--concurrency 2`. Pidag's `PiBackend` sizes a client pool to
  concurrency; per-node session dirs (`_tmp/.pidag-worker-sessions/<node_id>/`) isolate
  parallel workers.
- **TDD enforced by pidag:** the `quality-gate` step runs `cargo fmt/check/clippy/test`
  and the `validate-iter{n}` step runs the spec's exit criteria. The SDD loop implements →
  quality-gates → validates → re-implements up to `max_iterations` (`.pidag/config.toml`
  `[sdd]`, currently 2: run 1 free model, run 2 escalates to a paid model, then STOPS —
  no further auto-escalation).

## Repos

- `_repos/` — read-only clones of the 5 reference repos (pi_agent_rust, zeroclaw,
  yantrikclaw, grok-build, hermes-agent). NEVER modify.
- `_docs/` — SSOT documentation (01-overview … 06-research).
- `_tmp/` — test artifacts, sessions, scratch.
- `crates/` — the Pii Rust workspace (18 crates + `xtask`; landed, not
  aspirational — Phase 0 completed 2026-09-02).

## Hard rules

- **NEVER modify `_repos/`** (read-only reference).
- **MANDATORY: `specs/` are READ-ONLY for the workhorse.** Authoring/editing/creating
  specs is EXCLUSIVE PLANNER work (Claude Code / opus orchestrator). Pidag `split`
  may *partition* an existing spec (into numbered parts) but never rewrite its
  content. If a pi/sonnet/workhorse run writes `specs/`, kill it, `git checkout`
  the touched specs, and return spec-creation to the planner. Workhorse only
  *implements* existing specs.
- **450-line file gate, traits-first, `#![forbid(unsafe_code)]`** — workspace-wide,
  no per-crate override exists today (`Cargo.toml:54`). Earlier research docs
  planned an unsafe carve-out for `pii-hw`/`pii-memory`; `pii-hw` was never built
  and `pii-memory` ships with the same forbid as every other crate.
- Tests under `_tmp/`, never `/tmp`.
- **Memory + HANDOFF are mandatory:** after any commit, update `HANDOFF.md` and store an
  agent-memory insight (topic `pii`).
- No AI-assistant mentions in commit messages. No `rm -rf`.
- If pidag itself breaks mid-run, fix pidag first, then resume the implementation.

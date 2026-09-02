# Pii — Project Agents Guide

**Pii** ("Pi Improved") — a single static Rust binary unifying a terminal coding
agent, a personal assistant daemon, a reactive IoT/automation engine, and a
self-improving cognitive layer. This implements the architecture researched in
`_docs/06-research/001-003-*.md`.

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
- **2 parallel workers:** `--concurrency 2`. Pidag's `PiBackend` sizes a client pool to
  concurrency; per-node session dirs (`_tmp/.pidag-worker-sessions/<node_id>/`) isolate
  parallel workers.
- **TDD enforced by pidag:** the `quality-gate` step runs `cargo fmt/check/clippy/test`
  and the `validate-iter{n}` step runs the spec's exit criteria. The SDD loop implements →
  quality-gates → validates → re-implements up to 3 iterations.

## Repos

- `_repos/` — read-only clones of the 5 reference repos (pi_agent_rust, zeroclaw,
  yantrikclaw, grok-build, hermes-agent). NEVER modify.
- `_docs/` — SSOT documentation (01-overview … 06-research).
- `_tmp/` — test artifacts, sessions, scratch.
- `crates/` — the Pii Rust workspace (when Phase 0 lands).

## Hard rules

- **NEVER modify `_repos/`** (read-only reference).
- **450-line file gate, traits-first, `#![forbid(unsafe_code)]`** except pii-hw/pii-memory
  (see pass-2 `001-...:128` and pass-3).
- Tests under `_tmp/`, never `/tmp`.
- **Memory + HANDOFF are mandatory:** after any commit, update `HANDOFF.md` and store an
  agent-memory insight (topic `pii`).
- No AI-assistant mentions in commit messages. No `rm -rf`.
- If pidag itself breaks mid-run, fix pidag first, then resume the implementation.

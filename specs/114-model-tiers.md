# Spec: 114 — model tiers and tier-aware tool selection

**Project**: /projects/pi-improved
**Phase**: Stage A of the vision plan (`057`), last row. Vision pillar 3's second half: *"tier-aware tool
selection for model sizes 0.5B→100B+"*.

## What exists (measured 2026-10-10; do not re-derive)
- Model resolution today is a flat chain: `--model` (one-shot) > `$PII_MODEL` > the daemon config's
  `[core] model` (`crates/pii-server/src/daemon/config.rs:292-298`).
- `grep -rl 'tier' crates/*/src` → **nothing**: the concept does not exist yet.
- Capability enforcement exists and attenuates properly (`Ctx`, `caps` in a graph, `child()` monotonically
  attenuating) — this is the mechanism the second half of this spec must reuse.
- The providers from spec 111 give the kinds to tier over; spec 110's measurements say what a tier costs.

## Requirements
R1. **A tier table in config**: `[[model_tier]]` with at least `name`, `model` (a provider spec from 111), and
    optional `tools` (the tool names this tier may use) and `max_tokens`/limits it may declare. Name the keys.
R2. **Resolution becomes tier-aware, with one documented precedence**: a node's explicit `model` > a node's
    `tier` > the run's model > the daemon config > `$PII_MODEL`. State it in the guide and pin it with a test
    per level (a test that the wrong level wins is the control).
R3. **No silent downgrade.** If a tier's provider is unavailable, the run fails/degrades LOUDLY naming the tier
    and the provider; it must never quietly run on another tier's model. (This is the same rule as spec 106's
    "operator surfaces must not lie", applied to model choice.)
R4. **Tier-aware tool selection is ENFORCEMENT, not a prompt.** A run on a tier whose `tools` list is set
    receives a `Ctx` attenuated to those tools: a tool outside the list is REFUSED by the capability layer
    (denial event + `PiiError::Denied`), regardless of what the prompt asks. Say in the report how the
    attenuation is implemented (the `Ctx` path, not a wrapper that filters after the fact).
R5. **The choice is observable**: the run records the resolved tier + model in its events/health (named events
    registered in `ALL`), so `pii health --json` / a trace answers "which tier did this run use?" and a
    trajectory (spec 103) can be grouped by tier later.
R6. **Tests, offline**: with the scripted provider as two "tiers" (two fixtures), prove: a node with a tier
    resolves to that fixture; the precedence levels in R2; an unknown tier refuses with a named error; the
    tool gate denies a tool outside the tier and allows one inside (both pasted); the record names the tier.
R7. **Docs**: `_docs/0X-guides/model-tiers.md` (the keys, the precedence, the tool gate, the no-downgrade rule)
    with a config sample, and `001`'s tiering line updated to what exists.

## Exit Criteria
- [ ] the tier table parses; a node with `tier = "fast"` runs on that tier's model (paste the trace naming
      the tier and the model)
- [ ] the precedence is proven at each level (paste the four cases)
- [ ] an unknown tier refuses, naming the tier and the available ones (paste)
- [ ] an unavailable provider for a tier does NOT fall back (paste the loud failure) — the negative control
      (the other tier still works) pasted
- [ ] the tool gate: a tool outside the tier's list is DENIED by the capability layer with an event, and one
      inside is allowed (paste both)
- [ ] the new event names are emitted and present in `ALL`
- [ ] `_docs/0X-guides/model-tiers.md` exists with the sample; `001` updated
- [ ] `bash quality-gate.sh` exits 0 (`PASS: lint-observability`, `PASS: coverage-floor` or the loud skip);
      `fresh-gate --worktree` prints `fresh-gate: PASS`; clippy `-D warnings` and `fmt --check` exit 0
- [ ] the report lists files changed, config keys, name consts, and how the attenuation is implemented

## Guardrails
- **The tool gate must live in the capability layer.** A filter applied after a tool is chosen is a lie about
  enforcement; if it cannot be done in `Ctx`, say so and stop rather than faking it.
- No silent downgrade, ever — a per-tier failure must be visible in health and in the run's outcome.
- Do not add a second model-resolution path: extend the existing chain and keep it in one place (a test that
  the one place is the only place: grep-based criterion is acceptable if it names the file).
- Scratch under `/projects/_tmp/`; keep it small, delete your own, no daemon left running.
- Do NOT commit; leave the tree for the orchestrator.

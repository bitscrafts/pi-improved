# Spec: 114 — model tiers and tier-aware tool selection

**Project**: <repo>
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
- Scratch under `<scratch>/`; keep it small, delete your own, no daemon left running.
- Do NOT commit; leave the tree for the orchestrator.

## Orchestrator verification (2026-10-10) — plus a flaky test root-caused and fixed
**The feature verified:** `cargo test -p pii-server --test model_tiers` → **10 passed**; my mutation
**`Ctx::tool` stops denying** (replace the denial with `Ok(())`) → `tool_outside_the_tier_is_denied_by_the_capability_layer_with_an_event` **FAILS**; restored → 10 pass. The gate really is in `Ctx::tool`
(`pii-core/src/ctx.rs:211`, before any dispatch) and `CapSet.tools` only ever intersects — the tier's tool list
is an attenuation, not a filter applied after the fact.
**Accepted deviation:** the worker kept the EXISTING precedence (`--model` > `$PII_MODEL` > config) rather than
the order my spec wrote from memory, and documented it. Preserving a documented rule over a spec sentence is the
right call; the spec sentence was the error.

### The flake the report waved away: root-caused, fixed, and proven
`crates/pii-graph/tests/resume.rs::resume_never_reinvokes_completed_node` failed once inside fresh-gate and the
report called it "timing-flaky under load; I did not investigate it". A check nobody can trust is worse than no
check, so it was investigated. Measured facts, in order:
1. **My first hypothesis was wrong.** "Wait for `b`'s `NodeStarted` in the JOURNAL instead of sleeping 300 ms" —
   the test then failed at *`b must have exactly one NodeInterrupted`*, and a journal dump explained why: the
   journal is **write-behind**, and one flush carried `NodeStarted(b)` AND `NodeFinished(b)` of the same tick
   together at 6.1 s. So polling the journal for "b started" can only return AFTER b finished; the cancel lands
   too late and the run completes.
2. **The honest signal is `a`'s own side effect in the log**, which the emit node writes immediately. The fix
   waits for that (bounded, 15 s deadline, 5 ms poll) and then cancels — inside b's 6-second wait at any load.
3. **Stability**: 24 runs of that test under **16 CPU hogs** → **0 failures** (`<scratch>/verify-resume-flake-fix.sh`,
   re-runnable).
4. **Teeth**: my first teeth-mutation (making `replay`'s `finished` set empty) left the test **GREEN** — a green
   mutant is a claim about the EDIT, and the edit was not at the effective site. The mutation at the PRIMARY site
   (make `replay` a no-op, so the resumed run learns nothing) goes **RED** with the exact evidence:
   `a must NOT be re-run: "a\na\nc\n" left: 2 right: 1`. Restored → 3 pass, source byte-identical.

### Lesson recorded (`docs/01-overview/DO-NOT-REPEAT.md`)
**Before polling a store from a test, read its flush semantics.** `NodeStarted` and `NodeFinished` can arrive in
the SAME flush, so "the journal shows X" is not "the engine has done X" — and a test that waits on the journal
measures the flusher, not the engine. Wait on the node's own side effect.

## 114 CLOSED (2026-10-10) — and the way it closed corrected a rule I had written
`fresh-gate` on the committed HEAD (`4846143`, my test fix included): **`quality-gate: ALL PASS` · `fresh-gate: PASS`**,
exit 0, 71G free.

Getting there took four attempts and each failure was informative:
1. Run 1 (`--worktree`, HEAD `1dff96d`): **clippy** — `this function has too many lines (90/80)`: a lint **my own
   test edit** carried, because after editing I ran tests/fmt/stability/mutations and skipped clippy.
2. Fix attempt A: extracted the wait into `wait_until_a_appended` — verified in the kept export: `too_many_lines`
   gone, but clippy immediately found **`needless_borrow`** at the two `read_to_string(&log)` sites inside the
   helper (double reference; `log` is already `&Path`).
3. Fix attempt B (`4846143`): clippy clean and the test green **in that export**.
4. Run 2 (`--worktree`): **five** steps failed — and the cause was not mine at all: spec 115's worker was
   mid-edit, and `--worktree` overlays the working tree, so its half-finished `cmd_soak.rs` came along
   (`error[E0277]: RpcError doesn't implement std::fmt::Display`).
5. Run 3 (**plain `fresh-gate`** = exactly HEAD): **ALL PASS**.

**Corrected:** my earlier claim (spec 112, `DO-NOT-REPEAT` 1.30) that "fresh-gate exports HEAD, so a worker cannot
have verified its own work" was WRONG: base is HEAD, but `--worktree` overlays the working tree's edited paths, so
a worker's `--worktree` run does cover its own uncommitted work. What fresh-gate cannot do is shield a run from
**another writer's** dirty tree. The guide now carries both modes with the check that tells whose red it is
(`git status --porcelain`).

**Standing ordering rule adopted:** verify a COMMIT with the plain form **before** dispatching the next worker.
A `--worktree` run overlapping a worker's edits measures the worker's work in progress, not the commit.


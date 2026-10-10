# Spec: 124 — the verifier: a candidate lesson must survive a shadow run before it is believed

**Project**: /projects/pi-improved
**Phase**: **Stage C** of `057`, second stage. Spec 123 extracts candidates that *propose*; this stage decides
whether a proposal is **true**. Nothing is promoted here either — 125 acts on verified results only, and that
separation is the whole point: a system that both proposes and acts has no gate at all.

## What exists (measured 2026-10-10)
- `pii-learn` produces candidates with evidence (run ids + artefact paths + counts) to the learn area; the CLI is
  `pii learn {scan,list,show}`.
- The engine can already run a graph in-process against its own store (`pii graph run … --db …`), and the daemon
  runs graphs with the full registry.
- There is **no consumer** of candidates, no verdict type, and nothing that re-tests a claim.

## Requirements
R1. **A shadow runner**: given a candidate, re-execute the thing the claim is about, **isolated from production
  state** — its own store, its own trajectory area, its own runtime dir, and no writes to the configured
  workspace. The shadow run must be able to use the REAL engine (that is the point: a verifier is not an opinion),
  but never the production store. Prove the isolation with a test that asserts the production store's key set and
  byte size are **unchanged** across a verification.
R2. **Verdicts are typed and honest**: `confirmed` (the claim reproduced, with the shadow run's own numbers),
  `refuted` (it did not reproduce — this is a **successful** verification, not a failure), `inconclusive` with a
  reason (no re-runnable evidence, budget exhausted, the artefact is gone/unreadable, the claim is not testable).
  Every verdict cites the evidence it used and the shadow run's id.
R3. **A budget, enforced**: a max number of shadow runs per candidate and a wall-clock bound per run, both
  configurable with documented defaults; hitting either yields `inconclusive` with the bound and the elapsed time
  — never a hang, never an unbounded loop.
R4. **Refutation is first-class and must be reachable**: a fabricated candidate (a claim that is not true of the
  evidence) must come back `refuted` — build the test by hand-writing such a candidate, because a verifier that can
  only confirm is not a verifier. The verdicts of a batch must be recorded per candidate (nothing averaged away).
R5. **The recovery rule**: if a candidate's evidence includes run ids whose artefacts no longer exist (spec 115's
  retention prunes trajectories), the verdict is `inconclusive` naming the missing paths — not `refuted`
  (absence of evidence is not evidence of absence) and not `confirmed`.
R6. **No promotion, no side effects**: the verifier writes only verdicts (into the learn area, keyed by candidate
  id) and events. It must not change a config, a threshold, a graph, or the production store. A test asserts that
  the production store and the configured workspace are untouched by a full verification pass (paste before/after).
R7. **Observability**: named events `learn.verification_started` / `learn.candidate_confirmed` /
  `learn.candidate_refuted` / `learn.candidate_inconclusive` (each carrying the candidate id and, where relevant,
  the numbers and the bound hit), registered in `ALL`, plus counters in `pii metrics` — and they must render, in
  the same place the extraction counters do (that integration was broken once already, spec 123; do not repeat it).
  A `learn` health field reporting the last verification pass.
R8. **CLI**: `pii learn verify <id>` and `pii learn verify --all` printing one line per candidate with its verdict
  and the shadow run it used; `pii learn show <id>` gains the verdict when one exists.
R9. **Docs**: extend `_docs/0X-guides/learning-loop.md` with the verification stage: the isolation model, the
  budgets, the three verdicts, why `refuted` is a success, and that nothing acts on a verdict yet.
  `001`'s pillar-4 status line updated (extraction + verification built, promotion still absent).

## Exit Criteria
- [ ] `cargo test -p <crate>` exits 0 with: a confirmed candidate, a **refuted** fabricated candidate, an
      inconclusive for missing evidence, an inconclusive for the run cap, and the isolation assertion
- [ ] the isolation test pastes the production store's key count and byte size **before and after** a verification
- [ ] a mutation per rule (isolation bypassed; budget removed; missing evidence misreported as refuted; verdict not
      recorded) — each pasted red, then restored
- [ ] `pii learn verify --all` over the two real candidates from spec 123 pastes the verdicts, with the shadow
      run id used for each, and for a `refuted` case the numbers that refute it
- [ ] the new names are in `ALL` and the counters move in `pii metrics` through a **live daemon** (paste both)
- [ ] `deps-check`, `net-check`, `doc-drift-check`, `lint-observability` exit 0
- [ ] `bash quality-gate.sh` exits 0; plain `fresh-gate` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the guide is updated; `001`'s pillar-4 line updated (paste it)
- [ ] the report lists files, names, budgets, config keys, and what was NOT verified

## Guardrails
- **The verifier must not be able to write to production state.** If isolation cannot be guaranteed for a case, the
  honest outcome is `inconclusive` with that reason, not a best-effort run against the real store.
- Never treat "the evidence is gone" as refutation (R5), and never treat "the claim feels right" as confirmation.
- No model in the loop for the verdict: the verdict comes from an executed run's own numbers.
- Do not promote anything. If you find yourself editing a config or a default to make a verdict possible, stop.
- Scratch under `/projects/_tmp/`, delete your own, no daemon left running. Do NOT commit.

## Orchestrator verification (2026-10-10)
**My controls:**
- `cargo test -p pii-learn --test verify` → **10 passed**.
- **Mutation (mine)**: bypassing the isolation refusal (`if false && overlaps(&scratch, &p)` at `verify.rs:371`) turns
  `a_scratch_dir_inside_protected_state_is_refused_not_run_best_effort` **red**; restored byte-identical → 10 pass.
- **Isolation, read at the artefact — and my first attempt at it was vacuous.** I ran `verify --all` against a
  daemon-made production store and compared `stat` before/after: identical. Then I noticed the command had
  **errored** (`no trajectory root (use --traj …)`) — so nothing had run and the "pass" proved nothing. Re-run with
  `--traj`: a **real** pass produced verdicts and the store was still `bytes=1056768 mtime=1791641216` before and
  after. The lesson is row 1.29's: a check must prove the thing happened, not merely not-fail.
- **End to end, and the honest failure mode**: with the repo's own `graphs/` dir the two candidates from spec 123
  come back `inconclusive (graph_unavailable)` — *"no graph named unsatisfiable in graphs"* — because the fixture
  graph that produced them is not a real graph. That is the correct verdict, not a failure: the verifier refuses
  with a precise reason instead of manufacturing a confirmation. (With the graph available, the worker saw both
  **confirmed** at 3/3 shadow runs.)
- **The counters render** (the integration I had to fix on spec 123): through a live daemon,
  `pii_learn_verdicts{verdict="inconclusive"} 2`, `{verdict="confirmed"} 0`, `{verdict="refuted"} 0`, and health
  reports `last_verification: at … candidates 2 confirmed 0 refuted 0 inconclusive 2`.

**Design confirmed at the source:** a candidate is confirmed only if the **same `dedupe_key` re-emerges** from a
shadow run of the real engine; `limit_hit` and `cost_outlier` are `inconclusive (not_testable)`; the scratch dir is
canonicalised and refused if it overlaps the learn root, the trajectory root, the graph dir or a `--protect` path
(**no best-effort run**); budgets `max_shadow_runs=5`, `shadow_run_ms=30000`.

**Kept honest:** a shadow run uses an empty capability set and `--input null` because a trajectory stores only the
input's **hash**, so the failure *class* is compared and not the cause — a graph that fails only under its original
input can reproduce, or fail to, for a different reason. A run that hits the wall-clock bound is **abandoned, not
killed**. The timeout path is covered only by a `shadow_run_ms=1` test, not against a genuinely long graph.

### CLOSED (2026-10-10, plain fresh-gate on the committed HEAD)
`fresh-gate` (exactly HEAD `b7b4ddb`): **`quality-gate: ALL PASS` · `fresh-gate: PASS`**, exit 0, 14 steps.
**124: 9/9.**


# Spec: 125 — promotion and lifecycle: the loop closes, and it can be taken back

**Project**: /projects/pi-improved
**Phase**: **Stage C** of `057`, last stage — and the first place in this pipeline where the system is allowed to
**change its own behaviour**. Everything before it proposed or judged; this acts, so it carries the tightest rules
in the plan: a whitelist, a rollback, a measurement, and an automatic reversal when the change is worse.

## What exists (measured 2026-10-10)
- `pii-learn`: candidates with evidence (123) and typed verdicts from shadow runs (124): `confirmed`,
  `refuted`, `inconclusive`. Nothing acts on a verdict.
- The daemon has a graph engine, a scheduler, a store, a ledger (`pii ledger`), and health/telemetry.
- The vision's pillar 4 says the loop must learn; today it can observe and judge but not act.

## Requirements
R1. **A promotion whitelist with exactly ONE kind to start: `quarantine_graph`.**
  Applying it makes the daemon **refuse to start that graph** with a named reason (`learn: graph <name> is
  quarantined by promotion <id>`), until it is reverted or expires. Rationale: a graph that fails every run should
  stop being scheduled; the change is visible, reversible and measurable. **Adding a second kind requires its own
  spec** — state this in the guide and make the code structured so a new kind is one addition plus tests (an enum
  match, not a string dispatch).
R2. **Only `confirmed` verdicts may be promoted.** A candidate with no verdict, a `refuted` one, or an
  `inconclusive` one is refused with that reason, counted, and no state changes. The verdict must be re-read from
  the learn area at promotion time (not taken from an argument).
R3. **Every promotion is recorded in the ledger** (the existing decision ledger, not a second one): id, kind, graph,
  the candidate id and its verdict, the evidence, who/what asked (a named actor), and the state transitions. The
  ledger is the audit trail; nothing may be applied without a record.
R4. **Rollback is built in, not planned**: applying stores the previous state; `pii learn revert <id>` restores it
  and records the reversal; a promotion carries a **TTL** (default 24 h, configurable) and on expiry it lapses —
  recorded as `expired` — with the graph usable again. A promotion whose rollback data is missing must never be
  applied.
R5. **Outcome measurement with automatic reversal.** A promotion declares the metric it expects to move and the
  window (e.g. failures of that graph over the next N runs, or a wall-clock window). When the window closes the
  daemon evaluates it: if the metric did not improve (or worsened), the promotion is **reverted automatically** and
  the outcome recorded with both numbers. `pii learn promotions` shows state, verdict, metric, before/after and the
  decision. A promotion that **cannot** be measured (no window, no metric) is refused at apply time — an unmeasured
  change is a guess wearing a promotion's clothes.
R6. **Refusals by construction**: a promotion may not touch secrets, the observability contract, the gate, capability
  scopes, or any file outside the promotion's own store area. State the mechanism (the whitelist plus a scope check
  on every write) and paste the refusal for an attempt that tries.
R7. **Observability**: events `learn.promotion_applied`, `learn.promotion_reverted`, `learn.promotion_expired`,
  `learn.promotion_refused` (each with the promotion id, kind, graph, metric and numbers where relevant), registered
  in `ALL`; counters by state in `pii metrics` **rendered through a live daemon** (this integration broke once
  already — spec 123's learn-area mismatch; do not repeat it); a `learn` health field for active promotions.
R8. **CLI**: `pii learn promote <candidate-id>`, `pii learn promotions` (list with state/metrics), `pii learn
  revert <id>`. Every command prints the ledger id it wrote.
R9. **Docs**: extend `learning-loop.md` with the promotion stage: the whitelist and why it is one kind, the TTL and
  rollback, the automatic reversal rule, the refusals, and an honest statement that the loop's only action today is
  **stopping a broken graph** — it does not edit code, config or thresholds. `001`'s pillar-4 line becomes the
  closed loop: propose → verify → promote → measure → revert.

## Exit Criteria
- [ ] `cargo test -p pii-learn` (or wherever it lives) exits 0 with: promotion refused without a verdict; refused
      for `refuted`; refused for `inconclusive`; applied for `confirmed`; the quarantine actually taking effect (the
      daemon/engine refusing that graph with the named reason); revert restoring it; TTL expiry restoring it; the
      outcome window reverting a promotion that made things worse; a refusal when the metric is unmeasurable
- [ ] a mutation per rule: verdict check removed; rollback data not stored; TTL ignored; outcome always judged
      "improved"; ledger record skipped; scope check bypassed — each pasted red, then restored
- [ ] an end-to-end run against a **real** confirmed candidate producing: ledger entry, `pii learn promotions` output,
      the quarantine refusal observed from the engine, then the revert observed
- [ ] the ledger shows the promotion and its outcome (paste `pii ledger tail` or equivalent)
- [ ] the counters move in `pii metrics` through a **live daemon**, and health reports the active promotion (paste)
- [ ] `deps-check`, `net-check`, `doc-drift-check`, `lint-observability` exit 0
- [ ] `bash quality-gate.sh` exits 0; plain `fresh-gate` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the guide is updated; `001`'s pillar-4 line updated (paste it)
- [ ] the report lists files, names, the whitelist, config keys/TTL defaults, and what was NOT verified

## Guardrails
- **A promotion is the only thing in pii that changes behaviour by itself.** Every rule above exists to make that
  safe: whitelist, verdict, ledger, rollback, TTL, measurement, auto-revert. Do not trade any of them for a simpler
  demo.
- Never promote from an argument — re-read the verdict from the learn area.
- Never apply an action whose effect cannot be measured, and never report an outcome you did not compute.
- Do not add a second promotion kind. Do not edit `specs/`. Do not commit.
- Scratch under `/projects/_tmp/`, delete only your own, no daemon left running.

## Orchestrator verification (2026-10-10)

**My controls:**
- `cargo test -p pii-learn --test promote` → **17 passed**; `cargo test -p pii-server --lib a_quarantined_graph`
  → **1 passed**.
- **Mutation (mine)**: making the engine ignore the quarantine (`quarantine_of` returning `None`, `runs.rs:133-136`)
  turns `daemon::tests::a_quarantined_graph_is_refused_by_the_engine_and_runs_again_after_revert` **red**; restored
  byte-identical → passes. The quarantine is enforced by the engine, not merely recorded.
- **I re-ran the worker's end-to-end script myself** (`/projects/_tmp/e2e125/run.sh`) and the whole loop reproduced
  on my run: `L1 refused (unmeasurable)` → `L2 applied` → `L3 reverted` → `L4 applied (6 s window)` →
  **`L5 auto_reverted` by the daemon with `actor=daemon before=Some(0) after=Some(0)`**, and the counters after
  `active 0 · auto_reverted 1 · refused 1 · reverted 1`. Earlier in the same run the daemon's log shows the
  engine's refusal with the named reason:
  `run.start.refused {"error":"capability denied: learn: graph denied-emit is quarantined by promotion p-…"}`.
  So propose → verify → promote → measure → revert is verified end to end by me, not only by the worker's capture.

**The R3 deviation, judged rather than waved through.** R3 asked for the *existing* decision ledger; the worker
found the CLI **cannot** write it (it is a redb the daemon holds open, and there is no `pii ledger` command) and
used an append-only `learn/ledger.jsonl` behind `pii learn ledger`. **Accepted**: the promotion trail is
append-only, carries the actor (`e2e:operator`, `daemon`) and the before/after numbers, and is the single trail for
promotions — the intent of R3 (an action cannot be applied without a record) is met. **Filed as a follow-up**: the
decision ledger and the promotion ledger are two trails today and should be unified (RESUME-REGISTER).

**The sharpest thing the worker did** was attack its own weak metric: a quarantine drives the graph's failures to
zero *by construction*, so "did the metric improve" is near-vacuous. It added the two checks that are not vacuous —
a promotion on a graph with no failures in the preceding window (0 → 0) must be **auto-reverted**, and a quarantine
that fails to take effect must show `after >= before` — and documented the weakness in the guide instead of hiding
it. It also found a real bug in its own scope check (it rejected any store path spelled with `..`), fixed it and
added the regression test.

**Kept honest:** implementation was written before its tests (**no RED observed before GREEN**; the six mutations,
each pasted red — verdict check, rollback, TTL, outcome always "improved", ledger skip, scope bypass — are the
failing evidence); a **resumed** run of a quarantined graph is not refused (only starts are); a CLI revert racing
the daemon sweep is last-writer-wins on one JSON file; with trajectories off every promotion measures 0 → 0 and is
reverted, which is the safe direction; and the "kept" outcome (`before=4 after=0`) is covered by a unit test but not
in the final captured E2E.


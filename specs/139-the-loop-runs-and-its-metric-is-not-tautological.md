# Spec: the loop runs unattended on real data, and its metric is not tautological

## Overview

The self-improvement pillar is well engineered and has never run on real data
(`_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §2.2 Q2, F8). Two things make it a gesture
rather than a loop:

1. **It needs a person.** Extraction and verification are triggered by hand; only the promotion is gated by a
   human (and it should stay that way).
2. **The only measured outcome is tautological under the only action.** The window's metric is satisfied by
   the quarantine itself, so the loop cannot distinguish "the caller adapted" from "we quarantined and
   nothing changed" — the measurement confirms the action rather than testing it.

The reviewer's smallest honest next step, which this spec adopts: run extraction and verification unattended,
keep promotion human-gated, and replace the outcome metric with one that **quarantine cannot satisfy by
construction**.

**Project**: <repo>
**Phase**: post-vision hardening (finding 059 F8, Q2)
**Source of the finding**: `_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §2.2 (Q2, F8), §2.3

## Requirements

### Functional
- **R1** Extraction and verification run **unattended**: a daemon timer or a daily `[[cron]]` graph performs
  `scan`/`extract` (whatever the current entry points are) and the shadow-run verification without an
  operator. Promotion stays **human-gated**: the loop must not promote by itself.
- **R2** The window's outcome metric is replaced by one that the action cannot satisfy by itself. Pick from
  the reviewer's candidates (or a better one, with the reason):
  (a) the promoted graph's **definition changed** — a new graph hash appears within the TTL; or
  (b) **refused starts per day fell** after the promotion. Either way, the metric must be computed from the
  system's own record, and it must read **unsatisfied** when nothing changed. A metric that cannot read
  unsatisfied is the defect this spec exists to remove.
- **R3** The **decision rule is pre-registered** and written where a reader finds it (the spec's closure, the
  `_docs` entry that describes the loop, and the config comment): over 14 days of real deployment,
  **≥ 3 confirmed candidates and the user judging ≥ 2 of 3 worth acting on** → design a second action;
  **0 confirmed candidates** → the input does not exist in practice and more actions are waste. Name the
  command that produces each number.
- **R4** The unattended path is **observable**: an event per extraction/verification run and a counter for
  candidates proposed (`pii metrics`), so a fortnight's silence is visible as a zero rather than as nothing.
  A counter that only ever reads zero is a finding — the report must say what the expected rate is.
- **R5** Tests: (a) the new metric reads **unsatisfied** with no definition change (the control that proves it
  can fail); (b) it reads satisfied after a definition change within the TTL; (c) the unattended trigger
  exists and is wired (a test can drive it without a human), and (d) promotion without human approval is
  still refused (the existing guard must keep its test).

### Non-Functional
- No new dependency. The trigger reuses the existing cron/timer mechanism (spec 128's cron surfaces), and if
  a new config key appears it is refused-by-unknown-fields-safe like its neighbours (`deny_unknown_fields`).
- A 30-day run of this design must not grow the ledger without bound: say what the retention is.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `the_outcome_metric_can_read_unsatisfied` (new) | a promotion with no definition change | metric reports unmet (RED before the metric exists) |
| `the_outcome_metric_reads_satisfied_after_a_definition_change` (new) | a changed graph within the TTL | metric reports met |
| `extraction_and_verification_run_without_an_operator` (new) | the trigger | candidates counted, no promotion attempted |
| the existing promotion-requires-approval test | unchanged | still green |

## Exit Criteria

- [ ] `cargo test -p pii-learn 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-evolve 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-server --lib 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] the decision rule is written down and its commands exist: `grep -rn '14 days' _docs --include=*.md | head -1` is non-empty

## Guardrails

- Do NOT let the loop promote without a human. If the honest fix seems to require it, **STOP and report** —
  that is a decision for the user, not for this spec.
- Do NOT keep the tautological metric alongside the new one "for continuity": two metrics where one is
  always green is the defect with extra steps.
- Do NOT add a second action (a new kind of change the loop can make). The measurement decides whether that
  is worth building, and the measurement needs 14 days first.
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit; scratch under `<scratch>/`.

## Closure

(outcome, the new metric's name and both readings, the trigger's name and how a test drives it, and the
pre-registered rule with its commands — filled in when the item is closed.)

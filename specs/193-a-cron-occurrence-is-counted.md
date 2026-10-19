# Spec: 193 — a cron occurrence is counted, and a multiplied one is an incident

**Project**: /projects/pi-improved · **Phase**: 070 §5's remaining instrument (its third bullet)
**Filed**: 2026-10-19 by the orchestrator. Specs 188/190/192 built 070 §5's first two bullets and each
carries an explicit scope boundary excluding this one (`specs/190-…:109-111`, `specs/192-…:58`); this is it.

## Overview

070 §5 (verbatim): *"each `[[cron]]` gets an occurrence assertion, `runs_per_occurrence == 1`, checked by
the runtime reader."* Measured 2026-10-18: the 09:00 occurrence produced **~1,846 runs** for **one** cron
entry, and nothing in the product noticed — the runtime reader opened 1,850 `run-stalled` incidents (one per
abandoned run) and no occurrence-level one (measured 2026-10-19: `pii incidents scan --source runtime`
over the production log → 1,850 `run-stalled`, 73 `daemon-restart-unexpected`, 5 `refusal-repeated`, 0
occurrence incidents).

The reader is `crates/pii-learn/src/incident_runtime.rs` (spec 156/160): it reads a JSON-lines daemon log
and clusters repeated refusals by kind. What it cannot see today is a FIRING: the log has no line that says
"cron `digest` started run `r…`". `crates/pii-server/src/daemon/cron.rs::start_cron` has both facts in hand
(the entry name, the run id from `engine.start_run`) and emits nothing.

## Requirements

- **R1 — the firing is a log event.** Add `CRON_FIRED: &str = "cron.fired"` to
  `crates/pii-observe/src/names.rs` (and its `ALL` registry entry — the registry test enforces it), emitted
  by `start_cron` on the `Ok(run_id)` arm only (a refused start stays `cron.start.refused`, spec 177), with
  fields `cron`, `graph`, `run`.
- **R2 — the reader clusters firings per cron.** In `incident_runtime.rs`, collect `cron.fired` timestamps
  per `cron` field and, for a cluster of **≥ 2 firings within `cfg.window_secs`**, open ONE incident
  `cron-occurrence-multiplied:<cron>` with `count` = firings, `first_seen`/`last_seen`, `opened_at` = the
  2nd firing. The threshold is 2, not the refusal threshold 3: the assertion is `== 1`, so a second firing
  inside one window is already the defect (070 §0.1's shape: 4 armed copies, all firing).
- **R3 — controls (each an assertion).**
  1. One firing is silent (the normal day).
  2. One firing plus `cron.rearmed` is silent — the re-arm is a different kind and is not a firing.
  3. Two firings **24 h apart** are silent (outside the window): a healthy daily cron is never an incident.
  4. Two firings 3 s apart open exactly ONE incident with `count == 2` (not one incident per run — the
     1,850-incident shape is the failure this must not repeat).
  5. A log written before this feature (no `cron.fired` lines — the production copy is the fixture) opens
     0 occurrence incidents: no retro-fitted alarms on history.
- **R4 — no store, no model.** The reader stays a pure function of the log text; no new dependency; the
  `IncidentConfig` shape is unchanged (the window is reused).

## TDD Contract

| Test (`crates/pii-learn/tests/incidents_runtime_cron.rs`) | Input | Expected |
|---|---|---|
| `one_firing_is_silent` | 1 `cron.fired` line | no occurrence incident |
| `a_rearm_is_not_a_firing` (control) | `cron.fired` + `cron.rearmed` | no occurrence incident |
| `two_firings_a_day_apart_are_silent` (control) | 2 firings, 86,400 s apart | no occurrence incident |
| `two_firings_three_seconds_apart_are_one_incident` | 2 firings, 3 s apart | exactly 1 incident, `count == 2`, fingerprint names the cron |
| `a_log_from_before_the_event_opens_nothing` (control) | a log copy with the production kinds and no `cron.fired` | 0 occurrence incidents |

`crates/pii-server`'s existing log-wiring test pattern gains one assertion in
`tests/log_wiring.rs`: a started cron entry emits exactly one `cron.fired` naming the entry and the run
(the existing recorder-based harness; no daemon process).

## Exit Criteria

- [ ] `cargo test -p pii-learn --test incidents_runtime_cron 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-observe --lib 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `grep -q 'CRON_FIRED' crates/pii-observe/src/names.rs`
- [ ] `cargo test -p pii-server --test log_wiring 2>&1 | tail -2 | grep -q 'test result: ok'`

## Guardrails

- **Do NOT touch the store, the scheduler or the timer wheel.** This is an observability addition: one
  event on the start path, one clustering rule in the reader.
- **Do NOT reuse the refusal threshold.** `== 1` is the contract; the rule fires on the second firing in a
  window, and the tests pin the two controls that keep a healthy day silent.
- **Do NOT emit `cron.fired` on the refusal arm** — that is `cron.start.refused`'s job, and one refused
  start must not read as a firing.
- **450-line gate**: `incident_runtime.rs` is 284 lines; keep the clustering helper inside it or in a
  submodule, never past the gate by deleting comments.
- **A green `cargo test` is not a green gate**: `bash quality-gate.sh --routine` must pass on the tree this
  leaves.
- **Mutation controls to record in the Closure**: (a) emit `cron.fired` on both arms → control 1's sibling
  test reddens; (b) drop the `cron` field from the cluster key → the per-cron fingerprint test reddens;
  (c) set the occurrence threshold to 3 → `two_firings_three_seconds_apart_are_one_incident` reddens.

## Closure

(To be filled by the orchestrator after independent verification: the criteria's own output, the mutation's
red output, the byte-identical restore, and what this does and does not close in 070 §5.)
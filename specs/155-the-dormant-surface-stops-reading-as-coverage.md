# Spec 155: the dormant surface stops reading as coverage

**Project**: /projects/pi-improved · **Phase**: road to production, M7 (the last item)
**Why**: 059 measured four pillars and found one with a real user. Today the tree still *looks* fuller than the machine
is: three channel kinds with zero posts ever, and a hardware HAL whose steps sit in the gate as if they were coverage.
A green gate that implies hardware nobody has exercised is the most expensive kind of decoration — it makes a
non-existent product look finished. This spec removes that reading, without deleting the code.

## Requirements

- R1. The **default** channel configuration names only the channels that are actually used (`cli`, `discord`). The
  `telegram`, `matrix` and `webhook` kinds stay implemented and selectable, but they are no longer in the default set,
  and the config entry point carries one comment line recording the measured usage: "0 posts ever".
- R2. The gate's own summary marks `clippy(hal-gpiod)` (and the `hal-gpiod` feature steps) as **UNEXERCISED**: 117
  lines that have never run on hardware. The step must still run when the feature is enabled; the summary must not let
  its presence read as hardware coverage.
- R3. `_docs/05-operations/m7-retiring-the-dormant-surface.md` is updated to record this spec as the implementation of
  its rows 3 and 4 (the doc is the SSOT for these decisions; today it says "mine, load-free").
- R4. Nothing is deleted: the kinds and the feature remain in the code, and a test asserts they still resolve when
  explicitly requested. Removing a capability is a different decision from removing a misleading default.

## TDD Contract

| Test name | Input | Expected |
|---|---|---|
| `the_default_channel_set_names_only_used_kinds` | the default config as loaded | exactly `cli`, `discord` |
| `a_dormant_kind_still_resolves_when_requested` | `telegram` asked for explicitly | it resolves (control: presence of code is not removed) |
| `the_gate_summary_marks_gpiod_unexercised` | the gate's summary text | a line naming `hal-gpiod` and `UNEXERCISED` |

## Exit Criteria

- [ ] `grep -rE 'telegram|matrix|webhook' <default-config-path> | grep -v '^.*#' | wc -l` returns 0 (no dormant kind in the default set)
- [ ] `grep -q '0 posts ever' <default-config-path>`
- [ ] `<gate-summary-command> | grep -q 'hal-gpiod.*UNEXERCISED'`
- [ ] `cargo test -p pii-cli --test channels 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `bash scripts/quality-gate.sh` is green

## Guardrails

- Do NOT delete the `telegram`/`matrix`/`webhook` kinds or the `hal-gpiod` feature: this spec changes defaults and
  summaries, never capabilities.
- Do NOT touch `AGENTS.md`, `_docs/06-research/**` or another item's files.
- Do NOT weaken any existing gate step to make the summary pass; if a step genuinely cannot be exercised, say so in the
  summary instead of removing it.
- Do not commit — the orchestrator commits.

## Closure
- **2026-10-16, orchestrator: four `verify-failed` rows, none of them this spec's product** (which is committed in
  `7debbdf`). Three of them (2026-10-15 04:12, 07:25, 07:40) ran `bash scripts/quality-gate.sh` and answered in ~7 s:
  **that path does not exist** — the gate script lives at the repo root — so the criterion was answered by the
  shell, not by the gate. The row now reads `bash quality-gate.sh` (fixed in `850cc39`/`f02b465`), and this closure
  carries all five of this spec's criteria: the row had kept only the gate one, four criteria short.
- The fourth (2026-10-16 00:30) ran the real gate and died on `FAIL: clippy (llamacpp)` — bindgen could not find
  `libclang`, and the box had also lost `cmake`, `make` and `libssl-dev`, plus `cargo-llvm-cov` for the
  coverage-floor step. That is the environment, not this spec; the list and the measured evidence are in spec 160's
  Closure (restored 2026-10-16, and the llamacpp step re-measured rc=0).
- **Criteria 1-4, run by the orchestrator on the committed tree, rc=0:** the default config names no dormant kind
  (`grep -rE 'telegram|matrix|webhook' examples/pii.production.toml` → 0 non-comment lines); the config carries the
  measured `0 posts ever`; `quality-gate.sh --summary` marks `hal-gpiod` UNEXERCISED; `cargo test -p pii-cli --test
  channels` is green.
- **Criterion 5 (`bash quality-gate.sh` green) is the one still owed**, and it is the heavy one: the local gate
  measures the coverage floor by default, which is ~18 GB of instrumented build against the 33 GB free on
  `/projects` (95% used). It is owed to the loop, not to the worker — no dispatch is required.
- **2026-10-16 12:15Z, orchestrator: parked again (5 attempts), and the owed criterion is now red on a defect in the
  PRODUCT, not in this spec.** The gate's failing step is `boss_watch_reaches_detect_after_resume`, which stops
  growing its journal at a fixed entry under CPU load — reproduced twice, the second time with a 32-hog control, and
  recorded in `_docs/05-operations/DEFECT-2026-10-16-boss-watch-stalls-under-load.md`. A paid review was answering
  that defect's narrow question while this row was re-read. Criteria 1-2 of this spec re-run by hand on today's tree
  (`1d0066f`): 0 non-comment dormant kinds in `examples/pii.production.toml`, and the config carries the measured
  `0 posts ever`. Criteria 3-5 need the gate. **A re-dispatch before the boss-watch fix lands would spend on the
  same mechanical failure** — that is why this item stays parked and why the fix spec comes first; nothing about
  this spec's own surface is in question.

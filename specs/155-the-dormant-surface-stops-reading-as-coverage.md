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

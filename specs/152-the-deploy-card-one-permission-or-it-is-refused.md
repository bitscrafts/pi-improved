# Spec 152: the deploy card — one permission, five fields, or it is refused

**Project**: /projects/pi-improved · **Phase**: road to production, M4 (Opus 061 order 153 -> 150 -> 154 -> 151 -> 152)
**Why now**: 061 measured the operator's involvement per incident and named the single gate that must remain human: the
deploy. The deploy scripts exist and have been exercised **0 times**. What is missing is not a mechanism but a *decision
shaped so it takes five seconds*: everything a person needs, nothing they have to ask for, and a refusal when a field
is absent rather than a question asked in the dark.

## Overview

A card is rendered for one pending promotion: it carries (1) the incident and its count, (2) the diff stat and the
branch, (3) the two runs of the test that proves the change (red on base, green on branch), (4) the fresh-gate verdict,
(5) the rollback command already rehearsed in dry-run. Missing any field, the card is **refused** — printed as a
refusal, never as a question. Approval applies exactly the artefact the card describes, by revision, and nothing else.

## Requirements

- R1. A card is produced from the promotion's own records, never from prose: incident id, fingerprint and count from
  the incident record (spec 150); branch and diff stat from git; the proving test's two runs and the gate verdict from
  the run that produced them (spec 151); the rollback command from the deploy scripts.
- R2. **Refusal on incompleteness**: if any of the five fields cannot be sourced, the card is refused with the missing
  field named, and no approval path is offered. A card that cannot be refused is not a gate.
- R3. Approval is bound to the revision: the applied artefact must equal the revision the card names. A mismatch
  refuses and records the refusal as an event with the expected and the found revision.
- R4. The rollback line is executed **dry-run first** and its output is part of the card, so approval never requires
  trusting an untested way back.
- R5. Every card and every decision is recorded where the metric can read it (spec 153's operator records): one line
  per card with timestamp, fields present, and the person's answer.
- R6. The card is a CLI surface a person can run in a terminal and an agent can post: one command to render, one to
  approve, one to refuse — no interactive UI required.

## TDD Contract

| Test name | Input | Expected |
|---|---|---|
| `a_card_missing_any_field_is_refused` | five cards, each with one field removed | each refused, the missing field named, no approval offered |
| `approval_is_bound_to_the_revision` | card for rev A, approve rev B | refused, refusal event records expected/found |
| `the_rollback_line_is_rehearsed_before_the_card` | a card with no dry-run output | card refused (dry-run is a field, not a courtesy) |
| `one_card_one_operator_line` | a card and an approval | exactly one line in the operator records for that card |

## Exit Criteria

- [ ] `cargo test -p pii-cli --test deploy_card 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `bash deploy/rollback-pii.sh --dry-run` exits 0 and its output is embedded in a rendered card
- [ ] a card rendered with one field removed exits non-zero and prints `REFUSED: missing <field>`

## Guardrails

- Do NOT invent a field: if the proving test's runs are absent, refuse — do not fall back to "the worker says".
- Do NOT apply by anything other than the revision the card names.
- Do NOT make the card an interactive prompt; it must be usable headless (the loop posts it, a person answers it).
- Do NOT delete or overwrite the previously installed artefact: move it aside, dated, as the install script already does.

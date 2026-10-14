# Spec 152: the deploy card — one permission, five fields, or it is refused

**Project**: /projects/pi-improved · **Phase**: road to production, M4 (Opus 061 order 153 -> 150 -> 154 -> 151 -> 152)
**Why now**: 061 measured the operator's involvement per incident and named the single gate that must remain human: the
deploy. The deploy scripts exist and have been exercised **0 times**. What is missing is not a mechanism but a *decision
shaped so it takes five seconds*: everything a person needs, nothing they have to ask for, and a refusal when a field
is absent rather than a question asked in the dark.

## Overview

A card is rendered for one pending promotion: it carries (1) the incident and its count, (2) the diff stat and the
branch, (3) the two runs of the test that proves the change (red on base, green on branch), (4) the fresh-gate verdict,
(5) the rollback command already rehearsed in a scratch prefix. Missing any field, the card is **refused** — printed as a
refusal, never as a question. Approval applies exactly the artefact the card describes, by revision, and nothing else.

## Requirements

- R1. A card is produced from the promotion's own records, never from prose: incident id, fingerprint and count from
  the incident record (spec 150); branch and diff stat from git; the proving test's two runs and the gate verdict from
  the run that produced them (spec 151); the rollback command from the deploy scripts.
- R2. **Refusal on incompleteness**: if any of the five fields cannot be sourced, the card is refused with the missing
  field named, and no approval path is offered. A card that cannot be refused is not a gate.
- R3. Approval is bound to the revision: the applied artefact must equal the revision the card names. A mismatch
  refuses and records the refusal as an event with the expected and the found revision.
- R4. The rollback line is **rehearsed** first and that rehearsal's own output is a field of the card, so approval
  never requires trusting an untested way back. The rehearsal is the one this repo already runs green
  (`scripts/checks/deploy-dry-run.sh`, which points `PREFIX`/`CONFIG_DIR`/`STATE_DIR`/`UNIT_DIR`/`SYSTEMCTL` at a
  scratch prefix): `deploy/rollback-pii.sh` parses **no arguments**, so no `--dry-run` mode exists, and a rehearsal
  must never be pointed at the host's real `$HOME/.local/bin` — it either restores a dated `.old-*` set or it refuses.
- R5. Every card and every decision is recorded where the metric can read it (spec 153's operator records): one line
  per card with timestamp, fields present, and the person's answer.
- R6. The card is a CLI surface a person can run in a terminal and an agent can post: one command to render, one to
  approve, one to refuse — no interactive UI required.

## TDD Contract

| Test name | Input | Expected |
|---|---|---|
| `a_card_missing_any_field_is_refused` | five cards, each with one field removed | each refused, the missing field named, no approval offered |
| `approval_is_bound_to_the_revision` | card for rev A, approve rev B | refused, refusal event records expected/found |
| `the_rollback_line_is_rehearsed_before_the_card` | a card with no rehearsal output | card refused (the rehearsal is a field, not a courtesy) |
| `one_card_one_operator_line` | a card and an approval | exactly one line in the operator records for that card |

## Exit Criteria

- [ ] `cargo test -p pii-cli --test deploy_card 2>&1 | tail -2 | grep -q 'test result: ok'` — the four tests of the
      TDD Contract, green on the tree the worker leaves
- [ ] `cargo test -p pii-cli --test deploy_card a_card_missing_any_field_is_refused -- --exact 2>&1 | grep -q '1 passed'`
      — exactly that test ran and passed: a card missing any one of the five fields is refused, the field is named,
      and no approval path is offered (a filter that matches nothing is not a green)
- [ ] `cargo test -p pii-cli --test deploy_card the_rollback_line_is_rehearsed_before_the_card -- --exact 2>&1 | grep -q '1 passed'`
      — a card carrying no rehearsal output is refused
- [ ] `bash scripts/checks/deploy-dry-run.sh 2>&1 | tail -1 | grep -q 'DRY-RUN OK'` — the rehearsal the card's
      rollback field must come from, driven against a scratch prefix, including its own near-miss: a rollback with
      nothing to restore refuses rather than reporting success
- [ ] `cargo xtask fresh-gate` on this spec's commit is `ALL PASS` (the plan row's gate column runs it)

## Guardrails

- Do NOT invent a field: if the proving test's runs are absent, refuse — do not fall back to "the worker says".
- Do NOT apply by anything other than the revision the card names.
- Do NOT make the card an interactive prompt; it must be usable headless (the loop posts it, a person answers it).
- Do NOT delete or overwrite the previously installed artefact: move it aside, dated, as the install script already does.
- Do NOT rehearse the way back on the host: the rehearsal points every path at scratch. `deploy/rollback-pii.sh`
  parses no arguments, so "dry-run" against the host's real prefix is a live rollback.

## Closure

_(open — the implementing cycle fills it with what it measured, per the house shape.)_

One correction was made **before** the first dispatch, and it is recorded here so it is not mistaken for a criterion
softened to let an item pass (spec 141's criterion 4 was, and was reverted). As first written, exit criterion 2 asked
`bash deploy/rollback-pii.sh --dry-run` to exit 0. Measured on this host 2026-10-14, that command cannot mean what the
criterion needed: the script parses **no arguments at all**, so on a host that had a prior install the "dry-run" would
have performed a real rollback of `$HOME/.local/bin`; and with no `${PREFIX}.old-*` present it aborts rc=1
("ABORT: no ... directory to restore from"), which is this host today — the deploy has run 0 times. The behaviour the
criterion existed to bind — the way back is rehearsed before approval, and a card without that rehearsal is refused —
is now bound by the rehearsal this repo already runs green (`scripts/checks/deploy-dry-run.sh`, whose near-miss is a
second rollback refusing) plus the two named tests. Nothing was weakened; an unmeetable command was replaced by the
binding one.

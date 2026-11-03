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

Filled 2026-11-03. The card has now been used **four times**, and each use is a card filled from a measured PASS, not
from a judgement:

- `_tmp/deploy/card-e4bb787ed0fd.json`, `card-e2e6c765cf93.json`, `card-177e4efbcf07.json` — 2026-11-02, the three
  deploys that took the host probe from 0/5 to 5/5.
- `_tmp/deploy/card-085369902766.json` — 2026-11-03, the 0.2.0 deploy. Its fields: revision `085369902766`, the
  full twenty-step gate with 20 passed / 0 failed and `fresh-gate: PASS` (`/tmp/gate-rel5.log`), the four proving
  tests with their counts, the previous binary's version, revision and sha256 prefix, and the way back.

**What the four uses measured, in the order the card requires.** The gate ran on the COMMITTED revision in a clone
and failed five times before it passed; each failure was a real defect (`lint-observability`, `net-check`, a 462-line
module, a JSON fixture that was not JSON, a backoff that could never exceed 16 s). Filling the card from any one of
those red runs would have installed a revision that misbehaved in production, which is precisely the field this spec
exists to make impossible to skip.

**The way back, rehearsed and then needed.** The rollback is a copy of `~/.local/bin/pii.rollback-177e4efbcf07`
(11262488 bytes, sha256 `89ed1ff3…`) plus a restart. Note the card's guardrail was right and the installer is not:
`install-pii-user.sh` replaces `~/.local/bin/pii` in place and keeps the seven stores, but it does **not** keep the
previous binary, so the dated aside copy has to be taken before `--apply`. That gap is named here rather than
silently absorbed into the next upgrade; it is one `cp` in the installer and a test, and it belongs to its own row.

**Verification after the act**: `HOST-CONFORMANCE: 5 passed, 0 failed`, and `pii health` on the host printed
`pii 0.2.0 (085369902766, x86_64-unknown-linux-gnu)` with `client: 085369902766` — the identity field, so nobody has
to take anyone's word for what is live.

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

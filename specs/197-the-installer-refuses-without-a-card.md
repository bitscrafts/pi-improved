# Spec: 197 — the installer refuses without a deploy card for the revision it builds

**Project**: /projects/pi-improved · **Phase**: 070 step 5, the one process gap a patch can close
**Filed**: 2026-10-19 by the orchestrator from the measured deploy record, not from a theory.

## Overview

070 step 5 says "one deploy card for every install". It has never been enforced; it depends on discipline,
and the discipline failed. Measured on this host (`ls -l _tmp/deploy/`, 2026-10-19 12:2xZ):

| fact | evidence |
|---|---|
| three cards exist, named by the **seven**-character revision | `_tmp/deploy/card-0e34719.json`, `card-ebdd436.json`, `card-b61c659.json` |
| the deployed revision is quoted with **twelve** characters | `pii health` / the unit: `121fb6de67e1` |
| the installer has **no** card check | `grep -n card deploy/install-pii.sh` matches one comment about the revision suffix, no conditional |
| the installs outran the cards | 070's ledger: 13 starts vs 2 installs, 11 aside dirs vs 3 cards |

So the deploy with no card is indistinguishable, after the fact, from one that had a card: `PREFIX.old-<ts>`
directories accumulate and nothing ties a binary to the proof that authorised it.

The card convention already exists in the tree and must not be re-invented: `crates/pii-cli/src/cmd_deploy_card.rs`
writes and reads `_tmp/deploy/card-<rev>.json`, and every card on this host is keyed by the **7-char** short
revision while the binary reports the **12-char** one. Both forms are legitimate keying; a check that accepts
only twelve would refuse every deploy whose card was written the way all existing cards were written. The
installer therefore accepts either form for the HEAD it is about to build.

## Requirements

- **R1 — the gate sits with the other preconditions.** In `deploy/install-pii.sh`, after the argument/default
  block and **before** the build/export step and before anything is moved, the script checks for a deploy card
  for the revision it is about to build. Nothing is built, moved or written when the card is missing; the
  refusal uses the existing `abort()` (stderr, exit non-zero, "nothing was changed").
- **R2 — the revision is HEAD's, in both keyings.** Compute `REV7="$(git rev-parse --short=7 HEAD)"` and
  `REV12="$(git rev-parse --short=12 HEAD)"` from the repo the installer builds (`$REPO`). The gate is
  satisfied when `card-$REV7.json` **or** `card-$REV12.json` exists and is non-empty.
- **R3 — the refusal names what is missing.** The message contains `no deploy card for revision <REV7>` and
  both expected paths (`card-<REV7>.json`, `card-<REV12>.json`), so the operator can write the card without
  reading the script.
- **R4 — the card directory is overridable, for the check only.** `DEPLOY_CARD_DIR` defaults to
  `$REPO/_tmp/deploy`; nothing else about the default changes. This exists so the check can plant and withhold
  a card in a scratch directory instead of touching the operator's real cards.
- **R5 — a card present is a pass, and the check proves it.** `scripts/checks/installer-requires-a-card.sh`
  drives the **real** installer as a subprocess, twice, with `SKIP_BUILD=1` and scratch `PREFIX`, `CONFIG_DIR`,
  `STATE_DIR`, `DEPLOY_CARD_DIR` under `_tmp/` (created by the check itself):
  - `--case=a` (no card): the installer exits non-zero and its output contains `no deploy card for revision`
    and both expected filenames. It must exit **before** the network precondition, so the case needs no host.
  - `--case=b` (control, card present): a non-empty `card-<REV7>.json` is planted in the scratch card dir; the
    installer's output contains **no** card refusal. What it does afterwards (curl, graphs, missing binaries)
    is not this check's question: it may abort later and that is a pass for this control.
  - With no argument the check runs both cases and exits non-zero if either fails; it prints one line per case
    (`case a: PASS|FAIL …`).

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `installer-requires-a-card.sh --case=a` | scratch empty `DEPLOY_CARD_DIR`, `SKIP_BUILD=1`, scratch `PREFIX`/`CONFIG_DIR`/`STATE_DIR`, real `deploy/install-pii.sh` | rc≠0; output has `no deploy card for revision <REV7>` and `card-<REV7>.json` and `card-<REV12>.json`; no network call made |
| `installer-requires-a-card.sh --case=b` (control) | same, plus a non-empty `card-<REV7>.json` in the scratch card dir | output contains no `no deploy card for revision`; a later abort is acceptable |
| `bash -n deploy/install-pii.sh` | the installer | rc=0 |
| mutation (Closure) | delete the card block from `deploy/install-pii.sh` | `--case=a` reddens (rc=0 or no refusal text); restored byte-identical, then green again |

## Exit Criteria

- [ ] `bash scripts/checks/installer-requires-a-card.sh --case=a`
- [ ] `bash scripts/checks/installer-requires-a-card.sh --case=b`
- [ ] `bash -n deploy/install-pii.sh`

## Guardrails

- Do **NOT** add a bypass flag (`FORCE=1`, `SKIP_CARD=1`, …). The only way past the gate is a card file; a flag
  reproduces exactly the discipline hole this spec closes.
- Do **NOT** delete, move or rewrite the operator's cards in `_tmp/deploy/`. The check uses its own
  `DEPLOY_CARD_DIR` under `_tmp/` and creates it.
- Do **NOT** put the gate after the build or after the `mv` of existing binaries: a refusal that has already
  moved files aside is not a refusal.
- Do **NOT** make `--case=a` depend on the network, a built binary or the llama host: it must abort before those.
- Do **NOT** change the card's filename convention or the 12-char revision string; accept both keyings (R2).
- The check must **invoke** the installer and assert on its output. It must not `grep` the installer's source
  and it must not re-implement the check: a criterion that reads the script's text certifies a comment.
- Do **NOT** weaken the gate, the line budget or any existing control to make this pass. If the premise is
  wrong (e.g. cards on this host turn out to be keyed differently, or the installer cannot read `git` in the
  clean export), **STOP and report the mismatch** instead of forcing the change.
- `bash quality-gate.sh --routine` must pass on the tree this leaves.

## Closure

_(worker: record the three criteria green, the mutation that reddened case a and the byte-identical restore
with `sha256sum` before/after, and anything the deploy path still does not prove.)_

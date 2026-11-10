# Spec: The review becomes queue rows

## Overview

A review's numbered items must reach the queue without a human converting them by hand. Tonight six items from docs 064 and
065 sat open until that conversion was done by a person (`5bf0db7`: specs 161-166, rows A28-A33). The conversion step was
the human, and the human is the unreliable part.

The design is doc 066 (`_docs/06-research/066-opus-review-to-queue-automation-2026-10-17.md`), and its owner decision is not
negotiable from here:

- **Owner: `scripts/autonomy/plan_import.py`** — stdlib Python, versioned in the repo, selftested with the loop, **invoked
  by the tick under the cycle lock**, and reported by `ctl.sh status` as an owned producer.
- **Not the skill lane's proposal step.** The lane is an *agent*; an agent converting prose to rows paraphrases. That is
  the measured defect class: 066 found my own hand conversion had "161 inverted, 162 drifted, 163 based on a false premise".
  The lane keeps judgement only; it never writes `plan.tsv`.
- **Not a `ctl.sh` verb.** `ctl.sh` lives outside the repo and is unversioned, 158's contract covers only what can refuse,
  spend, start, stop or restart work, and 063 Q4 froze its mutating verbs.

**Project**: <repo>

## Requirements

Provenance (066 Q2): a review **author** writes a machine-readable *queue block* citing the review path, the item number,
and the evidence `file:line`. The importer converts blocks into `plan.tsv` rows; it never invents one.

The importer must REFUSE, loudly and with a non-zero exit, each of these:

- an item with no evidence `file:line`
- a row that already exists (same spec path)
- a decision disguised as a mechanical item
- a citation whose `file:line` has moved or vanished

Mechanical versus decision (066 Q3): the rule that lets an item be queued, the rule that escalates it to the human as one
permission, and both near-miss controls, are specified in 066 and must be implemented as written there.

## Exit Criteria

- [ ] `python3 scripts/autonomy/plan_import.py --self-test`
- [ ] `python3 -m pytest scripts/autonomy/test_plan_import.py -q`
- [ ] `grep -q 'plan_import' scripts/autonomy/tick.sh`

## Guardrails

- Do NOT weaken the gate, the write-amplification budget, or any existing control to make this pass.
- Do NOT write `plan.tsv` from the skill lane or from `ctl.sh`.
- Do NOT commit; the loop's orchestrator reviews the diff against this spec and commits.

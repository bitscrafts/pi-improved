# Spec: a two-tier gate

## Overview
Tonight a gate run cost forty-five minutes because the FRESH EXPORT was paid on every item: each run compiles the whole
workspace into directories created empty for that run. The fresh property exists to catch a dependency the warm tree hides
— a real CI property that belongs at a RELEASE boundary, not on every iteration. Routine runs should use the repo and the
warm target; the fresh export should run only before a deploy.

**Project**: /projects/pi-improved

## Requirements
- `quality-gate.sh --routine` runs in the repo against the warm `_tmp/gate-target` (minutes, not tens of minutes).
- `quality-gate.sh --fresh` keeps tonight's behaviour for the deploy boundary.
- The loop uses `--routine` per item and `--fresh` only when an item is about to be deployed; the deploy card records which
  one produced the verdict it quotes.

## Exit Criteria
- [ ] two consecutive `quality-gate.sh --routine` runs: the second finishes in under a third of the first
- [ ] `grep -c 'fresh' quality-gate.sh` shows both modes wired, and `--fresh` is still what the deploy card calls

## Guardrails
- Do NOT remove `--fresh`; the release boundary keeps it.
- Do NOT commit; the loop's orchestrator reviews and commits.

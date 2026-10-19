# Spec: the deployed revision has a gate

## Overview
070 step 4 is RED: the last full fresh-export PASS was recorded on 9045484, while production runs 420fe8a7e946. A gate on HEAD is not a gate on the running binary. 197 made the card mandatory; the card quotes a gate, so the gate must exist for the revision it names.

**Project**: /projects/pi-improved

## Requirements
A fresh-export gate run on the EXACT revision now deployed, its verdict recorded in _tmp/autonomy/logs/ and quoted verbatim in that revision's card. If the gate fails, the card is invalid and the deploy is not releasable.

## Exit Criteria
- [ ] grep -q 'fresh-gate: PASS' _tmp/autonomy/logs/gate-run3-20261017T020154Z.log

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT record a PASS from a different revision than the deployed one.

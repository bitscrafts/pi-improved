# Spec: the installer refuses without a card

## Overview
070 step 5: 1 card for 6 installs. The rule exists in prose and nothing enforces it, so it depends on discipline — which failed. Make it code.

**Project**: /projects/pi-improved

## Requirements
deploy/install-pii.sh aborts before building unless _tmp/deploy/card-<rev>.json exists for the revision it is about to build (git rev-parse --short=12 HEAD), and prints the expected path when it refuses.

## Exit Criteria
- [ ] grep -q 'card-' deploy/install-pii.sh
- [ ] bash -n deploy/install-pii.sh

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT delete an existing card; a refusal is the whole point.

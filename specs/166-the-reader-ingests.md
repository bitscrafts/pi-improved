# Spec: the incident lives in the store, with an id

## Overview
Tonight the reader found 1,425 identical refusals on the daemon's real log and opened an incident — as a printed line.
Ingest it, so the incident has an id and a fingerprint in the store and the card's incident field is a query rather than
a quote.

**Project**: /projects/pi-improved

## Exit Criteria
- [ ] `cargo test -p pii-learn --test incidents_runtime_production`

## Guardrails
- Do NOT weaken an existing budget or control to make this pass; change the design instead.
- Do NOT commit; the loop's orchestrator commits.

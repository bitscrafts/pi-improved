# Spec: bind an ingress only when it can read something

## Overview
The forum ingress is bound with no thread configured: it cannot read anything and spends 2 requests/s. It should not be
bound until it has a thread to read, and unbinding must be visible in health.

**Project**: <repo>

## Exit Criteria
- [ ] `cargo test -p pii-server --test channel_wiring`

## Guardrails
- Do NOT weaken an existing budget or control to make this pass; change the design instead.
- Do NOT commit; the loop's orchestrator commits.

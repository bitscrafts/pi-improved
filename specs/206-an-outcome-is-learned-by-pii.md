# Spec: an outcome is learned by pii, not by a shell hook

## Overview
2026-10-19: `scripts/autonomy/learn.sh` recorded each queue outcome into memory by running the CLI from a shell. It worked (two documents, recallable) and it is the wrong shape: the loop is shell, but learning is a product capability. Specs 183 ("learn every outcome to memory") and 184 (effect recorded before and confirmed after) both ask pii to do this itself.

**Project**: /projects/pi-improved

## Requirements
### Functional
- `pii memory learn --run <run-id> --status <s> --reason <text>` — records one outcome as a memory document: run id, graph, status, attempts, session, reason, newest commit when the caller supplies one.
- The same verb is usable by the autonomy loop (operator surface) and by a graph node (`memory` node, op=learn) when a run finishes.
- Duplicate suppression: the same run id + status + reason is recorded once (store-side key), so a retried row does not multiply documents.
- Failures are first-class: a parked, skipped or verify-failed outcome is recorded with the same fields as a success.

### Non-Functional
- Never blocks a run's completion: a memory failure is logged as an event (`memory.learn.failed`) and the run proceeds.
- No secret, token or environment value may enter the document.

## Exit Criteria
- [ ] `cargo test -p pii-cli --test memory_learn` passes: the verb records an outcome and a second identical call reports `unchanged`
- [ ] `pii memory learn --status skipped --reason "spec superseded"` then `pii memory search "spec superseded"` returns it
- [ ] `grep -rc 'learn.sh' scripts deploy` is zero (the shell hook is gone)

## Guardrails
- Do NOT make learning a precondition of a run's success.
- Do NOT record an outcome that was never measured (status must come from the run/queue record, not from a caller's guess).
- Do NOT store credentials or full prompts.

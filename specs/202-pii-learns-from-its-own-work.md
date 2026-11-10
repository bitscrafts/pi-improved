# Spec: pii learns from its own work

## Overview
Measured 2026-10-19: the loop's memory was `state.json` + git + `_docs`, all outside pii. `pii memory search "cancelled run timer"` returned only a round-trip note and an exercise nonce — nothing about 196, 187, the four 09:00 posts, or any spec implementation. The product has a memory subsystem (bm25 + vector, entities, tiers) and it had never once learned from its own operation. Specs 183 ("learn every outcome to memory") and 184 (effect recorded before and confirmed after) require this and nothing implemented it.

**Project**: <repo>

## Requirements
### Functional
- Every queue outcome — done, parked, skipped, verify-failed — is written to pii memory as one document naming the row, its spec, the status, the attempts, the session, the reason and the newest commit.
- A row that FAILS is recorded with the same fidelity as one that succeeds: the failure is the memory the next attempt needs.
- The write goes through `pii memory put`, so both retrieval lanes can find it later; a file on disk does not satisfy this.
- Memory unavailability never fails the work that produced the outcome.

### Non-Functional
- One document per outcome, no duplicates on retry (key on row id + status + commit).
- Recall must work by plain text: searching "row A70 run survives its process" must return that outcome.

## Exit Criteria
- [ ] `bash scripts/autonomy/learn.sh A70` then `pii memory search "row A70"` returns the recorded outcome in the bm25 or vector lane
- [ ] `grep -c 'learn.sh' scripts/autonomy/drain.sh` is non-zero (the dispatcher records every drained row)
- [ ] a parked or skipped row is also recorded: `bash scripts/autonomy/learn.sh A71` succeeds and names the skip reason

## Guardrails
- Do NOT write memory from the daemon's hot path: this is loop-side learning, not a journal replacement.
- Do NOT record a secret, a token or a spec's private content — the row, the status and the reason only.

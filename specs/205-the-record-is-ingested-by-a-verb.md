# Spec: the record is ingested by a verb, not a python script

## Overview
2026-10-19 user directive: "all must be implemented in rust!". `scripts/pii-ingest.py` fed 286 documents into memory — specs with their queue status and criteria, handoff sections, operational docs — from OUTSIDE the product, with a content-hash ledger in the repo to fake idempotency. None of it ships: the deployed artefact is the Rust binary, so on the host nothing would ever be ingested.

**Project**: <repo>

## Requirements
### Functional
- `pii index docs [--root D] [--kind spec|handoff|ops|all]` — walks `specs/*.md`, `HANDOFF.md` and `_docs/05-operations/*.md`, derives one memory document per unit, and writes it through the memory API.
- Each spec document carries: number, slug, title, queue status and row id when one exists, the verbatim Exit Criteria, the guardrails, and the overview — the same shape the script produced, so recall keeps working.
- Idempotency is a property of the STORE, not of a file beside it: a unit whose content hash is unchanged is not rewritten (a document key or a put-if-changed path).
- Counts are printed: `documents=<n> unchanged=<m> failed=<k>`.

### Non-Functional
- No python, no shell, no external process: the verb runs inside `pii`.
- Bounded memory: documents are streamed, not accumulated in one buffer.

## Exit Criteria
- [ ] `cargo run -q -p pii-cli -- index docs --kind specs` exits 0 and prints `documents=<n> unchanged=<m> failed=0`
- [ ] running it twice in a row reports `documents=0 unchanged=<n>` on the second pass
- [ ] `pii memory search "PII-SPEC 196"` still returns a spec document after the verb replaces the script
- [ ] `grep -rc 'pii-ingest.py' scripts deploy` is zero

## Guardrails
- Do NOT key idempotency on a repo file: the store must know what it already holds.
- Do NOT ingest `_tmp/` or any archived doc.
- Do NOT invent status: a spec with no queue row is `no-row`, never `pending`.

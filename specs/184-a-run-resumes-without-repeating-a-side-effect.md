# Spec: a run resumes without repeating a side effect

## Overview

Composability and recovery both rest on one property pii does not yet have: **a node's completion must be durable AND its
side effects must not repeat when the run is replayed.** Without it, "restart at any node" produces duplicate posts
(1,836 rate-limited posts were already measured on 2026-10-18), duplicate memory writes, and a deploy applied twice.

The journal (spec 154, "an accepted run is on disk before its first node") already holds the evidence a replay needs. What
is missing is (a) an idempotency key per node and (b) the verbs that make replay an operator action rather than a code path
nobody has ever run.

**Project**: /projects/pi-improved

## Requirements

1. Every node completion is journaled with: run id, node id, attempt, verdict, output digest, and an **idempotency key**
   derived from (run id, node id, input digest).
2. An EFFECT node (`post`, `memory put`, `spawn`, `hal`) records its key **before** the effect and confirms it **after**.
   A replay that finds a confirmed key SKIPS the node and reuses the recorded output.
3. A replay that finds a started-but-unconfirmed key **refuses** to re-run the effect and reports `effect.unknown` — it
   never assumes the effect did or did not happen.
4. `pii run resume <run-id>` re-dispatches only nodes with no completed journal entry, in dependency order.
5. `pii run retry <run-id> --node <node-id>` re-runs ONE node, with the previous attempt's output available as input.
6. `pii run abandon <run-id> --reason <text>` closes the run as failed with the reason recorded. Abandoned runs are as
   visible as successful ones.
7. A node that cannot be resumed (a worker mid-edit) is not restarted by the scheduler. Its session id is journaled so the
   WORKER resumes itself — the two levels must never be confused.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| replay skips completed | nodes a,b completed, c not | a,b not re-dispatched; c runs |
| effect not repeated | `post` completed, then resume | one post total; node reused from the journal |
| unconfirmed effect | key started, never confirmed | resume refuses with `effect.unknown`, does not post |
| retry one node | `retry --node c` | only c runs; a,b untouched |
| abandon is visible | `abandon --reason …` | state failed, reason stored, listed by `pii run list` |
| input digest changes | same node, different input | a NEW key; the node runs again |

## Exit Criteria

- [ ] `cargo test -p pii-graph --test resume_skips_completed`
- [ ] `cargo test -p pii-graph --test effect_not_repeated`
- [ ] `cargo test -p pii-server --test unconfirmed_effect_refuses`
- [ ] `grep -q 'effect.unknown' crates/pii-observe/src/names.rs`

## Guardrails

- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT make replay automatic on daemon start: replay is an operator action with an id.
- Do NOT delete or rewrite journal entries; a replay appends.
- Do NOT let a resumed run re-post, re-deploy or re-write memory because a flag was forgotten.

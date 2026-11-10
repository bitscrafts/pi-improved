# Review: the parked backlog

**Project**: <repo> · **Mode**: plan only — produce a verdict per row, commit nothing.
**Written by**: the queue, because these rows are stuck and a stuck row is not a finished one.

## Why this exists

Nine rows are parked or waiting and each one stopped for a different reason. Reviewing them one at a time as they stop
has cost most of a night; the same review done once, with all the evidence in front of it, costs one pass.

Every row below has its work preserved on disk. Nothing is lost — the question is only what each row needs next.

## The rows

| Row | State | What is on disk | Why it stopped |
|-----|-------|-----------------|----------------|
| 354 | parked | nothing (its WIP was another row's work) | the coder implemented spec 353 while the row said 354 |
| 357 | skipped | — | split into 370 + 371, both landed |
| 358 | skipped | — | split into 375 + 376, both landed |
| 362 | parked, needs-review | `_trash/362-models-in-one-place-wip-20261104T055759Z` (81 files + README + TREE.diff) | extraction half-done: `lib.rs` 451 and `config.rs` 492 effective lines, both over the 450 budget |
| 364 | parked (Opus running now) | `_trash/364-parked-20261104T084127Z`, `_trash/364-calls-spec-refusal-bug-*` | a refused callee dead-ends in the splice, so the row stays Running |
| 365 | needs-review | none found | never dispatched to completion |
| 366 | needs-review | none found | never dispatched to completion |
| 368 | parked | `_trash/368-parked-*` | the row about dirty trees was parked by dirty-tree handling |
| 369 | parked (3 attempts) | `_trash/369-parked-*`, `_trash/369-needs-test-update-*` | moving the phase markers left two controls reading the old source |

## What this review must produce

Write `_docs/06-research/review-parked-20261104.md` with, for **every** row above:

1. **Verdict** — one of `proceed` (the WIP is sound, finish it), `needs-decision` (a choice only the user can make, and
   state the choice in one sentence), or `close` (the row is obsolete; say what supersedes it).
2. **Evidence** — the file or command that supports the verdict, not an opinion. If the WIP is claimed sound, say
   which criterion you executed.
3. **The smallest next step** — the one command or edit that unblocks the row.
4. **Anything the row taught that the repo does not yet record** — a defect class, a guard that misfires, a doc that
   lies.

Then a short section per *class of failure* you can see across the rows, because several of them stopped for the same
reason dressed differently: a guard firing on the case it was built to permit.

## Files

- `_docs/06-research/review-parked-20261104.md`

## Exit Criteria

- [ ] `test -f _docs/06-research/review-parked-20261104.md`

## Guardrails

- **Plan mode.** Do not edit code, do not commit, do not re-dispatch any row. This review is read-only.
- Do not attempt to fix 362's extraction or 364's splice in this pass; other runs own them.
- If a WIP directory is missing or empty, say so plainly rather than assuming the row is done.
- Judge a row by executing its criteria (`uv run --no-project python scripts/autonomy/row-run-criteria.py <id>`), never
  by reading `row-verify`, which only compares the row with its spec.

# Runtime belongs in the binary; scripts are for tests

**Filed 2026-11-18, after a day that produced the wrong kind of progress. Owner's correction, verbatim:
*"estou vendo muitos scripts, tem noção que a solução esta associada a um binario apenas!"* — and then
*"claro que scripts pode ser utilizado para testes"*.**

## The drift, measured

pii is one binary plus graphs. On 2026-11-17/18 every defect closed was closed in a **shell or Python script**:

| defect closed that day | where it was fixed | where it belonged |
| --- | --- | --- |
| a row could not claim the tree it had just been given | `scripts/leftover.sh`, `resume-row-at-verify.sh` | `pii queue claim` |
| an environment block re-picked the same row forever | `scripts/autonomy/state.py`, `cycle.sh` | `pii queue pick` |
| the tree the gate validated was not the tree being committed | `quality-gate.sh`, `~/.config/git/hooks/gate-pass.sh` | `pii gate run` |
| a criterion that passed while the live path was broken | `sync-verify.sh` | `pii queue verify` |
| an unstaged deletion broke a tree-copy test | the test, correctly | the test |

Every one of those is a **boundary between two scripts that each hold half of one truth**: `session` vs
`tree=claimed`; the working tree vs the index; `pending` vs `blocked`; `plan.tsv` vs `state.json`. The queue grew a
parallel implementation of state that the binary already keeps in its own store, and the seams between the halves
are where the work was destroyed (five rows: 414, 416, 417, 418, 429).

The cost is not aesthetic: a hand-implemented row was **stashed and swept away twice**, and rows were re-dispatched
25 times against a lane that could not start, because the block had no name.

## The rule

Three kinds of file, three different homes. Do not blur them:

```
RUNTIME  — the binary. Anything that decides, records or mutates the state of work:
           pick, claim, park, dispatch, resume, gate, the proof a commit needs.
           Today in scripts: state.py, cycle.sh, leftover.sh, resume-row-at-verify.sh,
           quality-gate.sh, gate-pass.sh.  Target: `pii queue ...` and `pii gate run`,
           with the queue's policy as a graph (graphs/queue.toml).

TESTS    — scripts are the right home, and staying there is correct:
           scripts/autonomy/tests/*.test.sh, scripts/checks/*.sh. A test drives the binary
           from outside with fixtures; it must not re-implement it.

DATA     — neither: gate-steps.txt is gate input; plan.tsv + state.json are one truth
           split in two and become one store the binary already opens (redb).
```

## How progress is measured

Not by what the gate printed, and not by rows landed. **By runtime scripts deleted.** A migration row is only
done when:

1. the behaviour lives in the binary and a test of the binary proves it, and
2. the runtime script is **deleted in the same commit**, and
3. the existing test that used to exercise the script now exercises the command, with the same cases and the same
   controls (a rule that skips something still needs its near-miss).

If a row adds behaviour to a script and does not delete a script, it is drift, whatever its criterion says.

## The order that retires the most code first

```
1  435  pii queue claim    deletes leftover.sh            — the defect that destroyed hand work five times
2  436  pii queue pick     deletes state.py pick          — the block that had no name (spec 434's behaviour)
3  437  pii gate run       deletes quality-gate.sh + gate-pass.sh — steps from the graph, proof from the tree
4  438  pii queue cycle    deletes cycle.sh, as graphs/queue.toml — and plan.tsv/state.json die here
5  439  pii queue verify   deletes sync-verify.sh
```

Two things stay outside pii and are not drift: the **global git pre-commit hook** (it is git's, and it guards every
repository on the machine), and **`_tmp/` scratch** (a build cache is not a product).

## What to say when this happens again

If a fix touches more than one script, stop and ask which half of which truth is missing from the binary. If the
answer is "there is no command for this yet", the fix is the command, not the script — and the row is only finished
when the script is gone.

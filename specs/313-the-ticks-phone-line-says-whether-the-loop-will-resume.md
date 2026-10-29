# Spec 313: the tick's phone line for a stopped worker says whether the loop will resume it

## Overview
**Project**: /projects/pi-improved
**Phase**: production — unattended queue. Spec 312 made the queue do what its own outcome table promised:
a `needs-review` row whose `last` is `worker-rc=N` and which still holds a session is offered again by
`scripts/autonomy/state.py pick`, so the next cycle resumes that session (`cycle.sh:106` reads the row's session as
`RSID`, `cycle.sh:177` passes `--resume`, `cycle.sh:182` says so). The phone line did not follow.

`scripts/autonomy/tick.sh:137` still texts every rc-8 cycle the same sentence:

```
verify-failed|gate-failed|worker-rc-*|dirty-tree|bad-plan)
               say_once "bad:$ITEM:$OUT" "pii autonomy · $ITEM needs a hand ($OUT)" "log: ${LASTLOG##*/}" ;;
```

Two consequences, both measured on 2026-10-29:
- a row the loop is about to **resume by itself** is reported to the operator as a question for them, so a healthy
  unattended loop reads on the phone exactly like a stuck one — the operator is asked to look at work already in
  hand. The `say_once` key is `bad:$ITEM:$OUT`, so the sentence can never be revised for the same row either: the
  first "needs a hand" is the only one the phone ever sees for that item.
- the case that genuinely **is** a person's — a row that stopped early with **no** session, which no cycle can
  resume — gets no different wording, so the two states cannot be told apart from the message.

## Verified Premises
- **CONFIRMED (read, 2026-10-29)** `scripts/autonomy/tick.sh:130-140`: the tick already computes
  `OUT` (the last cycle's `OUTCOME=`), `ITEM` (its `ITEM=`) and `NEXT=$(python3 scripts/autonomy/state.py pick 2>/dev/null | cut -f1,4)`
  — the very row the next cycle will dispatch, first field the id. The rc-8 branch does not use `NEXT` at all.
- **CONFIRMED (read, 2026-10-29)** `say_once` (`tick.sh:21-26`) prints its lines only when its key differs from
  `_tmp/autonomy/reported`, so the key is what decides whether a corrected verdict can ever reach the phone.
- **MEASURED (2026-10-29)** after spec 312, `state.py pick` on a state row
  `{"status":"needs-review","last":"worker-rc=1","session":"S1"}` prints the row (id in field 1, session in field 9);
  with an empty `session` it prints `NONE`; and a row parked at the attempt cap never ends as `worker-rc` at all —
  `cycle.sh:212` sets `status=parked` and prints `OUTCOME=parked-attempt-cap` (exit 11). So for `OUT=worker-rc-*`,
  "will the loop resume it" is exactly "does `pick` name this item".
- **NOT CLAIMED**: that the tick can know whether the resumed run will *succeed*. The line says it is being resumed,
  which is all the queue has decided.

## Requirements

### Functional
- **R1.** The `worker-rc-*` phone line is decided from the live queue, not from the outcome string: when the tick's
  own `NEXT` names `ITEM` (first field equals `$ITEM`), the line reads
  `pii autonomy · <item> resuming with its session (<OUTCOME>)` and the follow-up line is `log: <last log>` as today.
- **R2.** In every other `worker-rc-*` case (no session, or the item is not offered) the line reads
  `pii autonomy · <item> stopped early with no session to resume — needs a hand (<OUTCOME>)`, so the operator can tell
  the two states apart.
- **R3.** The two verdicts carry different `say_once` keys (`resumed:$ITEM:$OUT` and `bad:$ITEM:$OUT`), so a row that
  first cannot be resumed and later can (a session recorded by a later cycle) still delivers the newer sentence once.
- **R4.** `verify-failed`, `gate-failed`, `dirty-tree` and `bad-plan` keep today's sentence and today's key,
  byte-identical: a failed verify is never reported as a resume.

### Non-Functional
- No new process, no new file in the tree, no extra call per tick beyond reading the `NEXT` the tick already has.
- `tick.sh` stays runnable under `bash -n` and under `set -u` with `NEXT` unset (a tick whose `pick` failed must not
  abort before reporting).

## TDD Contract
The test takes the case block **out of `scripts/autonomy/tick.sh` at run time** (the same technique as
`scripts/autonomy/tests/cycle-disk-wiring.test.sh` and the worker-rc block of `pick-resumes-worker-rc.test.sh`), so a
change to the live text is followed by the check instead of a re-typed copy of it.

| Test (`scripts/autonomy/tests/tick-message-worker-rc.test.sh`) | Input | Expected |
|---|---|---|
| `a-resumed-row-says-resumed` | `OUT=worker-rc-1 ITEM=A1 NEXT=$'A1\ttitle'` | a line contains `resuming with its session` and `A1`; key starts `resumed:` |
| `a-row-with-no-session-asks-for-a-hand` | `OUT=worker-rc-1 ITEM=A1 NEXT=$'A2\ttitle'` | a line contains `no session to resume` and `A1`; key starts `bad:` |
| `an-empty-queue-asks-for-a-hand` | `OUT=worker-rc=9 ITEM=A1 NEXT=NONE` | same as above (NONE is not the item) |
| `CONTROL-a-verify-failure-is-not-a-resume` | `OUT=verify-failed ITEM=A1 NEXT=$'A1\ttitle'` | the line is exactly today's `needs a hand (verify-failed)`; key `bad:A1:verify-failed` |
| `CONTROL-a-gate-failure-keeps-its-line` | `OUT=gate-failed ITEM=A1 NEXT=$'A1\ttitle'` | `needs a hand (gate-failed)`, unchanged |
| `CONTROL-the-two-keys-differ` | the first and second cases in order | the recorded keys differ, so the phone is not silenced by the earlier verdict |
| `the-live-block-still-has-the-case` | — | the extracted block contains `esac` and every outcome name, so an empty extraction fails loudly instead of passing |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/tick-message-worker-rc.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `! bash scripts/autonomy/tests/tick-message-worker-rc.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `grep -q 'resuming with its session' scripts/autonomy/tick.sh`
- [ ] `grep -q 'no session to resume' scripts/autonomy/tick.sh`
- [ ] `bash -n scripts/autonomy/tick.sh`
- [ ] `git diff --quiet -- specs scripts/autonomy/cycle.sh scripts/autonomy/state.py scripts/autonomy/plan.tsv`

## Files
- `scripts/autonomy/tick.sh`
- `scripts/autonomy/tests/tick-message-worker-rc.test.sh`

## Guardrails
- Do NOT change any `exit` code, any `OUTCOME=` string, or `cycle.sh` / `state.py`: callers read all of them. This
  spec changes one case arm's text and key, nothing else.
- Do NOT text a second line for one outcome (the phone shows the tick's lines as one message; a verdict plus a
  correction reads as two events).
- Do NOT re-derive "will it resume" from `attempts` or from the row's status with a new rule of your own: the queue's
  own answer is `state.py pick`, and a second opinion in the tick is a second thing to keep true.
- Do NOT edit `specs/`, `scripts/autonomy/plan.tsv`, or any other test's assertions.
- **If the case block at `tick.sh:134-140` no longer matches this shape** (for example `NEXT` is no longer computed
  before the case, or `say_once` takes a different argument list), STOP and say so in `## Closure` instead of inventing
  a second code path.

## Follow-ups (not this spec)
1. The same distinction is owed to the `done` line when the row was resumed: a resumed run that ends `done` texts
   `pii autonomy · <item> done`, which is right, but the tick never says the resumed session is what finished it.
2. `report loop.park` (`cycle.sh:213`) records the attempt-cap park for the dashboards; the phone line for
   `parked-attempt-cap` (exit 11) is not in this case block at all and reaches the operator only on the next tick's
   `pick` note.

## Closure
(pending — the orchestrator records the six criteria on the committed tree, the mutation that reddens the new test
with its byte-identical restore, and `bash scripts/autonomy/selftest.sh`'s verdict.)

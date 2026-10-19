# The path to production: what I think is still missing

2026-10-19, ~10:30Z (07:30 local). Written for review by Claude + Opus: this is MY view of the gaps, not a verdict, and
every line names the measurement behind it. 070's seven steps are the yardstick; where I disagree with 070 I say so.

## Where we are, measured

- deployed revision `121fb6de67e1`, unit active, `restarts 0`
- `cron_entries 1`, suspended (armed) runs `1` → the pair holds
- store: `runs 1866` — 1836 failed (yesterday's 09:00 storm), 29 succeeded, 1 suspended
- `posts 5`: one start-up post, plus **four from runs I had CANCELLED**

## The five gaps I believe are real

### 1 CANCELLATION DOES NOT DISARM — found, fixed, not proven in production

`pii run cancel` returned `{"cancelled":true}` for three runs, and all three still posted at 04:20Z (r…4129, 4161, 4193,
4225), joined by the one I kept. Spec 196 now requires that a cancelled run's timer does not fire it (`5a6445e`), with a
follow-up (`46fa583`).

GAP (corrected 12:35Z): the exercise this section asked for **exists** — `scripts/exercise-196-past-the-instant.sh`
runs a scratch daemon with a timer three minutes ahead, cancels the run, drops the `[[cron]]` entry and watches the
clock: the pre-fix deployed binary executed and posted 2 runs, this tree executed and posted 0. What is still
missing is a repeat against the **deployed post-196 binary**, i.e. production terms, not code.

### 2 THE 12:00Z OCCURRENCE — MEASURED, AND IT PASSED

070 step 2 is the reading that earns the verdict: at 12:01Z exactly **+1 run and +1 post**, unchanged at 12:05Z.

MEASURED 2026-10-19 12:00Z on revision `121fb6de67e1` (passive observation: `pii health`, `pii run ls`; the harness
injects nothing). Artefact: `_tmp/occ-watch/DIGEST-OCCURRENCE-2026-10-19-1200Z.md`, four snapshots (11:59:35,
12:01:35, 12:06:06, 12:16:06) plus the store counters.

| check | result |
|---|---|
| exactly one digest run fired (one transition out of suspended: `r000000004353`) | PASS |
| no second run by +6 min, none by +16 min | PASS |
| armed count unchanged at 1 (the next occurrence, `r000000004356`) | PASS |
| exactly one new post (`posted` 0 → 1) | PASS |
| no refused post (`post_refused` +0) | PASS |

VERDICT: 070 §1 step 2 **PASS** — one digest run, one post, no refusal, the armed run back to 1. This is the
multiplication defect NOT reproducing at a real occurrence: one firing, one post.

Two honest limits. The harness's own verdict step died on `ARG_MAX` the first time (a 270 KB run list passed as
one argv) and was corrected to hand it over as a file and to count a firing as a **status transition out of
suspended** rather than a new row — the armed run is updated in place, so the original "new terminal row == 1"
could never have passed. The counters above are the corrected run on the same recorded snapshots, not a re-run
of the occurrence. And step 2 is ONE occurrence; the verdict 070 names still needs step 6 (seven days of exactly
one digest per day with no unplanned starts).

### 3 THE STREAM DEFECT — fixed at the socket, unproven end-to-end, contradiction unresolved

`conn.rs` maps close_notify to end-of-stream; the diagnostic compiled into the binary never printed, which I read as proof
that the arm fired; but the **same error text kept appearing**. I could not reconcile those two facts before the night's
queue took over. Either the diagnostic's absence means something other than "the arm fired", or the error is re-raised above
it — and `http.rs`'s head loop, `chat_streaming`, and the SSE `feed` are all candidates.

GAP: the ollama-cloud switch is deployed with a truncation defect whose site is **not proven**. This is the least certain
thing on this page and deserves independent attention more than anything else here.

### 4 THE EXERCISES ARE PARTLY CLAIMED, NOT ALL EVIDENCED

E5/E6 were run with the deployed binary as a control (`9045484`). E1/E2/E3/E8 and the 187 restart proof need reading in
`EXERCISE-RESULTS-2026-10-19.md` against the matrix. **Where an exercise has no result line, it is NOT done** — the cron's
report is a self-report, and this project has already closed rows on self-reports twice (A42's green gate over uncommitted
work; 166's criterion that could not fail).

GAP: the matrix's PASS column must be earned line by line, not by a summary.

### 5 THREE OF THE SEVEN STEPS HAVE NO OWNER

070's steps 3 (production stops being the test bench — 85 starts vs 6 installs), 4 (a full gate on the deployed revision)
and 5 (a card for every install) are **process constraints, not code**. Nothing enforces them: `install-pii.sh` does not
require a card, and nothing counts starts against installs. I believe step 5 should be enforced IN CODE — the installer
refuses without `_tmp/deploy/card-<rev>.json` for the revision it builds — because it is the one gap a patch can close
permanently.

GAP: two of the seven steps depend on my discipline, and my discipline has failed repeatedly this week.

## Where I disagree with 070

070 says "what earns the verdict is steps 2 and 6". I would add a third: **step 1's proof must be a PRODUCTION restart
test, not a scratch one.** 185/187 changed the daemon's arming logic, and the only way to know it holds on lnx is to restart
the live unit once and watch the armed count stay at 1. A scratch daemon proves the code; only the live one proves the
deployment. 070's own step 3 argues against gratuitous production starts — I agree, which is why this should be ONE start,
counted, not a habit.

## What I would do next, in order

1. read the 12:01Z and 12:05Z counters and record them verbatim — that reading IS the verdict, not a step towards it
2. prove cancellation in production on a scratch daemon with a two-minute cron
3. resolve the stream contradiction by instrumenting the HTTP head loop and `chat_streaming`, not by another patch
4. make the installer refuse without a card (step 5) — the one process gap code can close for good
5. re-read `EXERCISE-RESULTS-2026-10-19.md` and mark every unrun exercise NOT DONE in the matrix

## The question I want Opus to answer

Given this page, the tree at `HEAD`, and 064-070: **is there a gap in this list that I have not seen** — and would you
change the ORDER above? Specifically: is measuring the stream defect worth more than waiting for the 09:00 occurrence, or
should the occurrence be read first regardless of what else is broken?

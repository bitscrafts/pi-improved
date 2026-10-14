# Open work on the experiment harness — the one thing blocking a real verdict on spec 151

**Written**: 2026-10-14, after the host stabilisation (load 40 → 8; an 11-day orphan ffmpeg killed).
**Why this file**: spec 151 (an incident → a proven fix on a branch) was implemented and gated green, but the
experiment meant to AUTHORISE it has never produced a valid verdict. The harness, not the thesis, is what failed —
six times. This is the diagnosis and the exact remaining fix, so the next session does not re-derive it.

## Where the harness lives

- `scripts/experiments/prove-fix-in-7-cycles.sh` — lists/seeds the 7 candidates (dry run free).
- `scripts/experiments/run-experiment-batch.sh` — loops them, stops when the account leaves `severity: normal`,
  prints the pre-registered decision.
- `scripts/experiments/measure-proven-fix.sh <idx> <fingerprint> <fix-commit> <crate> <paths>` — one candidate:
  seeds a tree at `fix-commit^`, one worker (sonnet, `--max-turns 60`, no budget cap), then measures.
- Results: `_tmp/autonomy/experiment/results.tsv` (append-only). Trees: `/projects/_tmp/exp-prove-fix/`.

## The six failures, all in the instrument (kept so they are not repeated)

1. relative paths evaluated inside a subshell that had `cd`-ed to the scratch tree → `claude -p ""`, rc=1, $0 spent.
2. `find -newermt '-90 minutes'` — an invalid predicate (negative times), so "no change" was guaranteed. Replaced by
   a **checksum manifest** taken at seeding.
3. a "honest note" patch spliced into the middle of a statement → syntax error at line 77; the manifest it compared
   was never written. Fixed by rewriting the file rather than patching it a third time.
4. the batch's `while read` loop is fed by a heredoc pipeline → **the worker inherited that stdin and ate the rest of
   the candidate table** (7 became 1). Fixed: the worker gets `</dev/null`.
5. the proof extractor took every `fn` in the changed files, including pre-existing tests → it tested a name that
   passes on base AND fix ("unproven" for a measurement reason). Fixed: the added names are a **set difference**
   against the base file.
6. **STILL OPEN — the base run does not carry the worker's TEST.** The harness runs the introduced test by name in the
   base copy, where that test does not exist, so cargo runs 0 tests and exits 0: "base rc=0" is guaranteed for every
   candidate, and a test that can never be red can never be proven. This is why all 9 rows in `results.tsv` are void.

## The remaining fix (exact)

In `measure-proven-fix.sh`, before the base run, **splice the worker's ADDED test functions into the base copy**:
the added names are already computed per file (the set difference), so insert, for each of them, the body of that
function from the worker's tree into the corresponding file of the base copy — then run it there. Expect RED.
Then run it in the worker's tree: expect GREEN. Both runs must use the same test name and the same shared
`CARGO_TARGET_DIR`, and the row must record both rcs plus the name.

**Control (mandatory)**: a candidate whose fix was applied to the base copy as well must go GREEN on base — if it
stays RED, the splicing is broken, not the fix. Without that control, "red on base" is as unearned as the current
"base rc=0".

## What happens on a valid verdict

- **≥3 of 7 proven** → 151 stands; 152 (the deploy card, running) carries its two runs.
- **fewer** → 151 is reduced to the manual path and 152 carries the person's evidence. Either way, record the decision
  line from `run-experiment-batch.sh` and say which candidates failed and why, not only the tally.

## Cost so far

$2.60 for the batch (7 workers, 11–19 turns each) + $0.29 before it. No verdict is accepted from any of it.

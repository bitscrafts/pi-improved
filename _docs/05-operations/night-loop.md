# The night loop — fill the queue, drain it, review, refill

Written 2026-11-02, during the first night it ran. This is the operating manual for "work the queue all night,
put an Opus analysis at the end of each drain, and repeat until the deadline". Every mechanism named here exists
in the repo and is exercised by a script that can be re-run.

## What it is, in one paragraph

A person says *"work until 07:00"*. A **queue** holds the gaps (one `plan.tsv` row per gap, one spec per row). A
**dispatcher** already running every five minutes drains it: it dispatches a worker on the head row, runs the
row's criteria, and either commits or parks the row. A **supervisor** (also every five minutes, and with no model
in it) applies four mechanical defaults so the queue cannot stall. When — and only when — every row of the current
tranche is closed, a **review** step writes a status brief, runs an Opus analysis, files the report, and converts
its next tranche into rows, which the dispatcher then drains. At the deadline the supervisor stops the loop and
writes a summary.

## The rule that is not negotiable

> *"it only will go to the next opus review after all gaps were closed"* — the operator, 2026-11-02.

`night.sh` enforces it: `open_rows()` lists every row that is `pending`, `running` or `needs-review`, and the review
refuses to start while one exists, naming it in the journal. A **parked** row is retried by the supervisor
(`state.py repair`) rather than skipped, because skipping would fake closure and let a review run on a queue that
still holds an unfinished gap. If a row parks three times, the night simply has no review — and the morning report
says which gap held it.

## The pieces

| Piece | What it owns | Command |
|---|---|---|
| the queue | what work exists, one row per gap | `scripts/autonomy/plan.tsv` + `_tmp/autonomy/state.json` |
| the dispatcher | the drain: dispatch → criteria → gate → commit or park | job `bccaf71bf569`, `scripts/autonomy/drain.sh`, every 5 min |
| the tick | fill, review, refill, deadline | `scripts/night/night.sh --tick` |
| the supervisor | the four defaults, so nothing stalls | `scripts/night/watch.sh`, job `1b2766167b48`, every 5 min |
| the row/spec repair | makes a spec's `## Files` and `## Exit Criteria` equal its row | `scripts/night/fix-row.sh <id>` |
| the tree mover | a stuck tree goes aside, never away | `scripts/night/clear-stuck-tree.sh <dest> --apply` |
| the report | the morning message, straight to Discord | `scripts/night/morning-report.sh`, job `7929d4069522` at 10:00Z |
| the tranches | tonight's gaps, split around the review | `scripts/night/tranche-a.tsv`, `tranche-b.tsv` |

## Running a night (the checklist)

```bash
# 0. the queue must be dispatchable FIRST — a row whose spec disagrees with it is refused before anything is spent
python3 scripts/autonomy/state.py pick          # must print an id, not "refused <id>: NO-FILES"
bash scripts/night/fix-row.sh --check <id>      # per row; repair with: fix-row.sh <id>

# 1. write the specs, then make each spec declare exactly its row's files and criteria
bash scripts/night/fix-row.sh <id>              # rewrites both machine-consumed blocks FROM the row

# 2. seed the night
bash scripts/night/night.sh --seed a            # appends tranche-a.tsv rows to plan.tsv (idempotent)

# 3. let the dispatcher work (nothing to do; it fires every 5 min)
bash scripts/night/watch.sh                     # one supervisor step, by hand, if you want it now

# 4. watch
bash scripts/night/night.sh --status            # state, deadline, queue, worker
tail -f _tmp/night/night.log                    # one line per decision
python3 scripts/autonomy/state.py list | awk -F'\t' '$1>=335'   # rows this night

# 5. prove the review step on a clean tree, before trusting it at 07:00
bash scripts/night/night.sh --review-now        # refuses if a worker is editing or the tree is dirty

# 6. stop early
#    bash scripts/night/night.sh --until <past-ISO>      (the next tick closes the night)
```

## The four default answers (what the supervisor may decide alone)

1. **A row disagrees with its spec** (`NO-FILES`, `NOT-A-CRITERION`, `COUNT`, `DIFF`) → `fix-row.sh <id>` rewrites
   the spec's two machine-consumed blocks *from the row*. The row is the contract; a model is never asked to
   reword a criterion, because a model that "fixes" criteria can weaken them.
2. **A tree that stays dirty across two firings** → `clear-stuck-tree.sh` moves every path to
   `/projects/_trash/night-stuck-<stamp>/` with a `MANIFEST.txt`. It refuses when a dispatchable row *declares* a
   dirty path (that is that row's work, not garbage). Nothing is deleted, ever.
3. **A parked row** → `state.py repair` and back to the dispatcher. Never `skipped`.
4. **An empty queue** → `night.sh --tick`, detached, which seeds the next tranche or runs the review.

## State, logs, and where the truth is

```
_tmp/night/state.json     tranche, opus_runs, done            (the loop's memory; one step per firing)
_tmp/night/night.log      every decision, timestamped
_tmp/night/SUMMARY.md     written at the deadline
_tmp/night/opus-<ts>/     each review's brief, json and plan file
_docs/06-research/        the filed reviews, with provenance headers
_tmp/autonomy/logs/       one cycle log per dispatch (SPEC, RUN, OUTCOME)
_trash/night-stuck-*/     a stuck tree, moved, with its manifest
```

## Failure modes, all of them measured tonight

| what happened | why | the fix now in the tree |
|---|---|---|
| an **agent** cron job died at its first firing: `RuntimeError: Gemini HTTP 503` | a cron job does **not** inherit the session's model; it took a default | both night jobs are **scripts**, `no_agent: true`. No model to be unavailable |
| `night.sh` ran Opus **synchronously** inside a cron | the scheduler kills a long script; the review would die mid-run after being paid for | the review is `setsid nohup … &` from the supervisor, and `night.sh`'s own lock prevents a second |
| the mover moved a **tracked** file | `mv` on a tracked, modified file leaves it deleted in the index | restore with `git show HEAD:<path> > <path>` (never `git checkout --`), then `chmod` to the mode in `git ls-files -s` |
| the mover wrote `_trash/` **inside the repo** | `_trash` was taken as relative to the repo root | destinations must be under `/projects/_trash`; a relative path is what created an untracked dir in the tree |
| the mover **refused every tree** | it refused when the head row was merely *valid*, and every row is valid | it now compares the dirty paths against the row's **declared** paths |
| a parked row's abandoned `gates.sh` change turned the **gate red for every row behind it** | a worker's partial work stays in the tree | the mover took it aside (4 paths, manifest), the tree went 6 → 0, the gate went back to 15/15 |
| rows were refused with `NO-FILES` / `NOT-A-CRITERION` | the spec did not declare the files and criteria its row states | `fix-row.sh`, and the six specs now declare both |

## What is proven, and what is not (2026-11-02, 06:10Z)

Proven with output: the dispatcher takes rows (335 → run `r000000001026`; 336 → `r000000001034`), parks a row after
three attempts, resumes on the next firing, and produces real diffs; the supervisor refuses to act while a worker
is in flight; the mover moved a real abandoned tree and refused a row's own work; `fix-row.sh` refused a broken
criterion and repaired it; `night.sh`'s unit behaviour is 15/15; the report job **delivered to Discord** (read back
through the API: `Cronjob Response: pii morning report …` at 05:57).

**Proven at 07:12 and 07:57 the same night:** the review step end to end — brief written by the script, Opus read the
tree, the plan file landed in `_tmp/night/opus-<stamp>/`, the filer put it in `_docs/06-research/`, and the tranche was
seeded after it. Two reviews, `$3.35` each. Getting there cost four silent bugs, now fixed and written down above:
a workdir stamp with colons made the filer's `cd` fail while it exited 0; the tranche-b existence check read the
wrong directory; the mover accepted a relative `_trash/` (creating dirt inside the repo); and `pgrep -f` matched any
shell whose *text* mentioned a worker script, so the loop sat still reporting "a worker is in flight".

## Cost

Measured 2026-11-02: one Opus review = **$3.35** (54 turns, plan mode) plus a $0.11 probe. A worker cycle on
`claude-haiku-5-5` ≈ **$0.002**. A night with two reviews and six rows is therefore ≈ **$7**, dominated by Opus.

## Three traps that only appear once the night is running

- **A one-shot cron job is consumed by a manual test.** Firing a `repeat: once` job with `cronjob run` to prove its
  delivery marks it `completed`, `1/1`, `next_run_at: null` — and the 07:00 report never happens. Test delivery with
  a **separate** job (or accept that the test IS the run), then arm the real one and read `next_run_at` back.
- **`pgrep -f` matches prose.** A pattern like `run-batchC-pii|cycle.sh` matches any shell whose *command line*
  mentions those words — including the grep that looks for them. Anchor it: `^bash (.*/)?(scripts/autonomy/cycle|
  scripts/run-batchC-pii)\.sh`.
- **A guard can eat the work it protects.** The supervisor's "dirty tree across two firings → move it aside" fired
  while a worker's finished-but-uncommitted row was in the tree, and moved the row's four files to `_trash`. The
  evidence survived, and it had to be copied back. If a person is at the keyboard, land the row before the next
  supervisor firing; if nobody is, prefer a longer streak over a faster unblock.

## The night of 2026-11-02, in numbers

Six rows seeded, six closed (`335` the host probe, `336` the mqtt allowlist, `337` the durable stores, `338` the
installer, `339` plain-shell health, `340` the incident publisher). Three Opus analyses: the audit at 05:34, a
review at 07:26 and a review at 08:01. Twenty-one commits. Every row needed a person to land it: the workers parked
after three attempts each, and the dispatcher never closed one on its own — that is the honest headline of the
night, and the reason the review's first recommendation is still about the loop, not the code.

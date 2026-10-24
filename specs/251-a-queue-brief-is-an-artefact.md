# Spec 251: a queue brief is an artefact of the run, not a change to the tree

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the autonomy loop's hygiene (measured 2026-10-24)

When the queue empties, `cycle.sh` writes a queue brief so the session ends with a file instead of a paragraph in a
terminal (`scripts/autonomy/cycle.sh:90-98`). It writes it into `_docs/05-operations/QUEUE-BRIEF-<hour>.md` — a **tracked**
path — once per hour, and nothing ever commits it. Measured 2026-10-24T06:20Z: **28 untracked `QUEUE-BRIEF-*.md` files,
188 KB**, sitting in the tree since 2026-10-19.

The consequence is not cosmetic. `cycle.sh:143-165` reads `git status --porcelain` before every dispatch: with the briefs
present the tree is always "dirty", so every dispatch that has no session to resume takes the leftover branch, writes a
patch, and posts an `ask` to the operator's phone about a leftover that is not a leftover. `deploy-row.sh:25` already
carves the same files out by hand (`grep -v '^?? _docs/05-operations/QUEUE-BRIEF'`) — a second place that had to learn the
workaround. Two writers emit the brief (`cycle.sh` and `drain.sh:73`), so a fix in one place leaves the other one dirtying
the tree.

The artefacts of a run belong under `_tmp/`, which the loop already treats as scratch (`deploy-row.sh:25` excludes
`^?? _tmp/`). This spec moves the writer there, through ONE helper both writers call, and moves the 28 stranded files
aside — never deletes them.

## Requirements
### Functional
- **R1.** A new helper `scripts/autonomy/brief-path.sh` prints, on stdout, the repo-relative path of this hour's brief:
  `_tmp/autonomy/briefs/QUEUE-BRIEF-<YYYY-MM-DDTHH>.md` (UTC, one file per hour, the same cadence the cycle uses today).
  It creates `_tmp/autonomy/briefs/` if it does not exist and exits 0. It has no side effect on the tracked tree.
- **R2.** `scripts/autonomy/cycle.sh` obtains its brief path from that helper (`BRIEF=$(bash scripts/autonomy/brief-path.sh)`)
  and keeps the `mkdir -p` it needs for the log directory only; the `mkdir -p _docs/05-operations` it does for the brief
  goes away with the path.
- **R3.** `scripts/autonomy/drain.sh` obtains its brief path from the same helper, so the two writers cannot drift apart.
- **R4.** The 28 `_docs/05-operations/QUEUE-BRIEF-*.md` files that are untracked in the tree are **moved** (plain `mv`,
  never deleted) into `_tmp/autonomy/briefs/archive-2026-10-24/`, preserving their names, so `git status --porcelain`
  names none of them and the history of the session summaries survives on disk.
- **R5.** The line the cycle prints still names the brief's location (`say "queue brief: $BRIEF"`), and the OUTCOME line
  is unchanged: a person can still find the file from the cycle log.

### Non-Functional
- Offline, deterministic, no new dependency; the helper is pure shell and returns the same path for every call within one
  UTC hour.
- Nothing under `specs/`, `_repos/` or `modules/` is touched.

## Architecture
```
cycle.sh  ──┐
            ├─> bash scripts/autonomy/brief-path.sh  ->  _tmp/autonomy/briefs/QUEUE-BRIEF-<hour>.md
drain.sh  ──┘                                             (scratch: never counted by git status)
queue-brief.sh  (unchanged: it PRINTS; the caller decides where)
```

The brief's content generator (`queue-brief.sh`) is not changed: it prints to stdout and knows nothing about paths. The
path decision lives in exactly one place.

## TDD Contract
No new Rust. The proof is one helper, two call sites, one behavioural reading of the real writer and one control:

| Check | Expected |
|---|---|
| the helper prints a scratch path, once, in the briefs dir | `grep -c '^_tmp/autonomy/briefs/QUEUE-BRIEF-'` = 1 |
| the helper really creates the directory | `test -d _tmp/autonomy/briefs` after calling it |
| both writers call it (no second path expression left) | `grep -c 'brief-path.sh'` = 1 in each of the two scripts |
| writing the brief the way the cycle now does leaves `git status` unchanged | count before = count after, brief non-empty |
| CONTROL: the OLD path is not ignored, so it would still dirty the tree | `git check-ignore` fails on `_docs/05-operations/QUEUE-BRIEF-x.md` |
| the stranded files are gone from `git status`, and archived on disk | 0 porcelain hits, at least 28 files in the archive dir |

## Exit Criteria
- [ ] `test "$(bash scripts/autonomy/brief-path.sh | grep -c '^_tmp/autonomy/briefs/QUEUE-BRIEF-')" = 1`
- [ ] `bash scripts/autonomy/brief-path.sh > /dev/null && test -d _tmp/autonomy/briefs`
- [ ] `test "$(grep -c 'brief-path.sh' scripts/autonomy/cycle.sh)" = 1`
- [ ] `test "$(grep -c 'brief-path.sh' scripts/autonomy/drain.sh)" = 1`
- [ ] `b=$(git status --porcelain | wc -l) && p=$(bash scripts/autonomy/brief-path.sh) && bash scripts/autonomy/queue-brief.sh > "$p" && a=$(git status --porcelain | wc -l) && test "$a" = "$b" && test -s "$p"`
- [ ] `! git check-ignore -q _docs/05-operations/QUEUE-BRIEF-x.md`
- [ ] `test "$(git status --porcelain | grep -c 'QUEUE-BRIEF')" = 0`
- [ ] `test "$(ls _tmp/autonomy/briefs/archive-2026-10-24 | wc -l)" -ge 28`

## Guardrails
- Do NOT delete the 28 stranded briefs, and do NOT `git add` them: **move** them with `mv` into the archive directory.
- Do NOT change `queue-brief.sh`, the OUTCOME line, the exit codes, or any other step of `cycle.sh`/`drain.sh`.
- Do NOT add `_docs/05-operations/QUEUE-BRIEF-*.md` to `.gitignore` as a shortcut: the point is that a run's artefact does
  not enter the tracked tree at all. If the helper cannot be threaded through both writers without touching another step's
  behaviour, **STOP and report** instead of widening the diff.
- Do NOT weaken a check to make it pass, and do NOT write anything under `specs/`. No commit. Scratch under `/projects/_tmp/`.

## Closure

Closed 2026-10-24T12:2xZ by the orchestrator, independently of the worker's report: the eight exit
criteria were run on the committed tree `09c06eb`, then criterion 1 was reddened by a mutation of
`scripts/autonomy/brief-path.sh` and restored byte-identical.

- **Criteria, on the committed tree** (`_tmp/autonomy/verify-251.sh`): C1..C8 all rc=0.
  `git status --porcelain` empty before and after the run — criterion 5 writes the brief through
  `queue-brief.sh` into the pinned path and the tree count is unchanged (that is the defect this spec
  named, measured live rather than grepped).
- **The brief still lands where the row pins it, and the cycle log names it**:
  `[11:58:54Z] queue brief: _tmp/autonomy/briefs/QUEUE-BRIEF-2026-10-24T11.md` — a run's artefact, not
  a change to the tree.
- **Archive count**: `ls _tmp/autonomy/briefs/archive-2026-10-24 | wc -l` = 28 (criterion 8 asks >= 28).
- **Mutation** (`_tmp/autonomy/mutate-251.sh`): `D=_tmp/autonomy/briefs` -> `D=_tmp/autonomy/briefs-mutated`
  (one directory off the pinned home). C1 went 0 -> 1 while the near-miss controls C2/C5/C7 stayed 0 —
  the criterion reddens on the path the defect is about, not on any change. Restored with `mv` +
  `touch`, `sha256` before `b25990a1…0b0789` equals after; criteria re-run green; `git status` clean.
- **The gate nobody asked for yet**: `bash scripts/autonomy/selftest.sh` at this tree —
  `RESULT: 40 passed, 0 failed` / `VERDICT: ALL MECHANISMS PASS`
  (`_tmp/autonomy/logs/selftest-20261024T121617Z.log`).

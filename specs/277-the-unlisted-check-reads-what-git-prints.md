# Spec 277: the unlisted-change check reads every shape git status prints, so a rename cannot slip a commit and a quoted path cannot park a row

## Overview
**Project**: <repo>
**Phase**: production — the queue's own integrity. `scripts/autonomy/unlisted-changes.sh` (spec 268) exists so that `cycle.sh`'s commit can never stage a subset of the worker's changes: the row's path list is the whole of what lands, and anything else refuses the commit and parks the row as `needs-review`. That guarantee is read from `git status --porcelain` by slicing fixed character offsets and by matching the row's paths as text — and git does not print paths as text. Two shapes are misread, in **both** dangerous directions, measured today:

- **A rename is silent (false pass, the direction spec 268 exists to prevent).** `git status` prints a rename as `R  old.rs -> new.rs`; the script's `case "$ST" in "??"|?[MD])` matches only `M` and `D`, so an `R` entry is skipped entirely. Measured in a scratch repo (`_tmp/diag-unlisted2-522752`): with the only change a rename of `old.rs` to `new.rs` and the row listing only `keep.rs`, the check printed nothing and exited **0**. A worker that renames a module and edits its `mod` line in a file the row does not list would have its rename left unstaged and the commit would land a HEAD that does not build — the exact class of A131/A132 (spec 268) and of A136/A141 (rows whose path lists missed `nodes/coder.rs` and `graph_node.rs`).
- **A quoted path is refused when it is listed (false refusal).** git quotes any path with a space, a quote byte, a tab or a non-ASCII byte; the script takes `P=${LINE:3}`, which yields `"with space.rs"` **including the quotes**, so the row's own entry `with space.rs` can never match it. Measured in the same scratch: the file is modified and listed, and the check printed `"with space.rs"` and exited **1**. A correct worker on such a path is parked as `needs-review` and its work is re-billed by a later resume.

Neither shape is a policy question: the check claims to compare changed paths with the row's list, and both readings are simply not the paths. The fix is one parse: ask git for the machine-readable form (`-z`) instead of parsing its human-readable one.

## Verified Premises
- **CONFIRMED (measured 2026-10-25, not read)** the two failures above, in throwaway repos under `_tmp/` created for the measurement (`_tmp/diag-unlisted-522573`, `_tmp/diag-unlisted2-522752`, both kept): `R  old.rs -> new.rs` produces **rc=0 with no output** when the row does not list `new.rs`; ` M "with space.rs"` produces **rc=1 printing `"with space.rs"`** when the row lists `with space.rs`.
- **CONFIRMED (read)** `unlisted-changes.sh` is the whole of the check: it reads `git status --porcelain --untracked-files=all` on stdin of a `while` loop, slices `ST=${LINE:0:2}` and `P=${LINE:3}`, skips everything but `??` and `?[MD]`, exempts `HANDOFF.md`, `scripts/autonomy/__pycache__/*` and any `QUEUE-BRIEF*` basename, and treats a listed directory as covering its subtree (`"$E"|"$E"/*`). It never stages, commits or resets. `cycle.sh:384` calls it with the row's `;`-joined path list and, on a non-zero rc, runs `git reset -q`, sets the row `needs-review` and exits 17.
- **MEASURE FIRST (do not assume)**: (1) `git status --porcelain=v1 -z -uall` field order for a rename — read the actual bytes with `git status --porcelain=v1 -z | od -c`, do not take the order from this spec; the new path is the one the row must list, and the old path follows the NUL. (2) Whether a copied (`C`) entry appears at all in this repo's git version, and what `-z` prints for a path that ends in a newline. (3) Whether any row in `scripts/autonomy/plan.tsv` lists a **directory** whose contents are renamed by a normal item, so the control (f) below is written against a real row shape and not an invented one. If any of the three contradicts R1/R2 as written, STOP and report it rather than forcing the parse.
- **NOT CLAIMED**: that this makes the row's path list complete. It only makes the check compare the paths git actually reports. A row that omits a path is still caught only because the worker's edit to that path is visible as its own entry; deriving the list from the spec is a separate item.

## Requirements
### Functional
- **R1. Read the machine form.** `unlisted-changes.sh` reads `git status --porcelain=v1 -z --untracked-files=all` (NUL-separated, unquoted) instead of the line form. A rename (`R`) or copy (`C`) entry is checked as the path the change **produces** (the new path); the old path is not a change the row must list. Added/untracked (`??`, `A`), modified (`M`), deleted (`D`), typechange (`T`) and unmerged (`U`) entries are each checked as their own path — the `?[MD]` narrowness is the defect, so the skip list must name what is deliberately ignored (none today) rather than what is handled.
- **R2. No quoting is possible.** With `-z` the path is the bytes git holds: a space, a tab, a newline, a quote or a non-ASCII byte compares verbatim against the row's entry. Nothing in the script may trim, quote, unquote or word-split a path (`read -r -d ''`, no `for x in $P`, no `ls`).
- **R3. The exemptions keep their exact current behaviour**, matched on the unquoted path: `HANDOFF.md`, `scripts/autonomy/__pycache__/*`, and any basename starting `QUEUE-BRIEF`. A listed directory covers its subtree exactly as it does today. Exit 1 when at least one path was printed, 0 when none was; the printed line is the bare path, one per line.
- **R4. Tests that could fail, with controls.** New `scripts/autonomy/tests/unlisted-changes-test.sh`: for each row it creates its OWN scratch repo under `_tmp/` (never `/tmp`, and it creates it), runs the real `scripts/autonomy/unlisted-changes.sh` against it, and prints `PASS <row>` or `FAIL <row>: <why>`; the last line is `RESULT: PASS` when every row passed, else `RESULT: FAIL <n>`; exit non-zero on any failure. Rows:
  - (a) `an-unlisted-rename-is-caught`: the only change is a rename, the row lists another file → rc=1 and the output names the **new** path.
  - (b) `CONTROL-a-listed-rename-is-not-refused`: the same rename, the row lists the new path → rc=0, no output. This is the near miss: the fix must not turn every rename into a refusal.
  - (c) `a-quoted-path-with-a-space-is-listed`: a modified `with space.rs`, the row lists it → rc=0.
  - (d) `an-unlisted-path-with-a-space-is-caught`: the same file unlisted → rc=1 and the output is the path **without** quotes.
  - (e) `CONTROL-an-unlisted-modification-still-refuses`: a plain unlisted modification → rc=1 (the behaviour that must survive the rewrite).
  - (f) `a-listed-directory-covers-its-subtree`: a modification under a listed directory → rc=0.
  - (g) `a-deleted-file-is-checked`: a deletion the row does not list → rc=1.
- **R5. The mechanism stays tested by the selftest.** One new row in `scripts/autonomy/selftest.sh` named `the unlisted check reads a rename and a quoted path` that runs R4's script, writes its output to a file and greps **the file** (never a pipeline's status — the 2026-10-20 lesson), and PASSes only on `RESULT: PASS`. The selftest's own verdict must stay `40 passed, 0 failed` plus this row.

### Non-Functional
- POSIX bash, no new dependency, both scripts under the 450-line gate (they are ~35 and ~0 lines today). No change to `cycle.sh`, to any row's semantics, or to what `cycle.sh` does with a non-zero rc.
- The test script must not leave a repo behind that `git status` of the tree can see (`_tmp/` is ignored), and must not touch the tree's own index.

## TDD Contract
| Test | Expected |
|---|---|
| `an-unlisted-rename-is-caught` | rc=1, new path printed (RED today: rc=0, nothing printed) |
| `CONTROL-a-listed-rename-is-not-refused` | rc=0 (the near miss) |
| `a-quoted-path-with-a-space-is-listed` | rc=0 (RED today: rc=1) |
| `an-unlisted-path-with-a-space-is-caught` | rc=1, unquoted path |
| `CONTROL-an-unlisted-modification-still-refuses` | rc=1 (unchanged behaviour) |
| `a-listed-directory-covers-its-subtree` | rc=0 (unchanged behaviour) |
| `a-deleted-file-is-checked` | rc=1 |

**The requirement a plausible implementation satisfies only in appearance:** switching to `-z` and dropping the `case` guard looks like the whole fix, but a parser that takes the **first** path of a rename entry checks the OLD path, so (a) passes for the wrong path and (b) goes red. (b) is the control that decides the direction of the rename entry.

## Exit Criteria
- [ ] `test -f scripts/autonomy/tests/unlisted-changes-test.sh`
- [ ] `bash scripts/autonomy/tests/unlisted-changes-test.sh 2>&1 | grep -qE "^RESULT: PASS"`
- [ ] `bash scripts/autonomy/tests/unlisted-changes-test.sh 2>&1 | grep -q "^PASS an-unlisted-rename-is-caught"`
- [ ] `bash scripts/autonomy/tests/unlisted-changes-test.sh 2>&1 | grep -q "^PASS CONTROL-a-listed-rename-is-not-refused"`
- [ ] `bash scripts/autonomy/tests/unlisted-changes-test.sh 2>&1 | grep -q "^PASS a-quoted-path-with-a-space-is-listed"`
- [ ] `bash scripts/autonomy/tests/unlisted-changes-test.sh 2>&1 | grep -q "^PASS CONTROL-an-unlisted-modification-still-refuses"`
- [ ] `bash scripts/autonomy/tests/unlisted-changes-test.sh 2>&1 | grep -q "^PASS a-listed-directory-covers-its-subtree"`
- [ ] `bash scripts/autonomy/selftest.sh 2>&1 | grep -q "ALL MECHANISMS PASS"`
- [ ] `bash -n scripts/autonomy/unlisted-changes.sh scripts/autonomy/tests/unlisted-changes-test.sh`
- [ ] `git diff --quiet -- crates graphs`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT weaken the check to make an item pass: an exemption may not be added for a path a worker touches, and R4(e)/(f)/(g) may not be dropped or relaxed. If a row cannot pass without such a change, STOP and report it.
- Do NOT touch `cycle.sh`, `specs/`, `graphs/`, any crate, or any other row's semantics. Do NOT `git add`, commit or reset anything in the tree. No `rm -rf`: a scratch repo is created per row and left in `_tmp/`.
- If a MEASURE FIRST premise is false (in particular the `-z` field order for a rename), stop and say so before writing the parser.

## Follow-ups (not this spec)
1. Derive a row's path list from the spec's own file list, so the list cannot be hand-written short (the A136/A141 class).
2. A rename whose OLD path the row lists and whose NEW path it does not: today that is one refusal naming the new path; whether the message should also name the old one is a wording decision, not a check.

# Spec 283: a row's file list comes from its spec, not from memory

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the queue's own reliability. A row's commit-path column (plan.tsv column 6) is the only
statement of what a cycle may stage. It is hand-written, and three times it has been written short of what the
spec required: A136 omitted `nodes/coder.rs`, A141 omitted `graph_node.rs`, and A146 — 2026-10-26 — omitted
`crates/pii-server/tests/spec_graph_spend_limit_bites.rs`, the file R2(e) of spec 282 requires. The worker created
it; the row did not name it; the item had already been parked on a gate red for five hours and the row was a
second, independent reason it could not land. Spec 268/277 closed the *worker's* half (a change the row does not
name is refused). Nothing checks the *row's* half: that the list it carries is the list its spec requires.

The fix is a derivation, not a warning: a spec that a row dispatches must carry a `## Files` block naming exactly
the files its implementation adds or changes, and the queue compares the row's column against that block **before
it buys a worker**.

## Verified Premises
- **CONFIRMED** (read 2026-10-26) `scripts/autonomy/plan.tsv` rows carry seven tab-separated columns; column 6 is
  the commit paths, split on `;` by `cycle.sh`. `scripts/autonomy/state.py:72-101` is `pick`: it walks the plan in
  order, skips parked rows to stderr, and prints `NONE` when nothing is dispatchable.
- **CONFIRMED** the A146 row named two of the three files spec 282 R2 requires, and the missing one was the
  run-level proof; the row was authored by hand from a reading of the spec (commit `562d739`).
- **CONFIRMED** `scripts/autonomy/selftest.sh:173-176` shows the house pattern for a new row: run a test script
  into a file, then `grep -q` **the file** (never a pipeline's status — the 2026-10-20 lesson), and the selftest
  counts its rows dynamically (`RESULT: 41 passed, 0 failed` on 2026-10-26), so adding one does not edit a total.
- **NOT CLAIMED**: that a `## Files` block can be inferred from prose. It is declared by the spec's author; the
  check compares two declarations. A spec that declares nothing is refused, not guessed.

## Requirements
### Functional
- **R1. The spec declares its files.** House shape gains `## Files`: one repo-relative path per line, in
  backticks, exactly the files the implementation adds or changes — the same set the row's column 6 must carry.
  `specs/283-*.md` itself and `scripts/autonomy/plan.tsv` are NOT entries (the row lists the implementation's
  files; the spec and the row are committed by their author).
- **R2. One reader, no guessing.** New `scripts/autonomy/row-paths.py` (Python 3, stdlib only, style and root
  discovery as `state.py`, honouring `PII_PLAN`):
  - `files <spec.md>` → the `## Files` entries, one per line, in order; exit 0. A spec with no `## Files`, or an
    empty one, → `NO-FILES <spec>` on stdout, exit 2. An entry naming a directory (a trailing `/`, or a path that
    exists and is not a file) → `NOT-A-FILE <path>`, exit 3.
  - `check <row-id>` → `OK <id> <n>` and exit 0 when the row's column-6 set equals the spec's `## Files` set:
    `MISSING <path>` (in the spec, absent from the row) and `EXTRA <path>` (in the row, absent from the spec) each
    printed, exit 1 on any. A row whose spec cannot be read or declares no files exits 2/3 as above, and the row
    is never reported `OK`.
  - `check-all` → one line per **dispatchable** row (state not one of `done`, `skipped`, `revoked`): `OK <id> <n>`
    or `REFUSED <id>: <why>`; exit 1 if any row was refused, 0 otherwise. Read-only: it writes nothing, so it is
    safe while a cycle is in flight.
- **R3. The teeth are at the dispatch boundary.** `scripts/autonomy/state.py`'s `pick` runs the same comparison on
  each row it would otherwise offer, and skips a row that does not match, printing
  `refused <id>: <why>` to **stderr** (the channel `pick` already uses for parked rows). Its stdout contract does
  not change: one dispatchable row as TSV, or `NONE`. A hand-short row therefore cannot buy a worker dispatch.
  `state.py` is executed by `cycle.sh` on every tick, so it must be written through a temp file and `mv`'d into
  place — a concurrent `pick` sees the old file or the new one, never a partial one.
- **R4. Tests that could fail, with controls.** New `scripts/autonomy/tests/row-paths-test.sh`: each row builds its
  OWN scratch directory under `_tmp/` (created by the test, never `/tmp`), writes a fixture `plan.tsv` and a
  fixture spec, runs the real `scripts/autonomy/row-paths.py` against them, and prints `PASS <row>` or
  `FAIL <row>: <why>`; the last line is `RESULT: PASS` when every row passed, else `RESULT: FAIL <n>`; exit
  non-zero on any failure. Rows:
  - (a) `a-row-short-a-file-is-refused`: spec declares two files, the row lists one → rc=1, output names the
    missing path.
  - (b) `CONTROL-a-row-that-matches-is-accepted`: the same two files on both sides → rc=0, `OK` — the near miss;
    the check must not refuse a correct row.
  - (c) `an-extra-path-in-the-row-is-refused`: the row carries a third path the spec does not declare → rc=1,
    `EXTRA`.
  - (d) `a-spec-without-a-files-block-is-not-silently-accepted`: no `## Files` → rc=2, `NO-FILES`, and **not**
    `OK`.
  - (e) `a-directory-entry-is-not-a-file-list`: a Files entry `crates/x/` → rc=3, `NOT-A-FILE`.
  - (f) `CONTROL-a-pick-with-a-refused-row-still-offers-the-next`: a fixture plan with a refused row A1 and a
    matching row A2, and a `PII_STATE` fixture marking both `pending` → `pick` names A1 on stderr and prints A2
    on stdout (a refusal must not swallow the queue behind it).
- **R5. The mechanism stays tested by the selftest.** One new row in `scripts/autonomy/selftest.sh`, named `a
  row's file list is checked against its spec`, that runs R4's script, writes its output to a file and greps **the
  file**; PASS only on `RESULT: PASS`. The selftest's verdict must stay `ALL MECHANISMS PASS`.
- **R6. The refusal does not cost an existing test its meaning.** `pick` is driven as a black box by
  `crates/pii-graph/tests/pick_skips_parked.rs` (spec 169), whose fixture rows carried `-` in column 6 and no spec
  file at all. Under R3 those rows are refused for the new reason, `pick` answers `NONE`, and
  `a_parked_head_does_not_freeze_the_queue` fails — measured on this spec's first pass, gate red at
  `pick_skips_parked.rs:57` (`left: "NONE\n" right: "P2"`). The fixture therefore gains the declaration the new
  contract requires and no more: a `spec.md` under the temp dir with `## Files` naming the row's path, column 6
  carrying that same path, and `PII_SPECS` pointing at it. The assertion itself is unchanged — a parked head is
  still skipped, the next row is still picked, and `all_parked_says_so_and_picks_none` still answers `NONE`.

### Non-Functional
- Python 3 stdlib only; POSIX bash for the test. Every file under the 450-line gate.
- **No change to `cycle.sh` or `tick.sh`** (both are executed by the live loop; editing one under a running cycle
  is DO-NOT-REPEAT 2.15). Exit codes and the meaning of `pick`'s stdout are unchanged; the refusal is on stderr.
- Read-only: `row-paths.py` writes no file and touches no index; `check-all` is safe during a cycle.

## TDD Contract
| Test | Expected |
|---|---|
| `a-row-short-a-file-is-refused` | rc=1, `MISSING <path>` (RED today: no such check exists) |
| `CONTROL-a-row-that-matches-is-accepted` | rc=0, `OK` (the near miss) |
| `an-extra-path-in-the-row-is-refused` | rc=1, `EXTRA <path>` |
| `a-spec-without-a-files-block-is-not-silently-accepted` | rc=2, `NO-FILES`, never `OK` |
| `a-directory-entry-is-not-a-file-list` | rc=3, `NOT-A-FILE` |
| `CONTROL-a-pick-with-a-refused-row-still-offers-the-next` | refused row on stderr, next row on stdout |
| `a_parked_head_does_not_freeze_the_queue` (existing, spec 169) | parked head skipped, `P2` picked — red under R3 until its fixture declares its own `## Files` (R6) |

**The requirement a plausible implementation satisfies only in appearance:** treating a spec with no `## Files`
as "nothing to check" — i.e. passing it. That is the silent pass this spec exists to stop, so (d) is the row that
decides it, and (b) is the control that keeps the check from refusing everything.

## Exit Criteria
- [ ] `test -f scripts/autonomy/row-paths.py`
- [ ] `test -f scripts/autonomy/tests/row-paths-test.sh`
- [ ] `python3 -m py_compile scripts/autonomy/row-paths.py`
- [ ] `bash scripts/autonomy/tests/row-paths-test.sh 2>&1 | grep -qE "^RESULT: PASS"`
- [ ] `bash scripts/autonomy/tests/row-paths-test.sh 2>&1 | grep -q "^PASS a-row-short-a-file-is-refused"`
- [ ] `bash scripts/autonomy/tests/row-paths-test.sh 2>&1 | grep -q "^PASS CONTROL-a-row-that-matches-is-accepted"`
- [ ] `bash scripts/autonomy/tests/row-paths-test.sh 2>&1 | grep -q "^PASS an-extra-path-in-the-row-is-refused"`
- [ ] `bash scripts/autonomy/tests/row-paths-test.sh 2>&1 | grep -q "^PASS a-spec-without-a-files-block-is-not-silently-accepted"`
- [ ] `bash scripts/autonomy/tests/row-paths-test.sh 2>&1 | grep -q "^PASS a-directory-entry-is-not-a-file-list"`
- [ ] `bash scripts/autonomy/tests/row-paths-test.sh 2>&1 | grep -q "^PASS CONTROL-a-pick-with-a-refused-row-still-offers-the-next"`
- [ ] `python3 scripts/autonomy/row-paths.py check-all`
- [ ] `python3 scripts/autonomy/state.py pick | grep -qE "^(A[0-9]+|NONE)$"`
- [ ] `bash -n scripts/autonomy/tests/row-paths-test.sh`
- [ ] `bash scripts/autonomy/selftest.sh 2>&1 | grep -q "ALL MECHANISMS PASS"`
- [ ] `git diff --quiet -- graphs`
- [ ] `test -z "$(git status --porcelain crates | grep -v 'crates/pii-graph/tests/pick_skips_parked.rs')"`
- [ ] `git diff --quiet -- scripts/autonomy/cycle.sh scripts/autonomy/tick.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `cargo test -q -p pii-graph --test pick_skips_parked 2>&1 | grep -qE "test result: ok\. 2 passed"`

## Guardrails
- Do NOT edit `scripts/autonomy/cycle.sh` or `scripts/autonomy/tick.sh`. Do NOT edit any file under `specs/` —
  specs are the planner's, and adding a `## Files` block to one is planner work, not implementation.
- Do NOT touch `graphs/`. The ONLY file under `crates/` this spec may change is the fixture in
  `crates/pii-graph/tests/pick_skips_parked.rs` (R6) — no other crate file, and never an assertion.
- Do NOT commit.
- Write `scripts/autonomy/state.py` through a temp file and `mv` it into place (R3): it runs on every tick.
- If a **real** plan row's spec has no `## Files` and the row is dispatchable, STOP and report it — do not invent
  the block. (On 2026-10-26 the only open row is A147, whose spec is this one.)
- Never widen a row's paths or weaken a criterion to make an item pass; if the check cannot be made to agree with
  a correct row, that is a finding to report, not a test to loosen.

## Follow-ups (not this spec)
1. Derive the row's column-6 text itself from `## Files` at authoring time (one writer), so a short row becomes
   impossible to type rather than merely refused.
2. The same derivation for the `verify` column: 282's own row carried 8 commands and spec 141's carried 5 of 6 —
   the class is the same, the separator (`;`) is the trap.

## Files
- `scripts/autonomy/row-paths.py`
- `scripts/autonomy/tests/row-paths-test.sh`
- `scripts/autonomy/state.py`
- `scripts/autonomy/selftest.sh`
- `crates/pii-graph/tests/pick_skips_parked.rs`

## Closure

**Closed by the orchestrator on 2026-10-26**, after a paid worker delivered the implementation and the queue parked
the row on a `verify-failed` that no later run could reproduce.

- **The worker delivered; the cycle's verify failed anyway.** Cycle `20261026T064317Z` dispatched A147 (session
  `6fc6558a-9f97-45d9-a24f-0beeb0df6444`): `status=succeeded`, rc=0, 0.065986 USD, four dirty paths — exactly this
  spec's files at that moment. `_tmp/autonomy/loop-events.ndjson` names the criterion that failed:
  `loop.verify item=A147 FAILED: bash scripts/autonomy/selftest.sh 2>&1 | grep -q "ALL MECHANISMS PASS"` (06:48:07Z).
  From then until 12:13Z the queue answered `pick: every open row is parked: A147(needs-review)` **61 times** — five
  and a half hours in which nothing was read, nothing was spent and nothing advanced.
- **That failure did not reproduce.** On the tree the worker left, the criteria were green twice: plain, and again
  with `_tmp/autonomy/lock` held by `flock` for the whole selftest — the one condition a cycle adds and a hand run
  does not. The selftest read `RESULT: 42 passed, 0 failed` / `VERDICT: ALL MECHANISMS PASS`
  (`_tmp/autonomy/logs/selftest-repro1.log`, `selftest-inflight.log`).
- **The gate — which the failed verify had prevented from ever running — was red for a real reason.**
  `fresh-gate --worktree` failed at `crates/pii-graph/tests/pick_skips_parked.rs:57`:
  `assertion left == right failed: head needs-review must be skipped: NONE`, `left: "NONE\n" right: "P2"`. The spec
  169 test drives `pick` as a black box with fixture rows carrying `-` in column 6 and no spec file at all; under R3
  those rows are refused for the new reason, so the test was measuring the refusal instead of the parked-head skip.
- **The cause was this spec's own scope, not the worker's code.** Its guardrail said "Do NOT touch `crates/`",
  forbidding the only file that could restore the test. Amended in this closure, committed with the work: **R6**
  requires the fixture to declare its own `## Files` (`PII_SPECS`, a `spec.md`, column 6 naming that path) with the
  assertion untouched; the guardrail now names that one file as the sole permitted `crates/` change; the
  "crates untouched" criterion was narrowed to "nothing under `crates/` changed except that file" — it contradicted
  R6 as written — and a new criterion runs the test itself.
- **Evidence, all measured on this tree.** **19 of 19** Exit Criteria green. Three mutations, each red, each restored
  byte-identical (`sha256sum` before == after):
  - **m1** `judge` drops the MISSING side (`if missing or extra` -> `if extra`): `a-row-short-a-file-is-refused` red
    (`rc=0, want 1 and MISSING crates/x/src/b.rs`) and the pick control red (`picked 'A1', want A2`),
    `RESULT: FAIL 2` — restored, sha `f2baae84c953d4a4…` identical, `RESULT: PASS`.
  - **m2** `pick`'s refusal block removed from `state.py`: `CONTROL-a-pick-with-a-refused-row-still-offers-the-next`
    red (`picked 'A1', want A2 with 'refused A1' on stderr`), `RESULT: FAIL 1` — restored, sha
    `113a507cb1f97e49…` identical, `RESULT: PASS`.
  - **m3** the fixture's column 6 back to `-`: `pick_skips_parked` `FAILED. 1 passed; 1 failed`, `NONE` vs `P2` —
    restored, sha `e6db3567ecf5c60b…` identical, `2 passed`.
  - The near-miss control (b) stayed green under m1 and m2: the check never started refusing a correct row.
- **The row and the spec agree.** A147 carries seven columns, its `verify` column equals this spec's Exit Criteria
  command for command (**19 = 19**, no criterion carrying the `;` separator), all five of its commit paths exist,
  and `row-paths.py check A147` prints `OK A147 5`.
- **Gate:** `fresh-gate --worktree` PASS (`_tmp/autonomy/logs/gate-a147-worktree-3.log`, `fresh-gate: PASS`), and the queue's own door `quality-gate.sh --routine` with coverage pinned on: **coverage 82.92% >= 74.00%**, `quality-gate: ALL PASS` (`_tmp/autonomy/logs/gate-a147-repo-coverage.log`). The gate cache had to be reclaimed first: `/projects` was at 100% and the gate's 12 GB floor refused the run.

### Follow-ups this closure adds
3. The selftest criterion should `tee` its output to a dated log and grep the file, so a failing row is named by the
   artefact instead of lost to a pipe — that blindness is why this park cost half a day.
4. A `verify-failed` that no later run reproduces should not park a row for hours: re-run the single failing
   criterion once on the same tree before incrementing the strike count, so a transient costs a second, not a day.

# Spec 303: a resumed queue row keeps the lane it started in, finds its session, and its commit says which lane produced it

## Overview
**Project**: <repo>
**Phase**: production — composability, lane hardening (Opus review findings F5 and F6, MEASURED 2026-10-27 23:4xZ on A166's unattended resume)
Observed on the real system when the cycle resumed A166 with its session: `LANE-GRAPH: coder (resume is not supported by the spec graph)`, then `== NOT resuming: session a8811921-... is not in the devbox (it was made by another lane); starting fresh on the tree as it is`.
So a resumed row (a) silently leaves the composed lane for the one-node graph: no judges, no docs call; (b) cannot find the session it asked to resume, because the transcript is not under `<agent-config>` in the container; (c) lands a commit that says nothing about either.
A row is resumed whenever a window parks it, a quota requeues it, or a person re-pends it, so this is the normal recovery path, not a corner.

## Requirements
### Functional
- R1: the spec graph accepts an optional `sid` in its run input for the `code` coder (the coder node already supports `sid` in the one-node graph; reuse that key, no new engine concept). With `--resume <sid>` in the spec lane the lane puts `sid` in the input JSON and keeps `LANE=spec`: no fallback to the coder lane. The message `LANE-GRAPH: coder (resume is not supported by the spec graph)` is removed. The test that asserted the removed fallback is `resume_falls_back` in `scripts/autonomy/tests/lane-graph-select.test.sh`; it is replaced by `resume_stays_on_the_spec_lane` over the same fixture (LANE-GRAPH spec, `sid` in the input, no fallback line), and `spec_graph_is_selected` gains `sid` in its expected key list. No other existing assertion is edited.
- R2: if the session transcript cannot be found in the container the lane still starts a fresh composed run (a resume is an optimisation, never a reason to refuse), but prints `RESUME: session <sid> not found in the devbox, starting fresh` AND writes the same line to the worker log. The lookup path is taken from the place the session was actually captured (see R3), not hard-coded to `<agent-config>`.
- R3: measured in TDD: after a composed run, find where the container keeps the claude session of `r000000000463`/`r000000000460` (the lane already captures `SID` from the first `coder.spawned`). The lookup in R2 uses that location. If the CLAUDE_CODE_TMPDIR/`devbox-test/tmp` location is the only one, the lookup searches it and the spec says so; the finding goes in the spec's Verified Premises before the change is made.
- R5 (found by the first run of this spec, r000000000465): spec 302's check that the three graphs equal HEAD refuses a dry run once a row has edited a graph, which makes any test that reads the working-tree graphs unpassable inside a row that must edit one. The check gains ONE explicit opt-out, the environment variable `PII_GRAPH_EDIT_OK=1`: with it the lane skips the HEAD comparison and prints `NOTE: graph edits allowed (PII_GRAPH_EDIT_OK)`. Default unchanged: unset or any other value keeps the abort. Only the tests set it, on their own dry runs: `scripts/autonomy/tests/lane-input-matches-graph.test.sh` and the three `lane-pin-head.test.sh` cases that exercise the opt-out itself. `cycle.sh`, `env.sh` and the lane never set it, and the lane ignores it when not in a dry run (a real start with edited graphs still aborts).
- R6 (measured while this spec was being finished, 2026-10-28 00:1xZ): the pin also blocks the ROW THAT EDITS A GRAPH from passing its own criteria — with `graphs/spec.toml` dirty, 15 lane cases aborted on `differs from HEAD` (lane-preflight 10 of 12, lane-graph-select 4 of 8, lane-pin-head 1 of 7), and not one of them is a graph test. So the graph half of R1 (`sid` on `code`) is committed as a PREREQUISITE before the row is dispatched (`4fab34b`), the worker does not edit a graph, and the opt-out stays a dry-run door for the next row that must edit one.
- R4: every commit made by the cycle for a row carries one trailer line `Lane: spec` or `Lane: coder` (read from the worker log's `LANE-GRAPH:` line, the LAST one), appended to the queue's commit message. `cycle.sh`'s commit step is the only place edited there; the trailer is absent when the log has no `LANE-GRAPH:` line (old logs).

### Non-Functional
- Shell only. Production files: `scripts/run-batchC-pii.sh`, `scripts/autonomy/cycle.sh` (the commit message expression only), `graphs/spec.toml` (one optional input key on `code`). Tests use the stub mechanism (`PII_SSH`, `PII_DAEMON_SH`); no real ssh, daemon or agent.

## Verified Premises
- **CONFIRMED** (2026-10-27 23:4xZ, A166 cycle log): `LANE-GRAPH: coder (resume is not supported by the spec graph)` then `== NOT resuming: session a8811921-3ce1-425e-8aa3-e721ba7ad0e2 is not in the devbox`.
- **CONFIRMED** (Opus review) for A160's `be22f443…` and A161's `8d08c324…`: `ls <agent-config>` finds nothing in the container; `find` locates only a temp dir under `_tmp/devbox-test/tmp/claude-0/-projects-pi-improved/<sid>`.
- **INFERRED** the sessions are written under `CLAUDE_CODE_TMPDIR` (`_tmp/devbox-test/tmp`) because the daemon sets it; R3's first TDD step proves or refutes this with `find` before the code changes.
- **CONFIRMED** `run-batchC-pii.sh:77-80` is the fallback; `:85-91` is the lookup.
- **MEASURED** (2026-10-28 00:15–00:25Z, this row's own tree): with `graphs/spec.toml` edited and uncommitted, every spec-lane case that runs the lane on the real repository aborts at the pin — `lane-preflight` 2 pass / 10 fail, `lane-graph-select` 4/4, `lane-pin-head` 6/1, `lane-input-matches-graph` 0/1. After the graph declaration went in at `4fab34b`: 12/0, 6/2 (the two cases R1 rewrites), 11/0 (7 original + 3 opt-out cases + controls), 1/0. The remaining failures were the two assertions that encode the pre-303 behaviour, not the pin.

## TDD Contract
| Test (scripts/autonomy/tests/lane-resume.test.sh) | Input | Expected |
|---|---|---|
| `resume_stays_on_the_spec_lane` | `PII_LANE_GRAPH=spec`, `--resume S`, dry run | `LANE-GRAPH: spec`, input JSON has `"sid":"S"`, no "resume is not supported" text |
| `CONTROL_without_resume_has_no_sid` | same without `--resume` | `LANE-GRAPH: spec`, no `sid` key |
| `missing_transcript_starts_fresh_and_says_so` | stub ssh: transcript lookup fails | exit 0 path continues, `RESUME: session S not found in the devbox, starting fresh` in stdout AND in the log |
| `CONTROL_found_transcript_resumes` | stub ssh: lookup succeeds | no `not found` line, `"sid"` kept |
| `commit_message_carries_the_lane` (scripts/autonomy/tests/cycle-lane-trailer.test.sh) | the cycle's message builder run on a log with `LANE-GRAPH: spec` | the message ends with `Lane: spec` |
| `CONTROL_no_lane_line_no_trailer` | a log without it | no `Lane:` line |
| `last_lane_line_wins` | a log with `coder` then `spec` | `Lane: spec` |
| `spec_graph_declares_sid_optional` | `graphs/spec.toml` | the `code` node reads `sid` from the input with a default of empty, and `lane-input-matches-graph.test.sh` still passes |

### Added by the amendment (R5)
| Test | Input | Expected |
|---|---|---|
| `lane-pin-head.test.sh` `CONTROL_the_opt_out_unset_still_aborts` | an edited spec.toml clone, dry run, `PII_GRAPH_EDIT_OK` unset, and again set to `0` | exit 2, `differs from HEAD` both times |
| `lane-pin-head.test.sh` `opt_out_allows_a_dry_run` | the same clone with `PII_GRAPH_EDIT_OK=1`, dry run | exit 0, `NOTE: graph edits allowed` |
| `lane-pin-head.test.sh` `CONTROL_the_opt_out_does_not_cover_a_real_start` | same clone, `PII_GRAPH_EDIT_OK=1`, NOT a dry run, stub ssh/daemon | exit 2, `differs from HEAD`, nothing started |
| `lane-input-matches-graph.test.sh` | its dry runs | set `PII_GRAPH_EDIT_OK=1` and pass |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/lane-resume.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(bash scripts/autonomy/tests/lane-resume.test.sh 2>&1 | grep -c '^ok')" -ge 5`
- [ ] `bash scripts/autonomy/tests/cycle-lane-trailer.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-input-matches-graph.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-preflight.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `grep -q 'PII_GRAPH_EDIT_OK' scripts/autonomy/tests/lane-input-matches-graph.test.sh`
- [ ] `bash scripts/autonomy/tests/lane-pin-head.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(grep -rl PII_GRAPH_EDIT_OK scripts/autonomy --include='*.sh' | grep -v tests/ | wc -l)" -eq 0`
- [ ] `bash -n scripts/run-batchC-pii.sh`
- [ ] `bash -n scripts/autonomy/cycle.sh`
- [ ] `git diff --quiet -- crates scripts/autonomy/env.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

Criterion 9 reads shell files only (`--include='*.sh'`) and that is not a relaxation: the marker can only be set by shell code, and the queue row's own verify text lives in `scripts/autonomy/plan.tsv`, which carries this criterion's literal string. Measured 2026-10-28: the unfiltered grep reports `plan.tsv` and can never reach 0 once the row exists — an unsatisfiable criterion, not a strict one.

## Files
- `scripts/run-batchC-pii.sh`
- `scripts/autonomy/cycle.sh`
- `graphs/spec.toml`
- `scripts/autonomy/tests/lane-resume.test.sh`
- `scripts/autonomy/tests/cycle-lane-trailer.test.sh`
- `scripts/autonomy/tests/lane-input-matches-graph.test.sh`
- `scripts/autonomy/tests/lane-graph-select.test.sh`
- `scripts/autonomy/tests/lane-pin-head.test.sh`


## Guardrails
- `graphs/spec.toml`: only the optional `sid` on `code`. The docs pin is NOT touched (docs.toml is not edited). If the pin needs no change nothing else moves; if `lane-input-matches-graph` needs the lane to supply `sid`, that is part of R1.
- Do NOT edit `crates/`, `env.sh`, or an existing assertion (new cases and the opt-out line are additions). cycle.sh: only the commit message expression. Scratch under `_tmp/` inside the repository.
- The appearance-only implementation: a lane that keeps `LANE=spec` on resume but drops `sid` from the input (so the "resume" starts a fresh session silently). The test that catches it: `resume_stays_on_the_spec_lane` reads `"sid"` from the input JSON the lane really wrote.
- Mutations the tests must kill: the fallback restored; `sid` omitted from the input; the not-found line missing from the log; the trailer read from the first `LANE-GRAPH:` line; a trailer when no lane line exists.

## Closure
Closed by the orchestrator, 2026-10-28 00:44Z, commit `275c2af`.

- **Criteria:** all 13 run by the orchestrator on the tree the cycle was about to commit, `rc=0` each, and then re-run by the cycle itself (`_tmp/autonomy/logs/cycle-20261028T002344Z.log`: every `verify:` line `rc=0`).
- **Gate:** the cycle's routine gate, 15 steps, `gate PASS`, coverage **measured** (not skipped) at `00:44:17Z`; `head-builds: HEAD 275c2af builds`.
- **Reddened by the orchestrator's own mutations, each restored byte-identical (`sha256sum` before/after, file `touch`ed, test re-run green):**
  | mutation | file | outcome |
  |---|---|---|
  | the opt-out also covers a real start (`&& [ "$DRY" = 1 ]` removed) | `run-batchC-pii.sh` | `CONTROL_the_opt_out_does_not_cover_a_real_start` red (10 ok / 1 FAIL) |
  | the opt-out never fires (`if false`) | `run-batchC-pii.sh` | `opt_out_allows_a_dry_run` red (10 / 1) |
  | `sid` dropped from the run input | `run-batchC-pii.sh` | `resume_stays_on_the_spec_lane` and `CONTROL_found_transcript_resumes` red (4 / 2) |
  | the trailer reads the FIRST `LANE-GRAPH:` line | `cycle.sh` | `last_lane_line_wins` and `first_lane_line_does_not_win` red (3 / 2) |
  | a production file sets `PII_GRAPH_EDIT_OK` (planted as `export` in `cycle.sh`) | `cycle.sh` | criterion 9 red, `cycle.sh` named; green once the plant was removed |
  Restores hashed: `run-batchC-pii.sh` `ddeb4fa347fffed0254ddedd9d51f7ae28be7ce370b5c4353419050a1e66c552`, `cycle.sh` `b9d3c3bc1c700fefce8d29e928f0f4d5a0ba9811c9cb2076c6378eddae3ff8b6`.
- **The prerequisite, as R6 records it:** the graph declaration (`sid` on `code`) went in first (`4fab34b`) because spec 302's pin refuses every spec-lane case while the row that must edit the graph holds it uncommitted — measured 15 cases over four test files, none of them a graph test.
- **What this commit does NOT prove (measured, not claimed):** `275c2af` carries **no** `Lane:` trailer although the worker log `_tmp/claude-batchC/303-….1793146913.log` has `LANE-GRAPH: coder`. Cause: `cycle.sh:260` assigns `WLOG` only inside the dispatch branch, so a commit produced after a phase-marker resume (`SKIP_DISPATCH`) reads an empty handle and R4's trailer is skipped without a word. A second hole in the same line: the newest log was run `r000000000466`, a 5-second run with `tree_changed=no`, `cost_usd=0.000000`, while the tree came from `r000000000465` in the spec lane — with `WLOG` fixed alone the trailer would have named the wrong lane. Spec 304 / row A168 carries the fix.
- **Also not done here:** the lane knob `_tmp/autonomy/lane-graph` reads `coder` (mtime `00:19:08Z`, not set by this tick), so no run this tick used the composed lane; the next dispatch reads the knob as it stands. The work the orchestrator finished (R5's dry-run opt-out, the two superseded `lane-graph-select` cases) was verified by the same 13 criteria and the five mutations above, not by a fresh worker session — the row was resumed at verify deliberately, after a worker run on the stale problem.

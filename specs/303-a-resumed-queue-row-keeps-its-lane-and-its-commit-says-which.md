# Spec 303: a resumed queue row keeps the lane it started in, finds its session, and its commit says which lane produced it

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composability, lane hardening (Opus review findings F5 and F6, MEASURED 2026-10-27 23:4xZ on A166's unattended resume)
Observed on the real system when the cycle resumed A166 with its session: `LANE-GRAPH: coder (resume is not supported by the spec graph)`, then `== NOT resuming: session a8811921-... is not in the devbox (it was made by another lane); starting fresh on the tree as it is`.
So a resumed row (a) silently leaves the composed lane for the one-node graph: no judges, no docs call; (b) cannot find the session it asked to resume, because the transcript is not under `/root/.claude/projects/*/<sid>.jsonl` in the container; (c) lands a commit that says nothing about either.
A row is resumed whenever a window parks it, a quota requeues it, or a person re-pends it, so this is the normal recovery path, not a corner.

## Requirements
### Functional
- R1: the spec graph accepts an optional `sid` in its run input for the `code` coder (the coder node already supports `sid` in the one-node graph; reuse that key, no new engine concept). With `--resume <sid>` in the spec lane the lane puts `sid` in the input JSON and keeps `LANE=spec`: no fallback to the coder lane. The message `LANE-GRAPH: coder (resume is not supported by the spec graph)` is removed.
- R2: if the session transcript cannot be found in the container the lane still starts a fresh composed run (a resume is an optimisation, never a reason to refuse), but prints `RESUME: session <sid> not found in the devbox, starting fresh` AND writes the same line to the worker log. The lookup path is taken from the place the session was actually captured (see R3), not hard-coded to `/root/.claude/projects/*/`.
- R3: measured in TDD: after a composed run, find where the container keeps the claude session of `r000000000463`/`r000000000460` (the lane already captures `SID` from the first `coder.spawned`). The lookup in R2 uses that location. If the CLAUDE_CODE_TMPDIR/`devbox-test/tmp` location is the only one, the lookup searches it and the spec says so; the finding goes in the spec's Verified Premises before the change is made.
- R5 (found by the first run of this spec, r000000000465): spec 302's check that the three graphs equal HEAD refuses a dry run once a row has edited a graph, which makes any test that reads the working-tree graphs unpassable inside a row that must edit one. The check gains ONE explicit opt-out, the environment variable `PII_GRAPH_EDIT_OK=1`: with it the lane skips the HEAD comparison and prints `NOTE: graph edits allowed (PII_GRAPH_EDIT_OK)`. Default unchanged: unset or any other value keeps the abort. Only `scripts/autonomy/tests/lane-input-matches-graph.test.sh` sets it (on its own dry runs); `cycle.sh`, `env.sh` and the lane never set it, and the lane ignores it when not in a dry run (a real start with edited graphs still aborts).
- R4: every commit made by the cycle for a row carries one trailer line `Lane: spec` or `Lane: coder` (read from the worker log's `LANE-GRAPH:` line, the LAST one), appended to the queue's commit message. `cycle.sh`'s commit step is the only place edited there; the trailer is absent when the log has no `LANE-GRAPH:` line (old logs).

### Non-Functional
- Shell only. Production files: `scripts/run-batchC-pii.sh`, `scripts/autonomy/cycle.sh` (the commit message expression only), `graphs/spec.toml` (one optional input key on `code`). Tests use the stub mechanism (`PII_SSH`, `PII_DAEMON_SH`); no real ssh, daemon or agent.

## Verified Premises
- **CONFIRMED** (2026-10-27 23:4xZ, A166 cycle log): `LANE-GRAPH: coder (resume is not supported by the spec graph)` then `== NOT resuming: session a8811921-3ce1-425e-8aa3-e721ba7ad0e2 is not in the devbox`.
- **CONFIRMED** (Opus review) for A160's `be22f443…` and A161's `8d08c324…`: `ls /root/.claude/projects/*/<sid>.jsonl` finds nothing in the container; `find` locates only a temp dir under `_tmp/devbox-test/tmp/claude-0/-projects-pi-improved/<sid>`.
- **INFERRED** the sessions are written under `CLAUDE_CODE_TMPDIR` (`/projects/pi-improved/_tmp/devbox-test/tmp`) because the daemon sets it; R3's first TDD step proves or refutes this with `find` before the code changes.
- **CONFIRMED** `run-batchC-pii.sh:77-80` is the fallback; `:85-91` is the lookup.

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
- [ ] `test "$(grep -rl PII_GRAPH_EDIT_OK scripts/autonomy | grep -v tests/ | wc -l)" -eq 0`
- [ ] `bash -n scripts/run-batchC-pii.sh`
- [ ] `bash -n scripts/autonomy/cycle.sh`
- [ ] `git diff --quiet -- crates scripts/autonomy/env.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/run-batchC-pii.sh`
- `scripts/autonomy/cycle.sh`
- `graphs/spec.toml`
- `scripts/autonomy/tests/lane-resume.test.sh`
- `scripts/autonomy/tests/cycle-lane-trailer.test.sh`
- `scripts/autonomy/tests/lane-input-matches-graph.test.sh`
- `scripts/autonomy/tests/lane-pin-head.test.sh`


## Guardrails
- `graphs/spec.toml`: only the optional `sid` on `code`. The docs pin is NOT touched (docs.toml is not edited). If the pin needs no change nothing else moves; if `lane-input-matches-graph` needs the lane to supply `sid`, that is part of R1.
- Do NOT edit `crates/`, `env.sh`, or an existing assertion (new cases and the opt-out line are additions). cycle.sh: only the commit message expression. Scratch under `_tmp/` inside the repository.
- The appearance-only implementation: a lane that keeps `LANE=spec` on resume but drops `sid` from the input (so the "resume" starts a fresh session silently). The test that catches it: `resume_stays_on_the_spec_lane` reads `"sid"` from the input JSON the lane really wrote.
- Mutations the tests must kill: the fallback restored; `sid` omitted from the input; the not-found line missing from the log; the trailer read from the first `LANE-GRAPH:` line; a trailer when no lane line exists.

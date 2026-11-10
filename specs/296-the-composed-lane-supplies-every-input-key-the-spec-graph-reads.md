# Spec 296: the composed lane supplies every input key the spec graph reads

## Overview
**Project**: <repo>
**Phase**: production — composability, drift guard. `scripts/run-batchC-pii.sh` (spec 294) builds the input of the tracked `spec` graph by listing nine keys in a Python snippet.
`graphs/spec.toml` and `graphs/docs.toml` read their own set (`$.input.<key>`), and nothing ties the two. Add a key to either graph and the composed lane starts every row with a
missing input: the run is refused at start (spec 288) and every queue row parks, with the cause two files away from the symptom. This spec adds the test that ties them.

## Requirements
### Functional
- R1: a test `scripts/autonomy/tests/lane-input-matches-graph.test.sh` reads every `$.input.<key>` named in `graphs/spec.toml` and `graphs/docs.toml`, runs `scripts/run-batchC-pii.sh 296 --dry-run` with `PII_LANE_GRAPH=spec`, reads the keys of the input JSON it writes (the `INPUT :` line names the file), and asserts the graph's keys are a SUBSET of the lane's. Keys the lane supplies that no graph reads are reported on one `NOTE` line and are not a failure.
- R2: the test prints one `ok`/`FAIL` line per check and ends `PASS: <n> FAIL: <m>`.
- R3: CONTROL: with `docs_brief` removed from a COPY of the lane's input the same comparison reports it missing and the check fails (the test builds this case itself from the real input, it does not edit the script).
- R4: CONTROL: a key added to a COPY of `graphs/spec.toml` (`$.input.zz_new_key`) is reported as missing from the lane.

### Non-Functional
- Pure shell plus `uv run --no-project python` for JSON, no new dependency, no daemon, no ssh, no agent, nothing spent. Read the repo's files; write only under a scratch dir inside `_tmp/`, moved to `_tmp/_trash` at the end.

## Verified Premises
- **CONFIRMED** (read 2026-10-27) `grep -oE '\$\.input\.[a-z_]+' graphs/spec.toml graphs/docs.toml` gives nine distinct keys: agent, base, brief, docs_brief, max_budget_usd, model, provider, spec, workdir.
- **CONFIRMED** `scripts/run-batchC-pii.sh --dry-run` (spec 294) writes `$LOGD/input-<tag>.json` and prints its path on an `INPUT :` line, then exits before the daemon.
- **CONFIRMED** the lane's input today carries agent, model, provider, spec, base, workdir, brief, docs_brief and max_budget_usd.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| `lane_supplies_every_key_the_graphs_read` | the real graphs and the real dry-run | graph keys ⊆ lane keys |
| `CONTROL_a_missing_docs_brief_is_caught` | the input without `docs_brief` | reported missing, check fails |
| `CONTROL_a_new_graph_key_is_caught` | a copy of spec.toml with `$.input.zz_new_key` | reported missing |
| `lane_extra_keys_are_a_note_not_a_failure` | a lane key no graph reads | NOTE line, no FAIL |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/lane-input-matches-graph.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(bash scripts/autonomy/tests/lane-input-matches-graph.test.sh 2>&1 | grep -c '^ok')" -ge 4`
- [ ] `bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `git diff --quiet -- graphs scripts/run-batchC-pii.sh scripts/autonomy/env.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/autonomy/tests/lane-input-matches-graph.test.sh`

## Guardrails
- Do NOT edit `graphs/`, `run-batchC-pii.sh` or `env.sh`. Do NOT start a daemon, use ssh or spawn an agent. Do NOT edit an existing assertion.
- Mutations to kill: the comparison is reversed (lane ⊆ graph), only spec.toml is read, docs.toml is skipped, a missing key is a NOTE, the control builds from a stale copy.

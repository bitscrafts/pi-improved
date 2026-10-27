# Spec 302: the composed lane checks the graph pin against HEAD, not against the working tree

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composability, lane hardening (Opus review 2026-10-27, finding F9)
`scripts/run-batchC-pii.sh` checks that `graphs/docs.toml`'s sha256 equals the pin in `graphs/spec.toml`, but both files are read from the WORKING TREE, and the lane then copies the working-tree graphs to the daemon.
A worker that edits `docs.toml` and the pin together passes the check, and if the cycle then refuses its row, the edited graphs stay in the tree and steer the next row's run. The pin is meant to freeze the reviewed graph.

## Requirements
### Functional
- R1: in the spec lane, before the pin comparison, the lane requires `graphs/spec.toml`, `graphs/docs.toml` and `graphs/preflight.toml` to equal their content at `HEAD` (`git diff --quiet HEAD -- graphs/spec.toml graphs/docs.toml graphs/preflight.toml`, and none untracked). On a difference it prints `ABORT: graphs/<file> differs from HEAD (the pin protects the reviewed graph): nothing started` and exits 2 BEFORE anything is spent. The existing pin comparison stays.
- R2: `--dry-run` runs the same check (it spends nothing) so the test needs no stub for it.

### Non-Functional
- Shell only; `scripts/run-batchC-pii.sh` is the only production file edited. The coder lane (`PII_LANE_GRAPH` unset) is untouched.

## Verified Premises
- **CONFIRMED** (2026-10-27) the pin check reads `$REPO/graphs/spec.toml` and `$REPO/graphs/docs.toml` from the working tree (`PIN=$(grep ... "$REPO/graphs/spec.toml")`, `DIG=$(sha256sum "$REPO/graphs/docs.toml")`).
- **CONFIRMED** the lane copies `graphs/*.toml` from the working tree to the daemon on every dispatch.
- **CONFIRMED** the Opus review (finding 9) names this and rates the likelihood low.

## TDD Contract
| Test (scripts/autonomy/tests/lane-pin-head.test.sh) | Input | Expected |
|---|---|---|
| `clean_graphs_pass` | the real repo, dry run, spec lane | no `ABORT`, `DRY RUN` printed |
| `CONTROL_a_matching_edit_of_docs_and_pin_is_refused` | a scratch clone where docs.toml gains a comment AND the pin in spec.toml is repaired to match | exit 2, `differs from HEAD`, nothing started |
| `CONTROL_an_untracked_graph_is_refused` | a scratch clone with `graphs/preflight.toml` removed from the index (untracked) | exit 2 |
| `coder_lane_ignores_graph_edits` | the same edited clone with `PII_LANE_GRAPH` unset, dry run | no `differs from HEAD` line |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/lane-pin-head.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(bash scripts/autonomy/tests/lane-pin-head.test.sh 2>&1 | grep -c '^ok')" -ge 4`
- [ ] `bash scripts/autonomy/tests/lane-preflight.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-input-matches-graph.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash -n scripts/run-batchC-pii.sh`
- [ ] `git diff --quiet -- graphs crates scripts/autonomy/env.sh scripts/autonomy/cycle.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/run-batchC-pii.sh`
- `scripts/autonomy/tests/lane-pin-head.test.sh`

## Guardrails
- Do NOT edit `graphs/`, `crates/`, `cycle.sh`, `env.sh` or an existing assertion. A test works in a scratch clone under `_tmp/` (moved, never deleted); it never edits the live tree's graphs. Scratch for the worker goes under `_tmp/` inside the repository (the judge's sandbox writes nowhere else).
- Mutations the tests must kill: the check reading the working tree again; the check skipped in dry run; only spec.toml checked; an untracked file accepted; the check applied to the coder lane.

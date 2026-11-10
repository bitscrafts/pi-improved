# Spec 294: the queue's worker can start the composed `spec` graph, opt in, and says which graph it ran

## Overview
**Project**: <repo>
**Phase**: production — the last step before every spec runs composed. `scripts/run-batchC-pii.sh` (the queue's worker, `PII_WORKER=pii`) writes a ONE-node coder
graph for each row and starts it. The shipped `graphs/spec.toml` (`code`, `verify`, the pinned `docs` call, `reverify`, `done`) has now run end to end with the real
claude through the production daemon (COMPOSITION-REAL-RUN.md, COMPOSITION-REAL-FAULTS.md), but no queue row has ever used it. This spec adds the opt-in; it does not
change the default.

## Requirements
### Functional
- R0: `run-batchC-pii.sh <N> --dry-run` writes the brief and the graph or the input, prints `AGENT`, `MODEL`, `PROVIDER`, `SPEC`, `LANE-GRAPH`, and exits 0 BEFORE it touches the daemon, ssh or any agent. Without the flag nothing changes.
- R1: `PII_LANE_GRAPH=spec` makes `run-batchC-pii.sh` start the tracked `spec` graph with `pii run start spec --input <json>` instead of writing a one-node graph. The input is
  built by `uv run` (no bare python3): `agent`, `model`, `provider` (pi only), `spec` (the `specs/<N>-*.md` path), `base` (`git rev-parse HEAD` of the repo), `workdir`
  (the repo path as the container sees it), `brief` (the brief file's text), `docs_brief` (a fixed text that names the one file the docs step may write: `NOTES-<N>.md` in the repo root), `max_budget_usd` (from `PII_MAX_BUDGET_USD`, default `1.00`, never above the graph's own `[limits]`).
- R2: unset or any other value of `PII_LANE_GRAPH` keeps today's one-node graph byte for byte.
- R3: a resume (`--resume <sid>`) with `PII_LANE_GRAPH=spec` falls back to the one-node graph and prints `LANE-GRAPH: coder (resume is not supported by the spec graph)`, because
  `graphs/spec.toml` has no session input. The line `LANE-GRAPH: spec|coder` is printed before anything is spent, in both cases.
- R4: the script copies `graphs/spec.toml` and `graphs/docs.toml` into the daemon's graph dir (`_tmp/devbox-test/graphs/`) and refuses (exit 2, nothing started) when the pin in `spec.toml` is not the sha256 of `docs.toml`.
- R5: the facts the cycle reads (`FACTS rc= tree_changed= quota_exhausted= cost_usd=`) come from the host `code` coder's `coder.exited`, the FIRST one of the run; a second line `DOCS-FACTS ...` carries the docs coder's.

### Non-Functional
- The script stays one file; if it passes 450 effective lines, move the input builder to `scripts/lane-input.sh`. No hardcoded path beyond what the script already has.

## Verified Premises
- **NOT PRESENT** (measured 2026-10-27, `grep -n -- '--dry-run\|DRY' scripts/run-batchC-pii.sh` found nothing): the script has no dry-run, so R0 adds it and every test below relies on it.
- **CONFIRMED** `scripts/run-batchC-pii.sh:88-132` writes the one-node graph; `:152` starts it by name; `:180` reads `rc`, `tree_changed`, `quota_exhausted` from the LAST `coder.exited`.
- **CONFIRMED** `scripts/archive/composed-real-run.sh` already builds the spec graph's input with `uv run` and passes `max_budget_usd`; it is the pattern to follow.
- **CONFIRMED** `graphs/spec.toml` declares `needs` for `docs_brief` and has `[limits] max_micro_usd = 1_000_000` and `max_wall_ms = 6000000`.
- **CONFIRMED** the real composed run reads `FACTS` from the journal by the lines added since the start (`LINES_BEFORE`); the lane does the same.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| `default_is_the_one_node_graph` | no `PII_LANE_GRAPH`, `--dry-run` | prints `LANE-GRAPH: coder`, the generated graph text is byte-identical to a saved copy |
| `spec_graph_is_selected` | `PII_LANE_GRAPH=spec`, `--dry-run` | prints `LANE-GRAPH: spec` and an input with all nine keys |
| `resume_falls_back` | `PII_LANE_GRAPH=spec --resume abc` | prints the fallback line and `LANE-GRAPH: coder` |
| `CONTROL_a_changed_pin_is_refused` | `docs.toml` with one byte changed | exit 2, nothing started |
| `CONTROL_cap_never_exceeds_the_limit` | `PII_MAX_BUDGET_USD=9` | the input carries `max_budget_usd` of 1.00 |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | grep -c '^ok')" -ge 5`
- [ ] `bash scripts/autonomy/tests/lane-graph-wall.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash -n scripts/run-batchC-pii.sh`
- [ ] `grep -q 'PII_LANE_GRAPH' scripts/run-batchC-pii.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/run-batchC-pii.sh`
- `scripts/autonomy/tests/lane-graph-select.test.sh`

## Guardrails
- Do NOT change the default graph or `PII_WORKER`'s default. Do NOT start a daemon, spawn an agent or spend in a test (`--dry-run` only). Do NOT edit an existing assertion.
- Mutations to kill: the default changes, the resume branch is dropped, the pin check is skipped, the cap exceeds the limit, FACTS read the docs coder.

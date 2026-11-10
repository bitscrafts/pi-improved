# Spec 295: the queue's lane graph is a runtime knob the cron tick can read

## Overview
**Project**: <repo>
**Phase**: production — composability, the switch. `run-batchC-pii.sh` starts the composed `spec` graph when `PII_LANE_GRAPH=spec` (spec 294), but the loop's cycles run from a
cron tick, and `scripts/autonomy/env.sh` states that "the loop's environment is set here and not inherited". So nothing a person types can reach a cycle, and the only way to
switch the queue to the composed graph would be to edit a script. This spec adds the knob the loop already uses for its other controls: a one-word file.

## Requirements
### Functional
- R1: `scripts/autonomy/env.sh` reads the first line of the file named by `PII_LANE_FILE` (default `_tmp/autonomy/lane-graph`) and, when the line (spaces and CR/LF stripped) is exactly `spec` or `coder`, exports `PII_LANE_GRAPH` with it.
- R2: an absent file, an unreadable file, an empty file or any other word leaves `PII_LANE_GRAPH` exactly as it was (unset stays unset). Nothing is printed.
- R3: a `PII_LANE_GRAPH` already set in the environment wins over the file.
- R4: the file's one-word contract is written in a comment above the code in `env.sh`, with the date and this spec's number.

### Non-Functional
- Pure shell, no new dependency, no `eval`, no command substitution of file content. `env.sh` stays under 450 effective lines.

## Verified Premises
- **CONFIRMED** (read 2026-10-27) `env.sh:1-3` says the loop's environment is set there and not inherited, and `cycle.sh:31` sources it.
- **CONFIRMED** `cycle.sh:244-245` runs `scripts/run-batchC-pii.sh` in a subshell, so an exported variable reaches it.
- **CONFIRMED** `run-batchC-pii.sh` reads `PII_LANE_GRAPH` and only the value `spec` selects the composed graph (spec 294).

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| `file_spec_exports_spec` | file holds `spec` | `PII_LANE_GRAPH=spec` after sourcing |
| `file_coder_exports_coder` | file holds `coder` | `coder` |
| `trailing_spaces_and_crlf_are_stripped` | file holds `spec  ` and a CRLF | `spec` |
| `CONTROL_absent_file_leaves_it_unset` | no file | unset |
| `CONTROL_garbage_word_leaves_it_unset` | file holds `rm -rf /` | unset and nothing executed |
| `CONTROL_env_wins_over_file` | env `coder`, file `spec` | `coder` |
| `CONTROL_empty_file_leaves_it_unset` | empty file | unset |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/lane-graph-knob.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(bash scripts/autonomy/tests/lane-graph-knob.test.sh 2>&1 | grep -c '^ok')" -ge 7`
- [ ] `bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash -n scripts/autonomy/env.sh`
- [ ] `grep -q 'PII_LANE_FILE' scripts/autonomy/env.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/autonomy/env.sh`
- `scripts/autonomy/tests/lane-graph-knob.test.sh`

## Guardrails
- Do NOT change the default (no file means today's behaviour). Do NOT edit `run-batchC-pii.sh`, `cycle.sh` or any existing test. Do NOT spawn an agent or start a daemon in a test.
- Mutations to kill: the file is ignored, a garbage word is exported, the file overrides the environment, the CR is not stripped, an absent file prints an error.

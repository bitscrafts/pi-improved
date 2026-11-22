# Spec: a test that exercises a graph edit owns its own git HEAD

## Overview

Measured 2026-11-22: a correct, linted change to `graphs/sdd.toml` could not be certified. Three
assertions in `scripts/autonomy/tests/lane-graph-select.test.sh` run a *real* lane start, and
`scripts/run-batchC-pii.sh` refuses a start when `graphs/spec.toml` differs from the git HEAD of the
repository it runs in:

    ABORT: graphs/spec.toml differs from HEAD (the pin protects the reviewed graph): nothing started

That guard is right — a lane must not start on a graph nobody reviewed. The defect is on the other
side: the test runs against a tree whose HEAD it does not own, so the only way to make the test pass
is to commit the change first, and the only way to commit is a green gate. A closed circle.

The opt-out that exists (`PII_GRAPH_EDIT_OK=1` with `--dry-run`, spec 303 R5) covers dry runs only,
precisely so a real start keeps comparing with HEAD. That is the correct rule and this spec does not
weaken it. The test must instead give the lane a HEAD of its own.

**Project**: /projects/pi-improved
**Phase**: 0.2.1

## Requirements

### Functional

- **R1** `scripts/autonomy/tests/lane-graph-select.test.sh` builds its scratch repository into a git
  repository of its own: `git init`, an initial commit of the copied tree, and then a commit of the
  graph files it wants the lane to run. The lane is pointed at that repository, not at the tree the
  test happens to be running in.
- **R2** The real-start assertions keep running a *real* start (no `PII_GRAPH_EDIT_OK`, no dry run):
  the property under test is what a real lane does with a clean HEAD.
- **R3** A new check, `scripts/checks/graph-edits-are-testable.sh`, fails naming any file under
  `scripts/autonomy/tests/` that runs a real lane start (`run-batchC-pii.sh`, `run.sh`, or
  `drain.sh` with a start) without initialising its own git repository first. It must fail when it
  inspects nothing — a check that finds no tests and passes is the false green this repo names.
- **R4** The guard in `run-batchC-pii.sh` is not touched. This spec is about the test owning its
  HEAD, never about the lane trusting a dirty tree.

### Non-Functional

- The scratch repository lives under `_tmp/` and is removed on exit by the test's existing `trash`.
- No test gains network access and no test spends money: the assertions are about the guard and the
  input, not about a model call.

## TDD Contract

| Test | Setup | Expected |
|---|---|---|
| a real start runs with the graphs committed in the scratch repo | scratch repo with the graphs committed | exit 0, and the lane's graph is the one under test |
| the same start refuses when the scratch HEAD does not carry the graph | one graph edited after the commit | exit 9 and the ABORT line |
| the check finds a test that starts the lane without a git repo | synthetic snippet in a temp dir | the check exits non-zero and names the file |
| the check inspects nothing | an empty directory | the check exits non-zero |

## Exit Criteria

- [ ] `grep -q 'git init' scripts/autonomy/tests/lane-graph-select.test.sh`
- [ ] `bash scripts/checks/graph-edits-are-testable.sh`

## Files

- `scripts/autonomy/tests/lane-graph-select.test.sh`
- `scripts/checks/graph-edits-are-testable.sh`

### New in this row

- [ ] `grep -q 'git init' scripts/autonomy/tests/lane-graph-select.test.sh`
- [ ] `bash scripts/checks/graph-edits-are-testable.sh`

## Guardrails

- Do NOT let a real start run against a tree whose HEAD the test does not own.
- Do NOT weaken the `run-batchC-pii.sh` guard, and do NOT extend `PII_GRAPH_EDIT_OK` to a real start.
- Do NOT satisfy the criteria by committing inside the real repository; the commit belongs to the
  scratch copy.
- A check that inspects nothing must fail, not pass.
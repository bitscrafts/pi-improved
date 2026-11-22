# Spec: every test that drives the real shell owns the HEAD it runs against

## Overview

Measured 2026-11-22, hours after spec 441 fixed this for the shell tests. `lane_run_parity`'s
`the_recorded_ssh_sequence_equals_the_shells` runs the real `run-batchC-pii.sh` against this checkout.
With a graph edited in the working tree the shell aborts before its first remote call, and the test
fails with

    the shell made no remote call

which reads as a defect in the recording or in the shell. It is neither: the shell did what it is
supposed to do, and the test never gave it a HEAD of its own.

Spec 441 fixed the same circle for `scripts/autonomy/tests/*.test.sh` and added a check that keeps it
fixed there. This row says the rule is older and wider than that directory: **any test that drives the
real shell on graphs that are being edited must own the HEAD it runs against, shell or Rust.**

Four shell tests were left in the check as `KNOWN-LATENT` by spec 441 — lane-facts, lane-preflight,
lane-resume, lane-wait-cancels. They are the same defect, recorded rather than fixed. A suppression
list that never shrinks is the blindness the previous row removed.

**Project**: /projects/pi-improved
**Phase**: 0.2.1

## Requirements

### Functional

- **R1** `crates/pii-cli/tests/lane_run_parity.rs` gives the shell a repository it owns: a scratch
  copy under `_tmp/` initialised as a git repository, with the graphs committed in it, and the shell
  driven against that copy. The guard in `run-batchC-pii.sh` is not touched.
- **R2** `scripts/checks/graph-edits-are-testable.sh` scans `crates/*/tests/*.rs` as well as
  `scripts/autonomy/tests/*.test.sh`, applying the same rule: a real shell start with no repository of
  its own is an offence.
- **R3** Each of the four `KNOWN-LATENT` files either gains its own HEAD or carries a one-line reason
  in the check explaining why it cannot — and the check prints the count, so the list is visible in
  every run.
- **R4** The check still fails when it inspects nothing, in both its directories.

### Non-Functional

- No test gains network access: the shell is driven with the same stub environment the parity test
  already uses.
- The scratch repository is removed on exit.

## TDD Contract

| Test | Setup | Expected |
|---|---|---|
| a real shell start with a dirty graph, test owning its HEAD | scratch repo, graphs committed, one graph edited after | the shell runs and the test asserts on its real behaviour |
| the check sees a Rust test that starts the shell | synthetic `*.rs` in a temp dir, no `git init` | non-zero, naming the file |
| the check sees a Rust test that does own its HEAD | same with `git init` | zero |
| the check inspects no Rust test and no shell test | empty dirs | non-zero |

## Exit Criteria

- [ ] `grep -q 'git init' crates/pii-cli/tests/lane_run_parity.rs`
- [ ] `grep -q 'crates' scripts/checks/graph-edits-are-testable.sh`

## Files

- `crates/pii-cli/tests/lane_run_parity.rs`
- `scripts/checks/graph-edits-are-testable.sh`

### New in this row

- [ ] `grep -q 'git init' crates/pii-cli/tests/lane_run_parity.rs`
- [ ] `grep -q 'crates' scripts/checks/graph-edits-are-testable.sh`

## Guardrails

- Do NOT weaken `run-batchC-pii.sh`, and do NOT extend the dry-run opt-out to a real start.
- Do NOT delete the four `KNOWN-LATENT` entries to make the count look clean: each must be fixed or
  justified in a comment.
- Do NOT satisfy criterion 1 by adding a comment containing `git init`: the string must be a command
  the test runs.
- A check that inspects nothing must fail in each directory it scans.
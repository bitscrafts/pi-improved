# Spec: the operator's records are not test fixtures

## Overview

The Opus 061 judgement measured it (C-read): the loop's selftest writes into the **real** `questions.md`,
`notes.md` and `state.json`, and one of its assertions reads the real control file instead of `$PII_CONTROL`. It
also recorded four stray notes in `_tmp/autonomy/notes.md` (15:07, 15:08, 15:10, 15:13) left by test runs. Any
operator-distance metric built on those records would count the tests' own noise as operator acts — so this comes
**before** the metric.

**Project**: <repo>
**Phase**: proactivity (the Opus 061 judgement; order 153 → 150 → 154 → 151 → 152)

## Requirements

### Functional

R1. The paths for `questions.md`, `notes.md` and the state file become overridable the way `PII_CONTROL` already is
    (`PII_QUESTIONS`, `PII_NOTES`, `PII_STATE`), with today's values as defaults so no existing caller changes.
R2. The selftest exports all of them, and its own assertion reads `$PII_CONTROL` rather than the real file.
R3. Every operator act — `pause`, `resume`, `answer`, `skip`, `park`, `unskip` — appends one timestamped line to
    `operator-acts.log` (who: `human`; what; which item). This is the record the distance metric counts.

### Non-Functional

R4. No behaviour change when the variables are unset: the same files, the same messages.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| the_selftest_leaves_the_real_records_untouched | hashes of control, questions, notes and state before and after a selftest run | unchanged |
| an_operator_act_is_recorded | `control.sh pause x`, `answer 1 y`, `resume` | three lines in `operator-acts.log`, timestamped, naming the act |
| the_defaults_are_unchanged | nothing exported | the same paths as today |

## Exit Criteria

- [ ] `bash scripts/checks/operator-records-untouched.sh` prints `UNTOUCHED` and exits 0 (the harness this spec adds:
      it hashes the four real files, runs the selftest, and re-hashes)
- [ ] `bash scripts/autonomy/selftest.sh` still ends `VERDICT: ALL MECHANISMS PASS` (the isolation must not break a case)
- [ ] `cargo xtask fresh-gate` on this spec's commit is `ALL PASS`

## Guardrails

- Do NOT delete the stray notes a test already wrote; leave them and say so in the closure.
- Do NOT change the meaning of PAUSED (it must still stop the loop without killing a worker mid-flight).
- Do NOT make an operator act fail if the log cannot be written; report it and carry on.

## Closure (2026-11-01)

The implementation landed with the loop (`91dc682` and neighbours: `PII_QUESTIONS`/`PII_NOTES` in
`control.sh`, the selftest exporting its own copies, `operator-acts.log` written by every act), but the spec was
never closed, so nobody had run its proof. It was run today, against the deployed tree:

```
scripts/checks/operator-records-untouched.sh   ->  UNTOUCHED   rc=0
   (hashes control, questions.md, notes.md, state.json, operator-acts.log, last-reported; runs the whole
    selftest; re-hashes — every hash held)
scripts/autonomy/selftest.sh                   ->  VERDICT: ALL MECHANISMS PASS
operator-acts.log                              ->  181 lines, one per act, `ts | human | act | item`
```

Why it mattered and why it is first in the order: the distance metric in the next step counts operator acts, and
a metric fed by test runs measures the tests. `operator-acts.log` is now the record that metric reads, and the
harness is what keeps a selftest from writing into it.

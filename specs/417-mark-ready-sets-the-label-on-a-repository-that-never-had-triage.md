# Spec: mark-ready sets the label on a repository that never had triage, and says so only after reading it back
#: depends=413 status=pending block=-

## Overview
Measured 2026-11-12 while running the first end-to-end test: `scripts/mark-ready.sh 1` failed with
`failed to update https://github.com/bitscrafts/pi-improved/issues/1: 'triage' not found`, and `ready` did not exist on the repository at all — it had to be
created by hand before the channel could be tested. The script does one `gh issue edit --remove-label triage --add-label ready`, and GitHub rejects the whole
call when any label it names is missing, so a repository without `triage` (every repository on its first issue) cannot be marked ready at all. It also prints
`ready: #N` without reading the labels back, so a call that silently changed nothing would still look like success.

The script is the owner's own hand on the trust gate (spec 406: only a `ready` label set by the owner starts a run). It has no test today and cannot have one:
it talks to the network in every path.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 413

## Requirements
- R1 `ready` is created on the repository when the repository does not have it, before any issue is edited: name `ready`, colour `0e8a16`, description
  `Specified well enough to start; only the owner sets it`. An existing `ready` is never re-created or altered.
- R2 A label is removed only when the issue actually carries it, in its own `gh issue edit --remove-label <name>` call: `triage` is removed when present and
  skipped when absent. A label that cannot be removed never prevents `ready` from being added.
- R3 After the edit, the script reads the issue's labels back (`gh issue view --json labels`) and exits non-zero unless `ready` is present and every label it
  meant to remove is gone. The success line is printed only after that read agrees. A read-back that disagrees names what it found.
- R4 The two refusals are unchanged and set no label: an issue that is not `OPEN` exits 3, and an issue the repository owner did not write exits 4 unless
  `--owner-approved` is given.
- R5 Every `gh` call is one a fake can serve offline: `repo view --json nameWithOwner`, `label list --json name`, `label create`, `issue view --json
  author,state,labels`, `issue edit --add-label`, `issue edit --remove-label`. The script reads no other field.

## Architecture
```
mark-ready.sh <N> [--owner-approved] [owner/repo]
  author, state            -> refuse (3 closed, 4 not the owner's)
  label list               -> ready missing? -> label create ready
  issue labels             -> carries triage? -> issue edit --remove-label triage
  issue edit --add-label ready
  issue view --json labels -> read back: ready present? removals gone? -> print, or exit 5
```
Key decision: the test drives a fake `gh` on `PATH`, so the script's own argv and its read-back are what the test observes — not a re-enactment of it.
Rejected: testing against the real repository (the test would need the network and would mutate a public project).

## TDD Contract
New check `scripts/checks/mark-ready-sets-the-label.sh`, registered as a gate step, driven by a fake `gh` that records every argv and holds a tiny state
(which labels exist on the repository, which on the issue).
| Test Name | Input | Expected Output |
|---|---|---|
| `a_repository_without_triage_gets_ready` | no `ready`, no `triage`; issue open, author = owner | exit 0; `label create ready` called once; `ready` added; no `--remove-label` call; success line printed |
| `triage_is_removed_in_its_own_call` | issue carries `triage` | exit 0; exactly one `issue edit --remove-label triage`; `ready` present |
| `CONTROL_a_strangers_issue_sets_no_label` | author ≠ owner, no `--owner-approved` | exit 4; the log has no `label create` and no `issue edit` |
| `CONTROL_a_closed_issue_sets_no_label` | state `CLOSED` | exit 3; no `label create`, no `issue edit` |
| `CONTROL_a_read_back_that_disagrees_fails` | the fake accepts `--add-label` but the read-back shows no `ready` | exit 5; the success line is NOT printed |
| `CONTROL_an_issue_that_keeps_triage_fails` | the fake accepts `--remove-label` but the read-back still shows `triage` | non-zero; the success line is NOT printed |

Acceptance: each row exits as stated and the check prints one `PASS:`/`FAIL:` line per row plus a final count. Negative controls are mandatory: a rule that
skips a call needs a case where the call must happen (`triage_is_removed_in_its_own_call`), and a read-back needs a case where it disagrees.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `bash scripts/checks/mark-ready-sets-the-label.sh`

## Guardrails
- Keep green: `bash scripts/autonomy/run-tests.sh` and `bash scripts/autonomy/sync-verify.sh --check`.
- The script must still refuse a stranger's issue and a closed one, and must not touch any label in those paths.
- Do NOT call the real GitHub in the test; the fake `gh` is the only `gh` the test may reach, and it must be first on `PATH`.
- Do NOT change `scripts/autonomy/plan.tsv` or any spec other than this one.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `scripts/mark-ready.sh`
- `scripts/checks/mark-ready-sets-the-label.sh`
- `gate-steps.txt`

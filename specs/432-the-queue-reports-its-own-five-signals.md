# Spec: the queue reports its own five signals
#: depends=431 status=pending block=-

## Overview
Research 084 records five deterministic signals that found 32 real defects in a foreign job table, after two thirds of
its first run were false positives on healthy jobs. pii already stores everything the five need — `state.json`,
`plan.tsv`, the commits, the per-row reports and `red-audit` — so this costs no new telemetry.

The measured state that motivates it (2026-11-15): 38 rows open, the oldest 66 days, with no transitions; rows marked
`done` whose issue was never closed; a gate reported as passing in under a second.

## Requirements
R1. `scripts/autonomy/signals.sh` prints each finding as one line: `signal<TAB>row<TAB>evidence-path<TAB>detail`.
R2. The five signals, defined against pii's own data only:
    - `scheduled-but-failing` — row `pending` with `attempts >= 3`.
    - `delivered-nowhere` — row `done` whose issue (when the card names one) is still open, or with no report.
    - `completed-in-no-time` — a `done` row whose gate log shows PASS in under 60 s.
    - `completed-with-no-record` — row `done` with no `commit` field.
    - `incident-never-closed` — `needs-review` older than 14 days.
R3. Every finding carries an evidence path that **exists**; a finding whose evidence does not resolve is not printed
    (research 084 §4: the sweep's first run was two thirds noise, and the lesson was to restrict signals rather than
    trust them).
R4. Lifecycle: findings are tracked as `NEW` / `RECURRING` / `RESOLVED` in `_tmp/autonomy/signals.tsv`, keyed on
    `(signal, row)` — never on who caused it. `RESOLVED` after 14 days unseen.
R5. Healthy rows produce no finding, and that is tested: the control cases (a `done` row with a commit and a closed
    issue, a `pending` row on its first attempt) must be silent.
R6. The report is read-only: it never mutates `state.json`, never touches the tree, never closes anything.

## TDD Contract
| Test Name | Input | Expected Output |
| --- | --- | --- |
| pending_with_three_attempts_is_flagged | row pending, attempts 3 | `scheduled-but-failing` |
| control_pending_on_first_attempt_is_silent | row pending, attempts 0 | nothing |
| done_without_commit_is_flagged | row done, no commit | `completed-with-no-record` |
| control_done_with_commit_and_closed_issue_is_silent | normal landed row | nothing |
| done_with_an_open_issue_is_flagged | row done, issue open | `delivered-nowhere` |
| a_finding_without_resolving_evidence_is_not_printed | missing log path | nothing |
| a_finding_seen_before_is_recurring | same finding twice | `RECURRING` |
| a_finding_unseen_for_fourteen_days_is_resolved | last seen 15 days ago | `RESOLVED` |

## Exit Criteria
- [ ] `bash scripts/checks/signals-report.sh`
- [ ] `bash scripts/autonomy/signals.sh | head -20`

## Files
- `scripts/autonomy/signals.sh`
- `scripts/checks/signals-report.sh`
- `gate-steps.txt`

## Notes
This is the cheap half of research 084: the five signals and the lifecycle, no model. Spec 431 is the expensive half.
Deliberately not included: a cron job. Adding a job to a system whose open-rows list is already full of stale entries is
how the noise grows (research 084 §5).

## Guardrails
- Do NOT flag a `no_agent`/script lane on speed alone: a fast exit is that lane's healthy result.
- Do NOT mutate state, close issues or touch the tree.
- Do NOT print a finding without an evidence path that exists.

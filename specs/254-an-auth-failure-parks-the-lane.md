# Spec 254: an authentication failure parks the lane, it does not burn the row's attempts

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the queue's worker is pii (user directive 2026-10-24)

The queue gives a row three attempts, then parks it. An attempt that dies on a dead login (HTTP 401, "OAuth token revoked", "Please log in again") is
counted like an attempt that failed on its work, so three auth failures park a row that was never tried, and the next row hits the same wall. Quota already
has the right treatment in `cycle.sh` (the reset time is read from the worker's own log and the row waits, spec: window.sh). Authentication is the same
shape: the row is fine, the LANE is broken, and a person has to act.

## Requirements
### Functional
- **R1.** New script `scripts/autonomy/classify-failure.sh`: reads a worker log on stdin, prints exactly one word on stdout and exits 0:
  `auth` when the text contains any of `OAuth token revoked`, `Failed to authenticate`, `Please log in again`, `api_error_status":401` (also `api_error_status\":401`), or the journal's own field `claude_api_error_status":"401"` (spec 253 writes the status as a string and, when claude gave no text, writes only that);
  `quota` when it contains `spend limit`, `usage limit` or `hit your` + `limit` (case-insensitive) and no auth marker; otherwise `work`.
  Auth wins over quota when both appear. Empty input is `work`.
- **R2.** `scripts/autonomy/cycle.sh`, in the branch that handles a worker with a non-zero exit, runs the worker's log through `classify-failure.sh` BEFORE the attempt
  counter moves. On `auth` it: sets the row `status=needs-review last=lane-auth-failed attempts=<unchanged>`, writes the line `PAUSED lane-auth-failed <UTC time>` to the
  queue's control file via `scripts/autonomy/control.sh` (the queue stops dispatching, nothing is retried), prints `OUTCOME=lane-auth-failed ITEM=<id>`, and exits non-zero.
  Quota and work handling are unchanged.
- **R3.** `scripts/autonomy/selftest.sh` is NOT edited.

### Non-Functional
- Pure text processing, no network, no credential read. Python only through uv (this spec needs none).

## Architecture
```
worker log --stdin--> classify-failure.sh --> auth | quota | work
cycle.sh (worker rc != 0) --> classify --auth--> row needs-review (attempts kept) + control PAUSED + OUTCOME=lane-auth-failed
                                         |--quota/work--> the existing branches, unchanged
```

## TDD Contract
New file `scripts/autonomy/tests/classify-failure.test.sh` (create the directory), exits 0 only if every case holds; each case prints `ok <name>` or `FAIL <name>`:

| Case | stdin | Expected word |
|---|---|---|
| `auth_revoked` | `Failed to authenticate: OAuth token revoked. Please log in again` | `auth` |
| `auth_status_401` | `{"is_error":true,"api_error_status":401}` | `auth` |
| `quota_spend` | `You've hit your monthly spend limit. Resets 4:00pm` | `quota` |
| `auth_beats_quota` | both an auth marker and `spend limit` | `auth` |
| `plain_work_failure` | `error[E0063]: missing field` | `work` |
| `empty_is_work` | (nothing) | `work` |
| `auth_journal_status_only` | `{"claude_api_error_status":"401","claude_result":"","rc":"1"}` | `auth` |
| `NEAR-MISS: journal status that is not 401` | `{"claude_api_error_status":"500","claude_result":"","rc":"1"}` | `work` |
| `NEAR-MISS: 401 as a plain number` | `took 401 ms` | `work` (a bare number is not the status field) |
| `NEAR-MISS: the word limit alone` | `line limit of 450 reached` | `work` |

## Exit Criteria
- [ ] `test -x scripts/autonomy/classify-failure.sh && test -f scripts/autonomy/tests/classify-failure.test.sh`
- [ ] `bash scripts/autonomy/tests/classify-failure.test.sh | grep -c '^ok ' | grep -qx 10`
- [ ] `bash scripts/autonomy/tests/classify-failure.test.sh > /dev/null`
- [ ] `grep -q 'classify-failure.sh' scripts/autonomy/cycle.sh && grep -q 'lane-auth-failed' scripts/autonomy/cycle.sh`
- [ ] `bash -n scripts/autonomy/cycle.sh && bash -n scripts/autonomy/classify-failure.sh`
- [ ] `git diff --quiet -- scripts/autonomy/selftest.sh`

## Guardrails
- Do NOT change how `quota` or ordinary failures are handled, or the attempt cap of three.
- Do NOT edit `selftest.sh`, `window.sh`, or any Rust file.
- Do NOT print any credential or token value; the markers are fixed strings.
- Do NOT commit.

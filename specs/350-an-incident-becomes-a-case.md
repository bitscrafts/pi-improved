# Spec: An incident becomes a case without a person

## Overview

The loop closes today only because a person reads the incident list. `pii incidents scan` on the host reports what
is wrong; the container's cron posts those incidents to Discord; and then **somebody** has to decide that
`observability-lost:r000000004577` deserves a row. Everything downstream of that decision is automated — the
dispatcher, the worker, the judge, the gate, and now (349) the treatment that can travel.

This spec automates the decision's *paperwork*, not the decision: a NEW incident opens a **case** with its evidence
attached, deduplicated by fingerprint, and a case is not dispatchable until its spec exists. That keeps the rule that
made today expensive to learn: a row must carry a criterion that can fail, and an auto-generated spec cannot invent
one.

**Project**: /projects/pi-improved
**Phase**: self-improvement — the link between detection on the host and the workshop in the container.

## Verified premises (measured 2026-11-02)

- `pii incidents scan --source runtime --log <host log>` prints one `incident <fingerprint> …` line per finding, and
  fingerprints are stable (`refusal-repeated:channel.poll.failed`, `observability-lost:<run>`).
- The host runs the binary only: `~/.local/bin/pii`, no source tree and no Rust toolchain, so the host cannot build a
  fix. The container has both, which is why the workshop stays there.
- `heal_detect` (S0.2) runs an xtask gate and records failures — it needs a repository, so it belongs to the
  workshop, not the host.
- A row whose criteria cannot fail is worse than no row (AGENTS.md §0b; four instances on 2026-11-02), so an
  incident may not become a dispatchable row by itself.

## Requirements

### Functional

1. **`scripts/autonomy/incident-cases.sh`** reads the host's incidents and appends every NEW fingerprint to
   `_tmp/autonomy/incident-cases.tsv`, one line per case:
   `fingerprint · first_seen · count · source_host · state · case_id`.
   States: `new` → `briefed` (a spec exists) → `queued` (a row exists) → `closed` (a treatment landed).
2. **Deduplication by fingerprint**: a fingerprint already present is never appended twice; a closed case that
   recurs appends a NEW case with an incremented suffix (`<fp>#2`), because a cure that did not hold is its own
   finding.
3. **Silent when nothing is new.** The script prints one line per new case and nothing at all otherwise, so it can run
   from cron with `no_agent` and never becomes noise.
4. **A case is not a row.** Cases in state `new` are listed by `--list` and are NOT dispatchable: `cycle.sh` ignores
   them. Promotion (`--brief <case_id>`) writes a spec stub at `specs/auto-<case_id>.md` containing the evidence
   (fingerprint, first_seen, count, the log lines that produced it) and a `## TODO criteria` section, then sets the
   state to `briefed`. Writing the criteria is the work, and a person or the skill lane does it.
5. **The cap**: at most `--max-new` (default 5) cases per run, oldest first, so one log full of old damage cannot
   flood the queue with a year of history.
6. **`--dry-run`** prints what would be appended and writes nothing.

### Non-Functional

- Bash only: this is loop infrastructure, not the product. The user's rule is that product logic lives in the binary;
  the loop's bookkeeping is a script and must stay one.
- The file is append-only and human-readable (TSV): a case ledger somebody can read with `column -t`.
- No network beyond the existing ssh to the host, and no writes on the host.

## TDD Contract

| Test (`scripts/checks/tests/incident-cases.test.sh`) | Assertion |
|---|---|
| `a_new_incident_becomes_a_new_case` | two incidents, empty ledger → two `new` lines with their evidence |
| `CONTROL_the_same_run_appends_nothing` | run twice over the same input → the second run prints nothing |
| `a_recurrence_after_close_is_its_own_case` | a `closed` fingerprint appearing again → `<fp>#2`, state `new` |
| `a_case_is_not_dispatchable` | after the run, `cycle.sh`'s pick ignores every case (they are not rows) |
| `CONTROL_briefing_a_case_writes_a_spec_and_flips_the_state` | `--brief` → spec file exists, contains the evidence, state `briefed` |
| `the_cap_keeps_a_backlog_from_flooding` | 9 new incidents, `--max-new 2` → 2 appended, the rest reported as deferred |
| `dry_run_writes_nothing` | rc 0, ledger byte-identical |

## Exit Criteria

- [ ] `bash scripts/checks/tests/incident-cases.test.sh`
- [ ] `bash scripts/autonomy/incident-cases.sh --dry-run`
- [ ] `bash scripts/checks/gates.sh`

## Guardrails

- Do NOT create a dispatchable row from an incident: a row needs criteria that can fail, and a machine cannot invent
  those from a fingerprint.
- Do NOT append a duplicate fingerprint.
- Do NOT write anything on the host.
- Do NOT let a backlog become a flood: the cap is what keeps the queue meaningful.
- Do NOT make the cron loud: silent when there is nothing new.

## Files

- `scripts/autonomy/incident-cases.sh`
- `scripts/checks/tests/incident-cases.test.sh`
- `_docs/05-operations/self-improvement-and-the-transport.md` (the case states join the ladder there)

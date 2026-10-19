# Spec: the starts and the hard deaths are counted

## Overview

070 §5 names the instrument that makes this system legible about itself: every run record carries
`{revision, process_start_ms, created_ms}`, and `pii health` reads a `since_deploy` block **from the store**,
keyed by the running revision. Spec 188 built the attribution and 190 added `posted`/`post_refused`. Two numbers
of that block are still missing — `starts` and `hard_deaths` — and without them the store cannot answer the
question that matters after a crash: *how many processes wrote into this revision, and which of them left a run
in flight and never came back?*

Measured 2026-10-18 (070 §0.1/§5): `c0818bf` reads `runs 0 / armed 4`, and the store held 1,846 runs of `?`
written by 10 processes — a fact recovered from run-id block arithmetic, not from any field. A crash-looping
supervisor is exactly the shape those two numbers must expose: several `starts`, `hard_deaths` > 0, and runs
still marked `running` whose process start no longer exists.

**Project**: /projects/pi-improved

## Requirements

1. `SinceDeploy` gains `starts`: the number of DISTINCT `process_start_ms` values among the run headers that
   name this revision — one per process that actually wrote a run. A revision with no runs reads `0`, never `1`:
   the number is evidence from the store, not a belief about the running process.
2. `SinceDeploy` gains `hard_deaths`: the runs of this revision that are still `running` whose
   `process_start_ms` is strictly OLDER than the newest `process_start_ms` in this revision's headers. A run left
   `running` by the newest process is NOT a hard death (it is the current process's work); a `suspended` run is
   armed, not dead.
3. Both numbers come from the run headers already in the store, in the same pass that counts `runs`. No new
   table, no process-local counter, and nothing that resets when the process restarts.
4. `pii health` shows both keys in its `since_deploy` block (the block is rendered from the status JSON, so a
   field the struct carries is a field the operator sees).

## TDD Contract

- RED first: `crates/pii-server/tests/since_deploy_starts.rs` must fail to compile before the fields exist
  (they are struct fields of the public `SinceDeploy`), then assert:
  1. two process starts, the older one leaving a `running` run → `starts == 2`, `hard_deaths == 1`;
  2. control: the newest process's own `running` run is not a hard death (`hard_deaths` stays 1 with it present);
  3. control: a run of another revision, and a header with no revision, move neither number.
- `crates/pii-cli/src/cmd_health.rs`'s tests gain `since_deploy_prints_starts_and_hard_deaths`: `render` of a
  status JSON carrying `{"starts": 3, "hard_deaths": 1}` prints both keys with their numbers.

## Exit Criteria

- [ ] `cargo test -p pii-server --test since_deploy_starts 2>&1 | grep -q 'test result: ok. 3 passed'`
- [ ] `cargo test -p pii-cli --bins since_deploy_prints_starts_and_hard_deaths 2>&1 | grep -q 'test result: ok. 1 passed'`

## Guardrails

- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT derive `starts` from a process-local counter or from `std::process::id()`: it must be read from the
  store, so it survives the restart it is meant to expose.
- Do NOT count a `suspended` run as a hard death, and do NOT count the current process's own `running` run as
  one — both are controls above, and inverting either makes the number alarm on healthy work.
- Do NOT change what a run header contains; the fields are already there (spec 188).
- If the store cannot be read, `since_deploy` must fail as it does today rather than report zeros that look good.

## Closure

**Verified 2026-10-19 08:47Z by the orchestrator, independently of the worker's report, on the committed tree
(`121fb6d`).**

*The two criteria, run by hand:*

- `cargo test -p pii-server --test since_deploy_starts 2>&1 | grep -q 'test result: ok. 3 passed'` → rc=0
  (`ok. 3 passed; 0 failed` printed).
- `cargo test -p pii-cli --bins since_deploy_prints_starts_and_hard_deaths 2>&1 | grep -q 'test result: ok.
  1 passed'` → rc=0 (`ok. 1 passed; 0 failed … 55 filtered out` printed).

*The controls hold in the committed tests, not only in prose:* `an_older_process_leaving_a_running_run_is_
a_hard_death` (starts 2, hard_deaths 1), `the_newest_process_own_running_run_and_a_suspended_run_are_not_
hard_deaths` (a `running` run of the NEWEST start plus a `suspended` run of an older one both stay out —
hard_deaths remains 1 — so a healthy in-flight run and an armed digest can never read as deaths),
`other_revisions_and_unattributed_headers_move_neither_number`. The "no runs reads 0, never 1" requirement
is its own assertion (`assert_eq!(counts(&store, "abc123"), (0, 0))` before any seed).

*Live agreement measured 08:40Z against the production store.* The running revision reads `starts 3,
hard_deaths 0` with `runs 5` — the 3 starts are the 07:07Z deploy session's own processes (hard-deploy
restarts), 0 hard deaths is correct because nothing died mid-run, and the armed digest run is `suspended`,
excluded from deaths by the control above.

*What this closes, and what it does not.* The store can now answer "how many processes wrote this revision,
and which runs a dead one left in flight" from headers alone — the 1,850-runs-by-10-unidentifiable-processes
shape can never recur unattributed. It does NOT build the per-cron `runs_per_occurrence == 1` assertion
(070 §5 bullet 3, excluded from 188/190/192 by their scope boundaries) — that remains the open item.

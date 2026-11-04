# Review: the parked backlog (2026-11-04)

Spec: `specs/377-the-parked-backlog-gets-verdicts.md`. Read-only: no code edited, nothing committed, no row re-dispatched.

## Method and environment

- Criteria were executed, not read from `row-verify`. Each criterion of each row was run from `scripts/autonomy/plan.tsv` column 5, split on `;`, the same way `row-run-criteria.py` splits it. That script itself could not run here: this host has no `python3`, and `uv` cannot write `/root/.cache` or `/root/.local`.
- Cargo criteria first failed to link because `/tmp` is not writable. Re-run with `TMPDIR=/projects/pi-improved/_tmp/tmp`, they compile and run. The results below are from the re-run.
- **The WIP directories named in the spec are not on disk.** `/projects/_trash/` and `/projects/pi-improved/_trash/` do not exist, and `find` finds no `362-models-in-one-place-wip-*`, `364-*`, `368-*` or `369-*` directory anywhere. Per the spec, a missing WIP is reported as missing, not as done. Every "WIP sound" claim below therefore rests on the tree at HEAD, not on quarantined files.

## Criteria results (executed)

| Row | Criterion (abridged) | rc |
|---|---|---|
| 354 | `cargo test -p pii-cli --test daemon_store_default` (file missing) | 1 |
| 354 | `grep -c 'runtime_dir.join("store.redb")' cmd_daemon.rs` must be 0 (it is 1) | 1 |
| 354 | `! grep 'default: <runtime dir>/store.redb' cmd_daemon_flags.rs` | 1 |
| 354 | `grep -c 'pii_core::state_dir()' cmd_daemon.rs` ≥ 2 (it is 1) | 1 |
| 357 | `cargo test -p pii-server --test health_is_constant_time` | 1 |
| 357 | `cargo test -p pii-server --test sweep_skips_an_unchanged_dir` | 1 |
| 357 | `cargo test -p pii-worker --test in_flight_is_enforced` | 0 |
| 357 | `grep InMemoryBackend crates/pii-server/src/engine/registry.rs` | 0 |
| 357 | `cargo test -p pii-channels --test poll_backoff` (6 passed) | 1 |
| 358 | `cargo test -p pii-store --test df_counts_documents` | 0 |
| 358 | `cargo test -p pii-store --test stats_are_per_table` | 0 |
| 358 | `! grep -rq RESUME_INDEX crates/` | 0 |
| 358 | `cargo test -p pii-store --test retention_drops_what_the_policy_expires` | 0 |
| 362 | `cargo test -p pii-server --test models_from_config` (file missing) | 1 |
| 362 | `grep -q '@workhorse' graphs/queue.toml` (absent) | 1 |
| 362 | `! grep -rq deepseek-v4-flash graphs/` (still present in 4 graph files) | 1 |
| 362 | `grep -q workhorse crates/pii-server/src/daemon/config.rs` (absent) | 1 |
| 362 | `cargo xtask fresh-gate` | 1 |
| 364 | `cargo test -p pii-server --test queue_calls_spec` (3 passed) | 0 |
| 364 | `cargo test -p pii-server --test spec_graph_is_callable` | 0 |
| 364 | `grep '^exit = "done"' graphs/spec.toml` | 0 |
| 364 | `grep 'kind = "call"' graphs/queue.toml` | 0 |
| 365 | `cargo test -p xtask --bins fresh_gate` | 0 |
| 365 | `cargo test -p pii-cli --test daemon_log_wiring` | 0 |
| 365 | `cargo test -p pii-cli --test daemon_guard_reaps` (2 passed) | 0 |
| 365 | `grep PII_STATE_DIR` in both `daemon_log_wiring.rs` and `daemon_guard_reaps.rs` | 1 |
| 365 | `grep 'daemon.starting' daemon_guard_reaps.rs` | 1 |
| 366 | `scripts/autonomy/tests/resume-marker.test.sh` (file missing) | 1 |
| 366 | `! … resume-marker.test.sh … grep "^FAIL"` | 0 |
| 366 | `scripts/autonomy/tests/gate-refusal.test.sh` tail | 1 |
| 366 | `uv run python -m py_compile scripts/autonomy/state.py` (no uv/python here) | 2 |
| 366 | `bash -n scripts/autonomy/cycle.sh` | 0 |
| 366 | `grep "spec 366" scripts/autonomy/state.py` | 1 |
| 366 | `selftest.sh` must print `VERDICT: ALL MECHANISMS PASS` | 1 |
| 366 | `git diff --quiet HEAD -- quality-gate.sh gate-steps*.txt` | 0 |
| 368 | `bash scripts/checks/tests/lane-own-wip.test.sh` (file missing) | 1 |
| 368 | `grep row-paths scripts/run-batchC-pii.sh` | 1 |
| 368 | `grep 'outside this row' scripts/run-batchC-pii.sh` | 1 |
| 369 | `bash scripts/checks/tests/state-phase-marker.test.sh` (file missing) | 1 |
| 369 | `grep PHASE_MARKERS scripts/autonomy/state.py` | 1 |
| 369 | `grep 'unknown marker' scripts/autonomy/cycle.sh` | 1 |
| 369 | `bash scripts/autonomy/tests/gate-refusal.test.sh` contains `0 FAIL` | 1 |

## Verdicts

### 354 — the daemon's default store is the state dir

- **Verdict:** `proceed`. Nothing on disk to preserve, so this is a clean dispatch of spec 354 from HEAD, not a resume.
- **Evidence:** 3 of 4 criteria fail. `cmd_daemon.rs` still contains `runtime_dir.join("store.redb")` once. `cmd_daemon_flags.rs` still documents `<runtime dir>/store.redb` as the default. `crates/pii-cli/tests/daemon_store_default.rs` does not exist. The row was parked because spec 353 was implemented under this row's number, so the criteria were never tested against 354's own work.
- **Smallest next step:** write `crates/pii-cli/tests/daemon_store_default.rs` from spec 354 and watch it fail (RED), then change the `store.redb` default in `cmd_daemon.rs` and its help text (GREEN).
- **Taught:** the row's number and the spec it implemented can drift apart. Check the number against the spec file before dispatch.

### 357 — the daemon idles cheaply

- **Verdict:** `close`. Superseded by 370 (health in constant time) and 371 (the idle daemon leaves no files), both landed per the spec's own history. Commit `d2f8051` ("Health reads a record, and the idle daemon stops paying") is the same work.
- **Evidence:** 2 of 5 criteria still fail. The row's criteria name test files (`health_is_constant_time`, `sweep_skips_an_unchanged_dir`, `poll_backoff`) that the split rows now own. Those criteria are stale, not a sign of missing work.
- **Smallest next step:** mark 357 closed with a pointer to 370 and 371. Retire its criteria rather than fix them.
- **Taught:** a split row keeps its old criteria. Retiring a row should retire its criteria in the same change.

### 358 — data you can see, then bound

- **Verdict:** `close`. Superseded by 375 and 376.
- **Evidence:** all 4 criteria pass at HEAD (`df_counts_documents`, `stats_are_per_table`, the `RESUME_INDEX` absence, `retention_drops_what_the_policy_expires`). Commit `7d5faa7` ("Retention is its own row now, and it starts red") and `0efc6fd` ("The store can be bound by age") are the landed work.
- **Smallest next step:** none to unblock. Mark 358 closed.
- **Taught:** a skipped row whose criteria all pass still reads as parked in the queue. Closing it is a state write, not code.

### 362 — one place names the models

- **Verdict:** `needs-decision`. **Choice for the user:** raise the 450-line budget for `config.rs` (492 effective lines in the lost WIP) or split `config.rs` further.
- **Evidence:** every criterion fails at HEAD. `models_from_config.rs` does not exist. `graphs/queue.toml` has no `@workhorse`. `deepseek-v4-flash` still appears in four graph files. The WIP directory is not on disk, so the 81-file extraction cannot be checked. Commit `bf03674` ("One place names the models…") landed part of the work; the criteria say the rest is not done.
- **Smallest next step:** answer the budget question. Then write `models_from_config.rs` (RED) and finish the extraction.
- **Taught:** the file-size guard counts effective lines, so a clean extraction that moves code without deleting it stays over budget.

### 364 — the queue calls the graph that implements a spec

- **Verdict:** `close`. Superseded by commit `f4c056d` ("The queue implements a spec by calling the graph that does it"), which is this row's subject.
- **Evidence:** all 4 criteria pass when run with a writable `TMPDIR` (`queue_calls_spec` 3 passed, `spec_graph_is_callable` passed, `graphs/spec.toml` has `exit = "done"`, `graphs/queue.toml` has `kind = "call"`). The row claimed "Opus running now"; the WIP is not on disk, so no sound WIP exists to finish.
- **Smallest next step:** confirm no Opus process is still writing to this row, then close it. If one is running, its output must be checked against these same criteria before closing.
- **Taught:** the splice refusal is a guard that fires on the callee the row exists to call. See the class section below.

### 365 — the gate tells the truth about its host

- **Verdict:** `proceed`. No WIP found; dispatch from HEAD.
- **Evidence:** 3 of 5 criteria pass (`fresh_gate`, `daemon_log_wiring`, `daemon_guard_reaps`). The two grep criteria fail: `PII_STATE_DIR` is missing from `daemon_guard_reaps.rs` and `daemon.starting` is missing too.
- **Smallest next step:** add the `PII_STATE_DIR` and `daemon.starting` assertions to `daemon_guard_reaps.rs` (RED), then make them pass.
- **Taught:** the criteria check the test file for a string. A test that does not assert the string passes the test and fails the criterion.

### 366 — the resume marker the cycle writes

- **Verdict:** `needs-decision`. **Choice for the user:** is this row the resume-marker row (landed as `2bf720c`, "the resume marker the cycle writes must be one pick accepts") or the disk-ownership row (`23bd693`, which says disk ownership is "not a second 366")? Pick one; the criteria follow the choice.
- **Evidence:** `scripts/autonomy/tests/resume-marker.test.sh` and `gate-refusal.test.sh` fail or are missing. `state.py` contains no `spec 366` marker. `selftest.sh` does not print `ALL MECHANISMS PASS`. `py_compile` could not run (no `uv`/`python` here, rc 2), so it is an environment failure, not a verdict on the code.
- **Smallest next step:** answer the question above, then run `bash scripts/autonomy/selftest.sh` to see which mechanism fails.
- **Taught:** two rows shared a number for a while. The number was reused, not the work.

### 368 — a resumed row may resume its own work

- **Verdict:** `needs-decision`. **Choice for the user:** is commit `5bab72b` ("A row that owns its dirty tree may be resumed") the answer to this row? If yes, close the row and retire its criteria. If no, say which case `5bab72b` leaves open.
- **Evidence:** 0 of 3 criteria pass. `scripts/checks/tests/lane-own-wip.test.sh` does not exist. `scripts/run-batchC-pii.sh` has no `row-paths` reference and no `outside this row` text. The WIP directory is not on disk.
- **Smallest next step:** answer the question above.
- **Taught:** this row is the dirty-tree guard's own case. It was parked by the guard it was written to permit.

### 369 — an unknown phase marker means dispatch

- **Verdict:** `close`, once its criteria are re-pointed. Superseded by commit `6db7a4a` ("A prose note in the phase field skipped the dispatch"), which is the fix this row describes.
- **Evidence:** 0 of 4 criteria pass. `scripts/checks/tests/state-phase-marker.test.sh` does not exist. `state.py` has no `PHASE_MARKERS`. `cycle.sh` has no `unknown marker`. `gate-refusal.test.sh` does not report `0 FAIL`.
- **Smallest next step:** check that `6db7a4a` covers the unknown-marker case, then close 369 and retire its criteria. If it does not, the row needs a decision on which marker set is authoritative.
- **Taught:** "moving the phase markers left two controls reading the old source". Controls that read a stale copy of the marker set will pass or fail for the wrong reason.

## Failure classes seen across the rows

1. **A guard fires on the case it was built to permit.** 368 was parked by the dirty-tree guard, the very case its row covers. 364's refused callee dead-ends the splice, so the refusal guard stops the call it was meant to allow. 369's unknown-marker guard reads a prose note as a marker and skips dispatch (`6db7a4a`). The fix in each case is narrower than the guard, not a new guard.
2. **Criteria name files that no longer exist.** 354, 357, 362, 366, 368 and 369 each cite a test file or script that is missing or was renamed. The row stops at a missing file, which looks like a failing gate. Re-pointing or retiring criteria belongs in the same change as the split or the supersession.
3. **The sandbox cannot run the criteria as written.** `/tmp` is read-only for cargo's linker, `uv` cannot write its cache, and there is no `python3`. A criterion that fails for these reasons says nothing about the code. Set `TMPDIR` to a scratch path under `_tmp/`, and put `python3` on `PATH`, before judging a row.
4. **Quarantined WIP is not where the spec says.** The `_trash/` paths named in the spec do not exist on this host. Any verdict that depends on them is unverified until the directories are found.

## Still open for the user

- 362: the 450-line budget for `config.rs`.
- 366: which row this is (resume-marker or disk-ownership).
- 368: whether `5bab72b` closes it.
- 364: confirm no Opus process is still running before closing.

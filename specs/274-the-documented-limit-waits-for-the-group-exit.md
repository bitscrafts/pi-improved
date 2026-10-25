# Spec 274: the documented-limit control waits for the detached grandchild to leave the group

## Overview
**Project**: /projects/pi-improved
**Phase**: production — gate reliability. On 2026-10-25 the full gate ran on the combined tree of specs 272+273
(HEAD `ae276bc`) and failed at `test (lib/bins/tests)` on exactly one test:
`crates/pii-core/tests/child_group_reaped.rs:131`,
`a_process_that_left_the_group_on_purpose_is_the_documented_limit` — the control that PINS the reaper's limit
(a `setsid`'d grandchild survives the group kill). The tree was correct and the test was wrong.

Measured the same hour: the same binary failed **3/3** immediately after the gate's failure, then passed **6/6**
run alone minutes later, and passed **3/3** under `strace`. A reproduction of the fixture's own ordering
(`_tmp/diag/race.py`, 8 trials each) shows the mechanism: killing the group as soon as `/bin/sh` exits lets the
grandchild survive **4/8** times, while waiting until the grandchild has its own session id lets it survive
**8/8**. The kill races `setsid()`: when the grandchild has not yet left the group, `killpg(SIGKILL)` reaches it.

So the control does not measure the documented limit — it measures a scheduler race. Under the gate's load
(many test binaries at once) the race is lost often, and a gate step fails on a correct tree. That is the worst
class of failure this repo has recorded: a check certifying a defect that does not exist.

## Verified Premises
- **CONFIRMED** (2026-10-25 06:22Z, gate log `/tmp/qgate-272-273.log:670`): `FAIL: test (lib/bins/tests)` names
  only `a_process_that_left_the_group_on_purpose_is_the_documented_limit`; the other three tests in the file
  passed in the same run (`3 passed; 1 failed`).
- **CONFIRMED** the test binary is NOT stale: `_tmp/gate-target/test-gate/debug/deps/child_group_reaped-93fb8a1c27d2b02b`
  is dated `2026-10-23T16:00:42Z`, newer than both `crates/pii-core/src/childproc.rs` (`11:30:27Z`) and the test
  (`2026-10-22T19:56:20Z`), so the failure is the current code's behaviour, not an export-baked artefact (spec 236).
- **CONFIRMED** `run_and_read_pid` (`child_group_reaped.rs:33`) calls `child.reap_group()` at line 47 the instant
  `wait_timeout` returns, and `reap_group` is `kill_group(self.pid())` = `killpg(<child pid>, SIGKILL)`
  (`crates/pii-core/src/childproc.rs:82`, `:383`). The child runs in its own process group (`process_group(0)`),
  so the group is reachable until the grandchild's own `setsid()` completes.
- **CONFIRMED by measurement** (`python3 _tmp/diag/race.py`): the fixture's ordering → survived **4/8**;
  the same script waiting for `/proc/<pid>/stat` field 6 (session id) `== pid` before the kill → survived **8/8**.
- **CONFIRMED** the same binary passed **6/6** without `strace` and **3/3** with `strace -f -e trace=setsid,kill`.
- **NOT CLAIMED**: that the reaper's limit is wrong, or that `reap_group` should wait. The product behaviour is
  correct and documented; only the control's measurement is broken. NOT CLAIMED that any other test in the file
  races: the other three assert that what the child started IS killed, and for them the early kill is the point.

## Requirements

### Functional
- **R1. The limit control waits for the group exit.** In `a_process_that_left_the_group_on_purpose_is_the_documented_limit`,
  the group kill must not run until the grandchild has actually left the child's process group, measured from the
  OS: poll `/proc/<pid>/stat` until field 6 (session id) equals `<pid>` (or, equivalently, its process-group id in
  field 5 differs from the child's pid), with a bounded deadline of 5 s. A fixed `sleep` is NOT the fix and does
  not satisfy R1; the condition is read, not guessed.
- **R2. A timeout is loud.** If the grandchild has not reached its own session within the deadline, the test FAILS
  with a message naming the pid and the wait — it must never fall through and let the group kill decide the verdict
  silently.
- **R3. No assertion is weakened.** `escaped` is still the assertion (`assert!(escaped, "documenting the limit: …")`),
  the pid is still read from the pidfile the shell wrote, and the cleanup kill of the grandchild stays. The limit
  is still documented as a limit.
- **R4. The other three tests keep their meaning.** `a_background_process_the_child_started_does_not_outlive_it`,
  `a_child_that_started_nothing_is_unaffected` and `killing_a_child_also_kills_what_it_started` must still reap
  immediately after the parent exits (no wait introduced into their path) and must still assert that the
  grandchild dies. Restructure `run_and_read_pid` only as far as needed: e.g. a helper that spawns, waits and
  returns `(ChildProc, pid)` and a thin wrapper that reaps — the three killing tests call the wrapper unchanged.
- **R5. The wait is load-bearing (TDD Contract).** Removing the wait must make the repeated run red: with the
  wait deleted, `seq 1 20 | xargs -I{} …` fails at least once (measured 4/8 survival before the fix). The reviewer
  performs this mutation and restores the file byte-identical (`sha256sum` before and after, then `touch`).

### Non-Functional
- One file changes: `crates/pii-core/tests/child_group_reaped.rs`. No `src/` change, no new dependency, no new
  crate. Stay under the 450-line gate. `cargo fmt` and `clippy` clean on `pii-core`.
- The test must not become slow: the wait is a `/proc` poll with a 1 ms step and returns as soon as the condition
  holds, so the wall time of the file stays ~1 s.

## TDD Contract
1. **RED (fixture, measured):** load the box (`seq 1 8 | xargs -P 8 -I{} cargo test -q -p pii-core`), then run
   `seq 1 20 | xargs -I{} cargo test -q -p pii-core --test child_group_reaped -- --exact a_process_that_left_the_group_on_purpose_is_the_documented_limit`
   and record the failure count. The current file reds at least once per ~8 runs (measured 4/8, 3/3 in the gate).
2. **GREEN:** after the fix the same 20-iteration run passes 20/20, and the whole file reports
   `4 passed; 0 failed`.
3. **Mutation:** delete the wait only (keep the assertion) → the 20-iteration run must fail at least once; restore
   the file, `touch` it, re-run → 20/20. Quote the sha256 before and after.
4. **Controls:** the three killing tests still pass unmodified in behaviour, and a run of them under the same load
   still shows the grandchild dead.

## Exit Criteria
- [ ] `cargo test -q -p pii-core --test child_group_reaped 2>&1 | grep -qE "^test result: ok\. 4 passed"`
- [ ] `seq 1 20 | xargs -I{} cargo test -q -p pii-core --test child_group_reaped -- --exact a_process_that_left_the_group_on_purpose_is_the_documented_limit 2>&1 | grep -c "test result: ok" | grep -qx 20`
- [ ] `cargo test -q -p pii-core --test child_group_reaped -- --exact killing_a_child_also_kills_what_it_started 2>&1 | grep -qE "^test result: ok\. 1 passed"`
- [ ] `git diff --quiet -- crates/pii-core/src/childproc.rs`
- [ ] `test "$(wc -l < crates/pii-core/tests/child_group_reaped.rs)" -lt 450`
- [ ] `cargo test -q -p pii-core --lib 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo fmt -p pii-core -- --check`
- [ ] `! cargo clippy -q -p pii-core --all-targets 2>&1 | grep -qE "^(error|warning)"`

## Guardrails
- Do NOT change `crates/pii-core/src/childproc.rs`, `reap_group`, `kill_group` or the reaper's actual behaviour.
- Do NOT weaken, delete or `#[ignore]` a test; do NOT replace the assertion with a print.
- Do NOT introduce a fixed `sleep` as the "wait" (R1) and do NOT make the file slower than ~2 s.
- Do NOT touch `specs/`, `scripts/autonomy/`, or the disk-floor and spec-graph work in the tree.
- Do NOT commit. Do NOT delete anything. If the premise does not hold when you look (the grandchild no longer
  races the kill), STOP and say so instead of forcing the change.

## Follow-ups (not this spec)
1. A `pgrep -f "sleep 100000"` leftover check is NOT added as a criterion: the checking command line itself
   contains that string and the check fires on a correct tree (measured 2026-10-25: `pgrep -fc` counted the
   probe's own shell). A leftover check must match `comm`, not a full command line.
2. The gate's `test` step runs every test binary at once; one load-sensitive control can fail the whole gate.
   Whether the gate should retry a single test binary once (with the retry journalled, never hidden) is an
   operator decision — not decided here.

## Closure
**Closed 2026-10-25 by the orchestrator** (the fixture repair was made in the tick that measured the gate failure;
the tree it edits is a test file, `src/` untouched).

- **All 8 exit criteria rc=0** on the committed content (`7249f6d6…`): the file `4 passed; 0 failed`; the
  single-test loop **20/20** while the previous cycle's gate was still loading the box; the killing control
  `1 passed`; `git diff --quiet crates/pii-core/src/childproc.rs`; 177 lines; `pii-core --lib` green; `fmt` clean;
  `clippy -p pii-core --all-targets` no `error|warning` (one `unused_mut` was fixed, not allowed).
- **Mutation m1** (the `wait_for_own_session` assert deleted, every assertion kept): the 20-iteration loop red
  **1 of 20**; file restored from a copy, `touch`ed, sha256 `7249f6d6…` before and after, then the loop and the
  file green again. Note the honest shape of that number: the race is load-dependent (4/8 in a direct
  reproduction, 3/3 immediately after the gate's own failure), so a single 20-run sample can red once — it still
  proves the wait is what the verdict rests on.
- **The gate that started this was re-read**: `FAIL: test (lib/bins/tests)` was its ONLY failing step;
  `coverage-floor: total 82.89% >= 74.00%`, `lint-files`, `lint-scratch`, `lint-observability`,
  `reachability-check`, `deps-check`, `net-check`, `doc-drift-check`, `devbox-check` all PASS.
- **What this did NOT do**: it did not commit specs 272/273's work (rows A137/A138 stay `needs-review` with their
  sessions); their commit script is re-run against the fixed tree. And it does not decide follow-up 2.


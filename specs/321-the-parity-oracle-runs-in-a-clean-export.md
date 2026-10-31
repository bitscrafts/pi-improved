# Spec 321: the lane's parity oracle runs in a clean export — the `_tmp/` refusal keeps its default and gains an exactly-`1` knob

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the product is one binary and ships no companion scripts (user, 2026-10-29/30)

Spec 319's oracle is a **differential**: `pii lane run` and `scripts/run-batchC-pii.sh` are run against the same recorded stub and their
argv sequence, stdout, log, session file and exit code must be identical. It holds in `/projects/pi-improved` and it does **not** hold in a
clean export: measured 2026-10-30 on HEAD `96df40e`, `cargo run -p xtask -- fresh-gate` fails with
`failing tests: ["the_recorded_ssh_sequence_equals_the_shells"]` — the shell reference exits 0 while the binary exits 2 and prints nothing.

The cause is measured, not inferred. R1 of spec 317 refuses a lane root whose path lies inside `_tmp/`
(`crates/pii-cli/src/cmd_lane/shell.rs:96`, `pii lane: <path> is inside _tmp/: refused`, exit 2), and `fresh-gate` exports HEAD into
`/projects/_tmp/fresh-gate-<pid>` (`xtask/src/fresh_gate.rs`, `BASE`). The shell half never sees that refusal because
`scripts/run-batchC-pii.sh` hardcodes `REPO=/projects/pi-improved`; in an export the two halves are not even reading the same tree.

The refusal is right as a **default** — a lane must not run on a scratch clone — but the gate that models CI runs the whole suite in a
scratch tree, so the default makes the oracle unpassable there. This spec keeps the default byte-for-byte and adds the deliberate escape
hatch the repo already uses for the same situation (`PII_GRAPH_EDIT_OK=1`, spec 317 R6): an exactly-`1` knob, a NOTE line, and the
differential setting it.

## Verified Premises
- **CONFIRMED (measured this tick)** `cargo run -q -p xtask -- fresh-gate` at HEAD `96df40e` fails only on
  `the_recorded_ssh_sequence_equals_the_shells`; the log is `_tmp/fresh-gate-319-closure.log`, the export
  `/projects/_tmp/fresh-gate-3943206`, and the failure text prints the shell's whole block (exit 0) beside an empty binary stdout
  (exit 2).
- **CONFIRMED (read)** the refusal is `crates/pii-cli/src/cmd_lane/shell.rs:96` — `"pii lane: {} is inside _tmp/: refused"` — reached from
  the argument/root handling before `plan` builds anything, which is why the binary prints no `ABORT:` line.
- **CONFIRMED (read)** `fresh-gate` exports into a `fresh-gate-<pid>` directory under a `_tmp` base, so every CI-model run puts the tree
  the suite reads inside `_tmp/`.
- **CONFIRMED (read)** the differential (`crates/pii-cli/tests/lane_run_parity.rs`, `fn differential`) runs the shell with
  `current_dir(&live)` and the script's hardcoded `REPO`, and the binary with `--root <live>`: in the dev tree both name
  `/projects/pi-improved`, in an export they name different trees.
- **NOT CLAIMED**: that the refusal should be removed, or that a lane may run on an arbitrary `_tmp` clone. The default stays.

## Requirements
### Functional
- R1 (the default is unchanged). A `pii lane plan`/`pii lane run` whose root lies inside a path component named `_tmp` still refuses
  with the current text, stream and exit code (2), byte for byte, when `PII_LANE_TMP_ROOT_OK` is unset or is anything other than `1`.
- R2 (the knob). `PII_LANE_TMP_ROOT_OK=1` **exactly** (`true`, `01`, `0` and the empty string do not) allows the run to continue, after
  printing `NOTE: the lane root <root> is inside _tmp/ (PII_LANE_TMP_ROOT_OK=1)` on stdout, before the plan's printed block. The NOTE names
  the root as given (`--root`'s value, canonicalised the same way the refusal names it).
- R3 (the differential sets it). `fn differential` in `crates/pii-cli/tests/lane_run_parity.rs` sets `PII_LANE_TMP_ROOT_OK=1` in the
  binary's environment, so both halves run on the same tree wherever the suite runs. No assertion of that test changes: the comparison
  (argv sequence, stdout, stderr, log, session file, exit code) stays exactly as committed.
- R4 (the knob is a documented escape hatch, not a second code path). It changes one predicate; nothing else in `plan`, `run`, the printed
  block, the goldens, the refusal text or any exit code moves.
- R5 (two new tests, both in `lane_run_parity.rs`): `a_tmp_root_is_refused_without_the_knob` — a root inside `_tmp/` with no knob refuses
  with the exact text and exit 2, and records **no transport call**; `a_tmp_root_runs_with_the_knob_and_says_so` — with `1` the NOTE is
  the first line of stdout and the run proceeds. Control: `the_tmp_knob_needs_exactly_1` — `PII_LANE_TMP_ROOT_OK=true` still refuses.

### Non-Functional
- Rust, clippy and fmt at zero; the knob is read once in `LaneEnv` with every other variable (a `tmp_root_ok: bool`, `var(...).as_deref()
  == Some("1")`); no `println!`; no new file needed — `shell.rs` holds the predicate it already holds.
- The tests stay in the existing file and reuse its `Stub`; no second scaffold, no new golden.

## Architecture
```
lane root ─► inside _tmp/? ── no ───────────────► plan (unchanged)
                    └─ yes ─► PII_LANE_TMP_ROOT_OK == "1" exactly? ─ no ─► "pii lane: … is inside _tmp/: refused", exit 2
                                                                        └ yes ─► NOTE … , then plan (unchanged)
```
Decisions: (1) the knob mirrors `PII_GRAPH_EDIT_OK`'s exact-`1` shape rather than inventing a new convention; (2) the NOTE goes to stdout
before the block so a reader of a cycle log sees why an unusual root was accepted; (3) the differential sets the knob rather than teaching
the binary about exports — the binary must not know what `fresh-gate` is.

## TDD Contract
Acceptance test: `crates/pii-cli/tests/lane_run_parity.rs` (the existing file; the differential must pass under `fresh-gate --worktree`).
The requirement a plausible implementation gets wrong is **R2's exactness** — a `var(...).is_some()` reads as obviously right and silently
widens the hatch to `PII_LANE_TMP_ROOT_OK=0`.
Tests that must exist and pass: `a_tmp_root_is_refused_without_the_knob`, `a_tmp_root_runs_with_the_knob_and_says_so`,
`the_tmp_knob_needs_exactly_1`, and the pre-existing `the_recorded_ssh_sequence_equals_the_shells` and its siblings, unchanged.
Mutations the tests must kill: the refusal removed outright; `is_some()` instead of `== "1"`; the NOTE after the printed block; the NOTE
without the root; the knob honoured on the shell half as well; exit 1 instead of 2 on the refusal.

## Exit Criteria
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q "fresh-gate: PASS"`
- [ ] `cargo test -q -p pii-cli --test lane_run_parity 2>&1 | grep -q "test result: ok"`
- [ ] `cargo test -q -p pii-cli --test lane_run_parity -- --exact a_tmp_root_is_refused_without_the_knob the_tmp_knob_needs_exactly_1 2>&1 | grep -q "2 passed"`
- [ ] `cargo test -q -p pii-cli --test lane_plan_parity 2>&1 | grep -q "test result: ok"`
- [ ] `cargo clippy -q -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `git diff --quiet HEAD -- graphs scripts crates/pii-cli/tests/golden`

## Files
- `crates/pii-cli/src/cmd_lane/shell.rs`
- `crates/pii-cli/src/cmd_lane/env.rs`
- `crates/pii-cli/tests/lane_run_parity.rs`

## Guardrails
- Do NOT weaken the differential: no assertion, normalisation or comparison in it may change. The only edit to that file is the one
  `env(...)` line plus the three new tests.
- Do NOT edit `scripts/`, `graphs/`, `specs/`, or the part-1 goldens. `scripts/run-batchC-pii.sh` stays the reference, hardcoded `REPO`
  and all.
- Do NOT make the binary aware of `fresh-gate`, of export directories, or of the suite: the hatch is an environment knob, nothing else.
- The first criterion is the one that decides this item: a green dev-tree suite is not the acceptance — **`fresh-gate: PASS` is**.
  `fresh-gate` needs free disk under `/projects/_tmp` and takes about 12 minutes; run it once, read its last line, and do not start a
  second one on the same tree (a second export reuses the shared test cache and can fail on a stale binary, not on the code).
- **STOP and report** if the refusal no longer lives in `crates/pii-cli/src/cmd_lane/shell.rs`, or if `fresh-gate` no longer exports
  under a `_tmp` base: both are premises measured at HEAD `96df40e`.

## Closure

### 2026-10-31 12:3xZ — criterion 3 was unsatisfiable, not failing (orchestrator)

The item sat parked after three attempts on "the same mechanical failure". The cause is in this spec, not
in the implementation: criterion 3 filtered `lane_run_parity` for `a_tmp_root_is_refused_without_the_knob`
**and `the_knob_needs_exactly_1`** — but that second name is spec 320's stale-binary-label knob, which lives
in `crates/pii-cli/tests/lane_binary_label.rs`. Under `--exact` in this file it collects nothing, so the
filtered run printed `1 passed` and `grep -q "2 passed"` could never match: no implementation could satisfy
it. The `_tmp` knob's own control in this file is `the_tmp_knob_needs_exactly_1`.

Repaired in three places (R5 prose, the TDD list, Exit Criteria 3) to name the test that exists. The
assertion is unchanged — both tests still must run and pass. Control that the repair is not vacuous, measured
on this tree at HEAD `59cfd4c`:

```
cargo test -q -p pii-cli --test lane_run_parity -- --exact a_tmp_root_is_refused_without_the_knob the_knob_needs_exactly_1
  -> test result: ok. 1 passed; 0 failed; 24 filtered out      (the old criterion: cannot print "2 passed")
cargo test -q -p pii-cli --test lane_run_parity -- --exact a_tmp_root_is_refused_without_the_knob the_tmp_knob_needs_exactly_1
  -> test result: ok. 2 passed; 0 failed; 23 filtered out      (the criterion, rc 0)
```

The other six criteria are green on the tree (`lane_run_parity` 25 passed, `lane_plan_parity` 41 passed,
`clippy -D warnings` rc 0, `lint-files` rc 0 with max effective 446, and the goldens/graphs/scripts diff
rc 0). Criterion 1 (`fresh-gate --worktree`) is the one that still decides the item and is not claimed here.

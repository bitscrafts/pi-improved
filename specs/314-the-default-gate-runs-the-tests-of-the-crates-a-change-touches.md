# Spec 314: the default gate runs the tests of the crates a change touches

## Overview
**Project**: <repo>
**Phase**: production — gate honesty. `bash quality-gate.sh` is the command the repo documents as *the* gate, and
the one an operator or an agent runs on the working tree before committing. In its default (`--routine`) mode it
runs `$GATE_STEPS`, which `scripts/autonomy/env.sh:47` defaults to `gate-steps-quick.txt`: three steps, `fmt`,
`clippy`, `build`. **No step runs a test.**

Measured consequence (`_docs/05-operations/open-items.md` §9b, 2026-10-18): commit `03e729a` left two tests red at
`HEAD` — `daemon::lag::tests::lagged_run_finished_still_starts_next_cron_firing` and
`…reactive_loop_lag_rearms_cron_chain_for_lost_run_finished`, `cargo test -p pii-server --lib` = **127 passed,
2 failed** — and no cycle noticed for 1.5 h, because `quality-gate.sh` printed `ALL PASS` having compiled the tree
and never run a test. A gate that certifies a tree it never tested is the failure class this project calls
"green but unproven".

The cost question that blocked the change is already answered (§9b, measured 2026-10-18, clean tree, HEAD
`136b23e`): a full `cargo test -j 2 --workspace --lib --bins --tests` against a cold cache took **293 s**; scoped
to one crate, `cargo test -j 2 -p pii-server --lib --bins` took **96 s**. A diff-scoped test step is therefore
affordable in a per-commit gate.

**Scope note.** The queue already gates with the *full* list (`cycle.sh:367` exports `GATE_STEPS=gate-steps.txt`,
spec 262), which contains `test (lib/bins/tests)`. This spec changes the **default/local** gate only: it adds a test
step to `gate-steps-quick.txt`, so `bash quality-gate.sh` stops certifying an untested tree. It does not change the
queue's cost and does not touch `gate-steps.txt`.

## Verified Premises
- **CONFIRMED (read, 2026-10-29)** `quality-gate.sh:58-73`: routine mode reads `$GATE_STEPS` line by line and runs
  each `label|command`; `scripts/autonomy/env.sh:47` sets `GATE_STEPS:=gate-steps-quick.txt` when unset;
  `gate-steps-quick.txt` holds exactly three lines: `fmt`, `clippy`, `build`. No test step exists.
- **CONFIRMED (read, 2026-10-29)** `gate-steps.txt` already contains `test (lib/bins/tests)`, so this is a gap in the
  quick list only, and adding a step there cannot double-test the queue's path.
- **MEASURED (open-items §9b, 2026-10-18)** the two costs above (293 s workspace cold, 96 s one crate), so a
  diff-scoped step is inside a per-item budget.
- **CONFIRMED (read, 2026-10-29)** the workspace members are `crates/<name>` (package name equals the directory
  name today, e.g. `crates/pii-graph/Cargo.toml` → `pii-graph`) plus the root-level `xtask`.
- **NOT CLAIMED**: that a diff-scoped test step is a substitute for the full suite. It is not; it is what a
  pre-commit gate can afford, and `gate-steps.txt` remains the boundary that runs everything.

## Requirements

### Functional
- **R1.** A committed helper `scripts/gate-touched-crates.sh` maps repository paths to the workspace packages that
  own them: `crates/<dir>/…` → the `name` read from `crates/<dir>/Cargo.toml` (never assumed from the directory
  name), `xtask/…` → `xtask`. A path outside every member (`_docs/`, `scripts/`, `specs/`, root files other than
  the workspace manifest) maps to no package.
- **R2.** With no path arguments the helper derives the list from the live tree — `git status --porcelain=v1 -z`
  (staged, unstaged and untracked) — so the shapes git actually prints (a rename, a quoted path) are handled by
  git's own output format and not by a second parser of our own. Paths may also be passed explicitly for tests.
- **R3.** `--print` prints the exact command it would run, one line, and runs nothing. `--run` runs it and exits
  with its status. The command is `cargo test -j 2 -p <pkg> -p <pkg> … --lib --bins --tests`, packages sorted and
  de-duplicated, so two files in one crate yield that crate once.
- **R4.** A change to the workspace manifest (`Cargo.toml`) or `Cargo.lock` names **every** member (the 293 s case):
  those files can break any crate, so the step must not narrow them away.
- **R5.** Nothing to test is a printed, explicit line, never silence: no touched crate prints
  `gate-touched-crates: no crate touched` and exits 0; a path under `crates/` that resolves to no package (a typo,
  a removed crate) prints `gate-touched-crates: <path> names no workspace package` on stderr and exits **2** — a
  loud failure, never a silent green.
- **R6.** `gate-steps-quick.txt` gains one step, `test (touched crates)|bash scripts/gate-touched-crates.sh --run`,
  so the default `bash quality-gate.sh` runs it and a red test in a touched crate fails the gate.
- **R7.** When the tree is not a git repository (the `git archive` export `fresh-gate` builds), the helper prints
  `gate-touched-crates: no git repository — nothing to test here` and exits 0. That is honest: the export is gated by
  `gate-steps.txt`, whose `test (lib/bins/tests)` step runs the whole suite. The line must be printed, so an operator
  reading the log can see the step did nothing and why.

### Non-Functional
- One process, no new dependency, no Rust change. Both the helper and its test stay under 450 effective lines.
- The step prints its own duration line so the gate log shows what the test step cost.
- The helper never writes to the tree and never deletes anything.

## TDD Contract
`scripts/autonomy/tests/gate-touched-crates.test.sh` prints `N ok, M FAIL` as its last line (the house style), and
drives the helper **through its live interface** — it never re-types the mapping in the test.

| Test | Input | Expected |
|---|---|---|
| `a-crate-source-maps-to-its-package` | `crates/pii-graph/src/sched/state.rs` | `--print` output contains `-p pii-graph` |
| `a-crate-test-file-maps-too` | `crates/pii-graph/tests/loops.rs` | same single `-p pii-graph` |
| `two-files-one-crate-name-it-once` | both of the above | exactly one `-p pii-graph` occurrence |
| `the-root-member-maps` | `xtask/src/main.rs` | `-p xtask` |
| `docs-and-scripts-name-no-crate` | `_docs/05-operations/open-items.md scripts/autonomy/plan.tsv` | no `-p ` at all, exit 0, prints `no crate touched` |
| `the-workspace-manifest-names-every-member` | `Cargo.toml` | the command contains `-p pii-graph` **and** `-p xtask` (not a narrowed set) |
| `an-unknown-crate-path-refuses` | `crates/no-such-crate/src/lib.rs` | exit **2**, stderr names the path |
| `the-live-git-path-is-what-is-read` | a scratch git repo under `_tmp/` (the test creates and commits it, then modifies one file) with `GATE_REPO` pointing at it | `--print` names that crate — the list came from git, not from the argument list |
| `CONTROL-a-clean-tree-names-nothing` | the same scratch repo with nothing modified | `no crate touched` |
| `CONTROL-the-step-line-is-live` | the `test (touched crates)` line **extracted from `gate-steps-quick.txt` at run time** | its command runs the helper with `--run` (so a step wired to something else, or removed, fails this row) |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/gate-touched-crates.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `! bash scripts/autonomy/tests/gate-touched-crates.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `grep -q "gate-touched-crates.sh --run" gate-steps-quick.txt`
- [ ] `bash scripts/gate-touched-crates.sh --print crates/pii-graph/src/sched/state.rs | grep -q -- "-p pii-graph"`
- [ ] `test -z "$(bash scripts/gate-touched-crates.sh --print _docs/05-operations/open-items.md scripts/autonomy/plan.tsv)"`
- [ ] `bash scripts/gate-touched-crates.sh --run crates/pii-abi/src/lib.rs`
- [ ] `bash -n scripts/gate-touched-crates.sh`
- [ ] `git diff --quiet -- gate-steps.txt specs scripts/autonomy/cycle.sh scripts/autonomy/state.py`

## Files
- `scripts/gate-touched-crates.sh`
- `scripts/autonomy/tests/gate-touched-crates.test.sh`
- `gate-steps-quick.txt`

## Guardrails
- Do NOT touch `gate-steps.txt`, `quality-gate.sh`, `scripts/autonomy/env.sh` or `cycle.sh`: the queue's gate and the
  release boundary are decided elsewhere, and this spec changes the quick list only.
- Do NOT delete or narrow any existing step, and do NOT weaken `fmt`, `clippy` or `build`.
- Do NOT re-implement git's quoting rules: read `-z` output (NUL-separated, unquoted) or shell out to
  `git diff --name-only`; a hand-rolled parser of `status --porcelain`'s quoted form is the defect spec 277 fixed.
- Do NOT map a package by stripping the `crates/` prefix and assuming the directory name is the package name: read
  `name =` from the crate's `Cargo.toml`, so a rename does not silently test nothing.
- Do NOT make "nothing to test" silent, and do NOT make an unresolvable `crates/` path pass.
- Do NOT add `--workspace` for an ordinary crate change: the 293 s case is only for `Cargo.toml`/`Cargo.lock`.
- Do NOT edit `specs/` or `scripts/autonomy/plan.tsv`.
- **If the premises do not hold** — `gate-steps-quick.txt` is no longer the default list, its format is no longer
  `label|command`, or the members are not `crates/<dir>` plus `xtask` — STOP, record the measurement in
  `## Closure`, and change nothing else. A spec whose premise is false is not implemented by force.

## Follow-ups (not this spec)
1. The full gate (`gate-steps.txt`) could reuse the same helper to run a touched-crate step *before* the workspace
   suite, so a red item is named in seconds instead of after the whole suite.
2. `scripts/autonomy/env.sh:47` still defaults to the quick list for anything that forgets `GATE_STEPS`; after this
   spec a forgotten variable is no longer a silent untested gate, which is the reason the default can stay.

## Closure
**Verified by the orchestrator on 2026-10-30, on the committed tree — HEAD `2dee03d`, worktree clean.**

- **Exit criteria, run by hand on that tree:** **8 of 8 green** (criterion 6 ran the helper for real:
  `-p pii-abi --lib --bins --tests` → 47 passed, 0 failed, `took 1s (exit 0)`; criterion 5 printed
  `gate-touched-crates: no crate touched`, not silence). The fixture printed **18 ok, 0 FAIL**.
- **The item was reddened three times by the orchestrator's own mutations**, each restored byte-identical:
  1. `gate-steps-quick.txt` last line `… --run` → `… |true` (a step that tests nothing — the defect class of this
     spec): **16 ok, 2 FAIL** — `CONTROL: the step line runs the helper with --run` and
     `CONTROL: the step line, run on a clean tree, names nothing`. Restored (plain `cp`, then `touch`):
     `sha256 3359b06c842b4399be83be324382753b90849c071d0de27b6e997bc706bf35ed`, identical before and after;
     green again at 18 ok, 0 FAIL.
  2. `scripts/gate-touched-crates.sh` R4 narrowed — the `Cargo.toml | Cargo.lock)` case appends nothing:
     **15 ok, 3 FAIL** (`the manifest names pii-graph`, `… xtask`, `… every member`). Restored:
     `sha256 13f1a1ee52bceadf358ec2c3d1caf17ea913f0d7a421e1460da5cf6795861abc`, identical both sides.
  3. The same helper's R5 silence — an unresolvable `crates/` path returns without setting `BAD`:
     **16 ok, 2 FAIL** (`an unknown crate path refuses with exit 2`, `… names itself`). Restored: same
     `sha256 13f1a1ee…1abc`, identical both sides; green at 18 ok, 0 FAIL.
- **`fresh-gate` on HEAD: PASS** — `_tmp/autonomy/logs/fresh-gate-a178-20261030T001628Z.log`, rc 0,
  `quality-gate: ALL PASS`, `fresh-gate: PASS`. **Honest gap:** that run carried `QG_SKIP_COVERAGE=1`, so the
  `coverage-floor` step read `SKIPPED` and the floor was *not* measured in this run; the two `FAIL unreadable` /
  `FAIL store down` lines in the log are `cli_util`'s own scenario output inside a passing test, not gate steps.
- **What committed it was the in-place gate, not a fresh-gate.** The cycle that landed `2dee03d`
  (`_tmp/autonomy/logs/cycle-20261029T182004Z.log`) ended `quality-gate: ALL PASS` on the working tree; this
  Closure supplies the export-of-HEAD verdict it did not have. That difference is exactly §9a of
  `_docs/05-operations/open-items.md`, still unauthored.
- **Selftest:** `42 passed, 0 failed — VERDICT: ALL MECHANISMS PASS` (`_tmp/autonomy/logs/selftest-20261030T001617Z.log`).
- **Finding worth keeping.** The step this spec adds is the one that would have caught the defect it documents
  (§9b: `03e729a` left two `pii-server` tests red at HEAD for 1.5 h behind `ALL PASS`), and mutation 1 shows the
  fixture refuses a step wired to anything else — the control reads the real `gate-steps-quick.txt` line at run
  time rather than re-typing the command beside it.

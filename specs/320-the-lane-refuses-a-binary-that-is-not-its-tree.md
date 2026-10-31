# Spec 320: the lane refuses a binary that is not the tree it is about to run, and `pii lane rev` says which build it is

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the product is one binary and ships no companion scripts (user, 2026-10-29/30)

Parts 1 and 2 (specs 317, 319) put the composed lane's whole body — setup (`pii lane plan`) and run (`pii lane run`) — inside the binary,
with a parity test against `scripts/run-batchC-pii.sh` as the oracle. The remaining step of that strangler is that `scripts/autonomy/cycle.sh`
dispatches **through the binary** instead of `bash scripts/run-batchC-pii.sh`, and the script is retired.

That flip is unsafe today for one measured reason: **the binary the loop would run is not necessarily the tree it would run on.** A binary
built at commit X, invoked while the tree is at commit Y, reproduces X's lane — the exact defect class spec 279 fixed on the devbox side
(`daemon.sh refresh` copied a stale binary and reported success while installing nothing new). The lane already checks its *inputs* against
the tree (the graph pin, spec 302); nothing checks the *program*.

This spec is the guard the flip depends on, and only the guard: the label the binary carries, a read-only subcommand that prints it, and a
refusal — before the daemon is touched and before anything can be spent — when the label's commit is not HEAD. It does not change
`cycle.sh`, and it does not change `pii lane plan` (part 1's goldens pin its printed block byte-for-byte).

## Verified Premises
- **CONFIRMED (read this tick)** `crates/pii-server/src/daemon/identity.rs:262` is `pub const REVISION: &str = env!("PII_GIT_REV");` and
  `crates/pii-server/src/daemon/mod.rs:12` is `pub mod identity;`, so the constant is reachable as
  `pii_server::daemon::identity::REVISION`. `crates/pii-cli/Cargo.toml:23` already depends on `pii-server`; nothing in `pii-cli` uses it yet.
- **CONFIRMED** `crates/pii-server/build.rs` sets `PII_GIT_REV` to `git rev-parse --short=12 HEAD` (12 hex characters), appends `-dirty`
  when `git status --porcelain --untracked-files=no` was non-empty at build time, and reports `unknown` when there is no checkout and no
  `PII_BUILD_REVISION` handed in. It re-runs on `.git/HEAD` and `.git/index`, so a new commit rebuilds the label.
- **CONFIRMED** `crates/pii-cli/src/cmd_lane/run.rs:178-197`: `Driver::lane` begins with `let d = (self.io.daemon)();` — the daemon-up call —
  and only then prints `DAEMON   : …`. Nothing is spent before that line; a refusal placed above it costs nothing.
- **CONFIRMED** the printed block of `pii lane plan` is pinned by `crates/pii-cli/tests/golden` and `lane_plan_parity`; `pii lane rev` is a
  **new** subcommand and must not add a line to that block.
- **CONFIRMED (measured 2026-10-30, this tick)** the host's `/root/.local/bin/pii` is dated 2026-10-20 and its usage line has no `lane`
  subcommand, so no dispatch goes through the binary today. The guard this spec adds is what makes the follow-up flip safe; it is not
  needed by any caller that exists yet.
- **NOT CLAIMED**: that the flip itself (cycle.sh → `pii lane run`) is safe. It is the next item and it needs this guard, a resolved binary
  path and a rebuild path; none of them is this spec.

## Requirements
### Functional
- R1 (`pii lane rev`). `pii lane rev` prints exactly one line, `rev <label>`, where `<label>` is
  `pii_server::daemon::identity::REVISION` — byte for byte, including a trailing `-dirty` or `unknown` — and exits 0. It reads no
  environment, writes no file, runs no `ssh`, and is not reachable through `plan`'s printed block.
- R2 (the guard, and where it sits). The **first** thing a real `pii lane run` does — before the daemon-up call of `run.rs:184`, before any
  `ssh`, before any file is written — is compare the label's commit part with the first 12 characters of the HEAD `plan` already took
  (`RepoSnapshot::head`, `cmd_lane/git.rs:95`) — no second git call. The commit part is the label with one trailing `-dirty` removed.
  Equal → the run continues unchanged.
- R3 (the refusal). Any of — the commit parts differ; the label is `unknown`; git fails — aborts with this stdout text, exit 2, and nothing
  after it (no `DAEMON   :` line, no log append, no file):
  `ABORT: this binary was built at <label>; HEAD is <sha> — rebuild: cargo build -p pii-cli --bin pii`
  where `<label>` is the label as printed by R1 and `<sha>` is HEAD's 12 characters, or `unknown` when git failed.
- R4 (the escape hatch). `PII_STALE_BIN_OK=1` **exactly** (not `true`, not `0`) skips R3 and prints
  `NOTE: running a binary built at <label> while HEAD is <sha> (PII_STALE_BIN_OK=1)` on stdout, after which the run proceeds exactly as
  today. The knob is read once, in `LaneEnv`, with every other variable.
- R5 (the label is an input, not a constant read at the call site). The label reaches the guard as a parameter of the run core
  (`pii_server::daemon::identity::REVISION` is what the thin IO shell passes in production), so every test above is an in-process test with
  a label it chooses. No test may depend on the label of the machine it runs on.
- R6 (the guard reads HEAD, not the working tree). A tree with the worker's own WIP — the normal dispatch state, `ALLOW_DIRTY=1` — passes
  when HEAD matches the label; only HEAD is compared. A staged change that does not move HEAD is not a mismatch.
- R7 (`plan` and `--dry-run` are untouched). `pii lane plan`, `pii lane run --dry-run`, the goldens under `crates/pii-cli/tests/golden` and
  `lane_plan_parity` behave exactly as they do at HEAD; `--dry-run` does not run the guard (nothing is spent there either).

### Non-Functional
- Rust, clippy and fmt at zero; every file under 450 effective lines. The comparison is pure and lives in one small module
  (`crates/pii-cli/src/cmd_lane/label.rs`) with its own unit tests; no new `build.rs`, no second build label — `pii-server`'s `REVISION` is
  the one identity this project has, and a build script that invents a second one is a defect.
- No `println!` (`writeln!(std::io::stdout(), …)` as `cmd_deploy_card`/`cmd_lane` already do); no direct `Command` spawn — git through
  `Ctx`/`ProcCap`; the environment read once into `LaneEnv`.
- Emit the abort on **stdout**, like every other `ABORT:` line of the lane, because `cycle.sh` captures stdout into its log and its
  `ABORT:` greps read it there.

## Architecture
```
pii lane rev ──────────────► label::one_line(REVISION)                    (pure)
pii lane run N ─► Driver::lane ─► label::verdict(REVISION, head12)        (pure)
                                     ├─ Match → the run continues (unchanged)
                                     └─ Stale → ABORT text, exit 2, nothing spent
```
Decisions: (1) the verdict is a pure function of two strings, so the whole rule is unit-testable without a repo, a clock or a transport;
(2) the run test asserts the **recorded transport is empty** on the abort path — that is the observable that proves "before the daemon",
not the position of a line in a file; (3) the label parameter is threaded, not read at the call site, so the tests own it.

## TDD Contract
Acceptance test: `crates/pii-cli/tests/lane_binary_label.rs`, built on the stub `ssh`/scratch-repo scaffold of
`crates/pii-cli/tests/lane_run_parity.rs` (reuse it; do not invent a second one). The requirement a plausible implementation gets wrong is
**(2)** — a guard that sits *after* `DAEMON   :` looks identical in stdout except for two lines nobody diffs, and it spends the daemon call
before refusing.
Tests that must exist and pass:
`lane_rev_prints_the_build_label`, `lane_rev_prints_a_dirty_label_verbatim`, `a_matching_label_runs_past_the_guard` (control),
`a_stale_label_aborts_with_the_rebuild_command`, `a_stale_label_records_no_transport_call` (the daemon was never touched),
`a_stale_label_writes_no_log_and_no_file`, `the_dirty_suffix_is_not_a_mismatch` (control: `abc123def456-dirty` vs `abc123def456`),
`an_unknown_label_aborts`, `git_unreadable_aborts_with_unknown_sha`, `the_knob_prints_a_note_and_runs`, `the_knob_needs_exactly_1`
(`PII_STALE_BIN_OK=true` still aborts — control), `the_guard_reads_head_not_the_working_tree` (a dirty tree at a matching HEAD runs).
Mutations the tests must kill: the guard moved below the daemon-up call; the `-dirty` suffix compared raw; `unknown` accepted as matching;
`PII_STALE_BIN_OK` accepting any non-empty value; the abort text without the rebuild command; exit 1 or 9 instead of 2; the NOTE printed
without the label; the label read from the environment instead of the parameter.

## Exit Criteria
- [ ] `cargo build -q -p pii-cli --bin pii`
- [ ] `./target/debug/pii lane rev 2>&1 | grep -qE "^rev [0-9a-f]{12}(-dirty)?$"`
- [ ] `cargo test -q -p pii-cli --test lane_binary_label 2>&1 | grep -q "test result: ok"`
- [ ] `cargo test -q -p pii-cli --test lane_binary_label -- --exact a_stale_label_records_no_transport_call the_dirty_suffix_is_not_a_mismatch the_knob_needs_exactly_1 2>&1 | grep -q "3 passed"`
- [ ] `cargo test -q -p pii-cli --test lane_run_parity 2>&1 | grep -q "test result: ok"`
- [ ] `cargo test -q -p pii-cli --test lane_plan_parity 2>&1 | grep -q "test result: ok"`
- [ ] `cargo clippy -q -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `git diff --quiet HEAD -- crates/pii-cli/tests/golden graphs scripts`

## Files
- `crates/pii-cli/src/cmd_lane/label.rs`
- `crates/pii-cli/src/cmd_lane/mod.rs`
- `crates/pii-cli/src/cmd_lane/run.rs`
- `crates/pii-cli/src/cmd_lane/env.rs`
- `crates/pii-cli/tests/lane_binary_label.rs`

## Guardrails
- Do NOT edit `scripts/`, `graphs/`, `specs/`, or any existing test assertion. `scripts/run-batchC-pii.sh` stays the reference; `cycle.sh` is
  not touched by this spec at all.
- Do NOT add a `build.rs` to `pii-cli` and do NOT introduce a second identity string: `pii_server::daemon::identity::REVISION` is the label.
- Do NOT print anything before the guard that writes a file; the abort path must leave the tree exactly as it found it.
- **STOP and report** if `pii_server::daemon::identity::REVISION` turns out not to be reachable from `pii-cli` under the workspace's feature
  selection, or if `Driver::lane` no longer begins with the daemon-up call. Both are premises of this spec, measured at HEAD; a shape
  mismatch is a corrected brief, not a redesign.
- The guard must not fire on `--dry-run`: a dry run spends nothing and is how a person inspects the plan of a tree with an older binary.

## Closure

### 2026-10-31 12:3xZ — the exit criteria re-run by the orchestrator, independently of the worker's report

All nine criteria rc 0 on the tree at HEAD `59cfd4c` (the worktree also carried an uncommitted, unrelated
`spec 322` change in `cmd_daemon.rs` / `pii-learn` / `pii-observe`; nothing this spec lists):

```
build -q -p pii-cli --bin pii                     rc 0
pii lane rev                                      rev 59cfd4c34d6f-dirty   (the -dirty suffix is the label's documented form)
cargo test -p pii-cli --test lane_binary_label     13 passed, 0 failed
  -- --exact a_stale_label_records_no_transport_call the_dirty_suffix_is_not_a_mismatch the_knob_needs_exactly_1
                                                   3 passed, 0 failed
cargo test -p pii-cli --test lane_run_parity       25 passed, 0 failed
cargo test -p pii-cli --test lane_plan_parity      41 passed, 0 failed
cargo clippy -p pii-cli --all-targets -D warnings  rc 0
cargo run -p xtask -- lint-files                   rc 0 (405 files, max effective 446)
git diff --quiet HEAD -- crates/pii-cli/tests/golden graphs scripts  rc 0
```

The gate half was the run recorded the same day: `fresh-gate` over an export of HEAD `59cfd4c` printed
`quality-gate: ALL PASS` / `fresh-gate: PASS` / `gate rc=0` at 07:08:28Z (`/tmp/pii-night-gate.out`). The
state row's `parked` was stale bookkeeping — the commits that carry this spec's work landed after it
(`593a936`, `bdb404d`, `c531856`, `c8c2f7b`, `db2f10b`, `eb7a3d1`, `385f78b`…`59cfd4c`).

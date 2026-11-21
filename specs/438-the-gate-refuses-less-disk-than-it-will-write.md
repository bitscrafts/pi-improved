# Spec: the gate refuses to start with less disk than it will write
#: depends=- status=pending block=-

## Overview

Measured three times on 2026-11-17/18, and every time it looked like a code defect:

- `/projects` at 100 %, a gate run produced six `FAIL`s — two tests, two discipline lints, coverage — and **every one**
  was `mkdir: No space left on device`. The code was fine; the volume was full.
- The coverage step alone writes **37 GB** (`target/llvm-cov-target` after instrumentation).
- The cycle refuses to gate below **20 GB** free (`AGENTS.md`, `scripts/autonomy/cycle.sh`), which is not enough for the
  step that runs last and needs almost twice that.

A gate that starts knowing it cannot finish spends an hour to produce failures that point at the wrong place. The floor
must be the number the gate will actually need, and the refusal must name it.

## Requirements

- **R1** The floor is **60 GB** on the build volume: 37 G for coverage plus room for the other steps and for the tree.
  The number is written once, in the binary or in a single configuration value, not repeated in three scripts.
- **R2** The refusal happens **before** the first step, not during the coverage step: the run exits non-zero with one line
  naming the free space, the floor, and the biggest contributor under `_tmp/`.
- **R3** The refusal is distinguishable from a failed gate: exit code and message both say "refused for disk", so the
  queue parks the row with `blocked=runtime:disk` (spec 434's mechanism) rather than marking it a row failure.
- **R4** `pii disk clean` replaces the ad-hoc script: it enforces the windows in
  `_docs/05-operations/disk-ceilings.md` (trash 7 days, worker logs 14 days, side caches of finished runs, target/
  cleaned), refuses any path outside `_tmp/`, refuses the warm gate cache, and prints what it will free before freeing it.
  Runtime belongs in the binary: a script that must be remembered is a script that will be forgotten.
- **R5** The clean has a `--dry-run` default. Nothing is removed unless asked.
- **R6** The preflight checks the **tools** the steps need, not only the disk: `cargo-llvm-cov` and `llvm-tools` are the
  two whose absence is measured (2026-11-18 present, 2026-11-21 absent, and the gate discovered it at step 23 of 23
  with `error: no such command: llvm-cov`). A missing tool is reported in the same one-line refusal, by name, before the
  first step runs. This is not a skip: the step is still a FAIL, discovered in one second instead of one hour.
  The list is read from where the requirement is written, not guessed: `.cargo/config.toml:1-7` forces `CC = "clang"` and
  `CXX = "clang++"` for the vendored llama.cpp build (spec 35), so the preflight checks those two by asking the
  configuration, and a run whose `clang` is absent refuses before compiling two hundred crates to find out
  (`cc-rs: failed to find tool "clang"`, measured 2026-11-21 after the tool vanished from the image).

## TDD contract

| Test | Input | Expected |
| --- | --- | --- |
| refuses below the floor | free space reported as 30 GB | exit non-zero, message names 30 and 60, no step runs |
| proceeds above the floor | 61 GB | gate starts |
| the contributor is named | `_tmp/gate-target` at 81 G | the refusal names it as the largest |
| the clean refuses a foreign path | `pii disk clean --path /projects/other` | exit non-zero, nothing touched |
| the clean refuses the warm cache | `pii disk clean --path _tmp/gate-target` | exit non-zero, cache intact |
| a missing tool is refused up front | `PATH` without `cargo-llvm-cov` | exit non-zero, message names `cargo-llvm-cov`, no step runs |

The second row is the control: a floor implemented as an unconditional refusal would pass the first row and fail this one.

## Exit Criteria

- [ ] `pii gate run --routine --dry-run 2>&1 | grep -qE 'disk|free'`
- [ ] `pii disk clean --dry-run | grep -q 'would'`
- [ ] `pii disk clean --path .pii 2>&1 | grep -qiE 'refus|outside'`
- [ ] `grep -rqE 'free_gb|AVAIL_GB|disk_floor' crates/pii-cli/src/ crates/pii-server/src/`

## Files

- `crates/pii-cli/src/cmd_gate.rs` (or the gate entry point found by reading before writing)
- `crates/pii-cli/src/cmd_disk.rs`
- `scripts/disk-clean.sh` (deleted once `pii disk clean` exists)
- `gate-steps.txt` (only if the floor is expressed there)

## Guardrails

- Do NOT satisfy this by freeing space inside the gate. A gate that cleans caches to fit is a gate that deletes the warm
  cache it depends on, and the next run pays an hour.
- Do NOT lower the floor to make a particular run pass. The floor is what the coverage step needs; if that changes, the
  number changes with a measurement, in this spec.
- Do NOT remove `scripts/autonomy/cycle.sh`'s own check until the binary's check is the one that runs first.
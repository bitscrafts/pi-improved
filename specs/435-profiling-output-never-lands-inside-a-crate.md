# Spec: profiling output never lands inside a crate
#: depends=- status=pending block=-

## Overview

Measured 2026-11-20: `crates/` held **9.0 GB for ~1 GB of source**. Inside `crates/pii-cli` a `du` listed hundreds of
`default_<id>_0_<n>.pro` files at 664 KB each — profiler output — and minutes later `find crates -name '*.pro'` returned
zero. So something writes them into the source directories *during the test run* and usually removes them.

Two reasons this is not merely untidy:

- They are **ignored**, so `git status` stays clean and nothing in the repository notices them grow.
- A run that leaves one behind surfaces as an untracked file to `scripts/autonomy/leftover.sh`, which refuses the next
  commit with `DIRTY TREE that no queue row owns` and **stops the queue**.

`lint-scratch` (see AGENTS.md) polices temporary *directories*; it does not see a stray output *file* dropped next to a
`Cargo.toml`. The ignored-not-catalogued state is also the class named in `_docs/05-operations/disk-ceilings.md`.

## Requirements

- **R1** Find the writer: which test or tool, invoked by which step, produces `default_*.pro` and into which working
  directory. Record the caller and the line in this spec before changing anything.
- **R2** The writer must write under `_tmp/` (per the lint-scratch rule), not under `crates/`. If the writer is an
  external profiler pointed at a crate directory, the invocation changes; the profile stays available for the person who
  asked for it.
- **R3** `.gitignore` gains an explicit `*.pro` rule, so the artefact is *named* rather than caught incidentally by some
  broader pattern (the tree currently ignores `*.profraw` and not `*.pro`).
- **R4** A check fails when any `*.pro` (or other profiler extension the row discovers) exists anywhere under `crates/`.
  It must fail on the file existing, not on it being tracked: ignored files are the whole problem here.
- **R5** The existing 8 GB is removed *after* R2 lands, not before: a writer that is still pointed at `crates/` will
  refill it during the very verification run.

## TDD contract

| Test | Input | Expected |
| --- | --- | --- |
| the check sees a stray profile | a fixture tree with `crates/x/default_1_0_2.pro`, untracked and ignored | check exits 1, names the path |
| the check accepts a clean tree | the real `crates/` after R5 | check exits 0 |
| the writer's target | run the profile-producing step | no new file under `crates/`; one under `_tmp/` |

The second row is the control that a check which always fails would not pass, and the first is the near-miss that a
check which only looks at tracked files would not catch.

## Exit Criteria

- [ ] `bash scripts/checks/no-artifacts-in-crates.sh`
- [ ] `grep -qE '^\*\.pro$' .gitignore`
- [ ] `find crates -name '*.pro' | wc -l | grep -qx 0`

## Files

- `scripts/checks/no-artifacts-in-crates.sh`
- `.gitignore`
- the caller of the profiler, named by R1 (path recorded here once found)

## Guardrails

- Do NOT satisfy R4 by adding `*.pro` to `.gitignore` alone. The artefact being ignored is what made it invisible; the
  check must see it anyway.
- Do NOT delete the 8 GB before R2 is proven: the verification run would recreate it and the criterion would pass on an
  empty tree for the wrong reason.
- Do NOT delete anything outside `_tmp/`: the rule is move aside, never `rm -rf` existing work.
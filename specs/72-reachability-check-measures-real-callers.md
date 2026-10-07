# Spec: 72 — reachability-check measures real callers

**Project**: /projects/pi-improved
**Phase**: intervention batch A — proven gate defect D3 (report 043 §6, Decision 5)
**Status**: NOT STARTED (worker lane)
**Derives from**: 043 report candidate 3 + Decision 5 ruling (goal approved; call-graph REJECTED)

## Overview

`cargo run -p xtask -- reachability-check` is proven blind: with BOTH of its
real callers deleted (pii-server/src/lib.rs:203,
pii-server/src/engine/node_kinds/nodes/worker.rs:38) it exits 0. Roots, all
verified by the review (C-read):
1. The seed searches the DECLARING crate only, and matches the declaring
   crate's own internal calls — deleting external callers still leaves the
   declaration's crate matching itself.
2. The needle `.invoke(` matches any same-named method on any type —
   unrelated `.invoke(` calls satisfy it.
3. A phantom documented caller `line_is_test` does not exist.
4. Hardcoded `/projects/pi-improved` path at :171.
5. The check is absent from quality-gate.sh, so nothing red ever surfaces.

## Requirements (the two-string-class fix + hardening, per Decision 5 step 1)

### R1 — seeds measure EXTERNAL callers
For each configured (symbol, needle) entry: the seed crate must NOT be the
declaring crate; internal self-matches must not count. The two real callers
(lib.rs:203 `dispatch…invoke(…)`, worker.rs:38) must be the evidence the
check depends on — deleting either or both must turn the check RED.

### R2 — exclude comments and `#[cfg(test)]` regions
Reuse `lint_files`' skipping (it already strips both); a match inside a
comment/cfg(test) block is not a caller.

### R3 — delete the phantom `line_is_test` documentation entry; replace the
hardcoded path with `env!("CARGO_MANIFEST_DIR")`-derived path (no host paths
in-tree; this is also D7's class, scoped to this file only).

### R4 — wire into quality-gate.sh (after the test step, before the summary),
so a regression turns the shared gate red.

### R5 — guardrails: no new deps (syn explicitly NOT this batch, Decision 5);
no behaviour change for checks that PASS; clippy/fmt clean.

## TDD Contract (ACCEPTANCE)

| Test | Scenario | Expected |
|---|---|---|
| `reachability_fails_when_callers_deleted` (xtask test, NEW, red today) | run the check against a scratch export with both real callers commented out | exit != 0, message names the orphaned symbol |
| `reachability_passes_on_head` | run on HEAD | exit 0 |
| `comments_are_not_callers` | a comment containing `.invoke(` in an unrelated crate, seed pointing at a symbol whose only textual match is that comment | exit != 0 (comment does not satisfy) — i.e. the check does NOT pass on the comment |

## Exit Criteria (FAILING on today's tree where applicable)

- [ ] `cargo run -q -p xtask -- reachability-check` exits 0 on HEAD
- [ ] scratch-export check with both callers muted exits NONZERO (worker proves with a script under _tmp/)
- [ ] `grep -n 'line_is_test' crates/xtask/src/reachability.rs | wc -l` = 0
- [ ] `grep -c '/projects/pi-improved' crates/xtask/src/reachability.rs` = 0
- [ ] `grep -n 'reachability' quality-gate.sh | wc -l` >= 1
- [ ] `cargo test -p xtask 2>&1 | tail -1 | grep -q '0 failed'`; clippy+fmt clean

## Implementation Notes (worker)

- Scratch exports via `git archive` under /projects/_tmp (NEVER mutate the real tree mid-check).
- Keep the check's output shape (symbols × evidence) unchanged.
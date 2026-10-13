# Spec: the test helpers are counted before a crate is built for them

## Overview

`tempfile` is now a normal dependency of `pii-store`, and `pii-store`'s `mod testing` is `pub`
(`crates/pii-store/src/testing.rs`). A `pii-testkit` crate is therefore *possible* — but the backlog's own rule
is "worth it only if a second or third crate duplicates those helpers — check before building"
(`_docs/05-operations/open-items.md` §6). This spec makes that a measurement with a written outcome, not a crate.

**Project**: /projects/pi-improved
**Phase**: the gate's own robustness (pre-video backlog, specs 145–148)

## Requirements

### Functional

R1. A census of test-helper duplication across the workspace: for every crate, the helper functions that
    duplicate another crate's concern (scratch/`_tmp` dirs, temp paths, store/redb fixtures, fake clocks,
    event capture). The census records the command used, so it can be re-run.
R2. The census and the decision are written to `_docs/05-operations/testkit-decision.md`: the per-crate counts,
    the rule, and the outcome. The rule: build the crate **iff** at least two crates outside `pii-store`
    duplicate at least three of those helpers; otherwise record the counts and "not worth it".
R3. If the rule fires, one crate owns the helpers: the extracted functions live in exactly one place, at least
    two crates are migrated, and no test loses coverage in the migration (the workspace's test count must not
    drop).
R4. The decision file names the residual duplication that is deliberately left alone, with the reason.

### Non-Functional

R5. The census is a script under `scripts/checks/`, not a hand-written table — so the next person can re-run it.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| the_census_reports_counts | the current workspace | one row per crate with a count, and the command that produced it |
| the_rule_is_applied | the census of today | the decision file states the outcome and the numbers |
| no_test_is_lost_when_helpers_move | (only if R3 fires) the migration | the workspace test count is >= the count before it |

## Exit Criteria

- [ ] `bash scripts/checks/test-helper-census.sh` exits 0 and prints one row per crate
- [ ] `_docs/05-operations/testkit-decision.md` exists and contains the numbers **and** the rule's outcome
- [ ] if the rule fired: `cargo test --workspace` passes and `grep -rn 'fn scratch_dir\|fn temp_store' crates --include=*.rs | wc -l` shows the helpers in one place only
- [ ] `cargo xtask fresh-gate` on this spec's commit is `ALL PASS`

## Guardrails

- Do NOT create the crate "for symmetry" or "because it might be useful": the rule in R2 decides, and a
  "not worth it" outcome with numbers is a PASS.
- Do NOT move a helper that is used once.
- Do NOT add a test-only dependency to a non-dev context.

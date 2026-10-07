# Spec: 74 — mutation-closure: the four partly-guarded fixes get their missing guards

**Project**: /projects/pi-improved
**Phase**: batch B (from 043 §3 mutation table — the residue the 09-18 review left open)
**Status**: NOT STARTED
**Derives from**: 043 §2 ("four of the eight spec fixes turn out to be only partly guarded")
+ §3 rows: spec 62 (call-site containment), spec 65 (non-hermetic + no layer read-back),
spec 66 (no message assertion), spec 68 (`& 0xff` dead code, no 128+ case)

## Overview

The September fixes are real but their TESTS don't prove them: reverting each fix still
leaves the suite green (proven by the review's mutation runs). A guarded fix dies loudly.
This spec adds the missing teeth — four independent requirements, one commit each acceptable.

## Requirements

### R1 — spec 62: containment enforced AT THE CALL SITE
The post-join containment check (`if false && ...`) exists only as a helper unit test. Add
a test on the CALLER path (worktree join flow in pii-server) that feeds a path-traversal
payload and asserts the join is REFUSED — reverting the call site must turn this test red.

### R2 — spec 65: hermetic layer tests + read-back
- The consolidate test must not depend on stale `crates/pii-memory/_tmp/phase3/cons-*.redb`
  files: create its store fresh in a tempdir (test-local).
- Add a pii-store test that WRITES with `MEM_LAYER` set and READS the layer back out.
Reverting either fix must red its new test.

### R3 — spec 66: the error message is asserted where it's constructed
One test (or extension of the constructor test) asserting the rendered message format at
representative call sites: op and resource appear in order. Reverting the swap must red.

### R4 — spec 68: a real 128+ exit-code case
`cmd_sandbox_exec`'s mask (`& 0xff` on `as u8` — dead code) is masked by the absence of any
case with exit ≥ 128. Add a case exercising code 255 (e.g. a command that exits 255) and
assert the reported code is 255. Document the `as u8` truncation reality: codes ≥ 256
cannot be represented; assert the DOCUMENTED behaviour (mask to u8), not an impossible one.

### R5 — guardrails
Test-file/store test additions only; no production behaviour change; no new deps; clippy
`-D warnings` + fmt clean; every new test MUST fail when its target fix is reverted
(the worker proves each with a scratch-export mutation, like 043 §3's method).

## TDD Contract (ACCEPTANCE)

| Test | Mutation (scratch export) | Expected |
|---|---|---|
| R1 caller-path traversal | `if false &&` restored at call site | RED |
| R1 contended control | pristine HEAD | GREEN |
| R2 fresh-store consolidation | MEM_LAYER insert dropped | RED |
| R2 layer read-back | same mutation | RED |
| R3 message shape | op/resource swapped again | RED |
| R4 exit 255 | mask changed to 0x7f | RED |

## Exit Criteria

- [ ] `cargo test -p pii-server --lib --bins --tests 2>&1 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-store --lib 2>&1 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-cli --bin pii 2>&1 | grep -q 'test result: ok'` (the cli bin's sandbox tests; no lib target exists — `p pii` matches nothing)
- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] worker's mutation-log for all four rows shows RED-on-mutation, GREEN-on-HEAD (file under `_tmp/`)

## Implementation Notes (worker)

- Copy 043 §3's mutation method (scratch exports via `git archive`, logs on disk) — that's
  the acceptance procedure, not ad-hoc greps.
- Where a "fix" turns out to be genuinely dead code (`& 0xff`), the honest outcome is
  documenting + pinning the real behaviour — not pretending the mask is load-bearing.
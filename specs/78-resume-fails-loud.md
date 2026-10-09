# Spec: 78 — close the resume fail-open (D4, 043 §10 item 4)

**Project**: /projects/pi-improved
**Phase**: batch C — 043 §6 D4 MED-HIGH.

## Context
`crates/pii-server/src/engine/runs.rs`:
- `:171` (also `:151-158`): `stored_run_scope_root(..).ok().flatten()` — a STORE error
  inside a resume path is flattened to "no scope" and the resume proceeds;
- the graph-lookup failure path gives an UNLIMITED budget (`map_or(u64::MAX)`).
A confined heal/promotion run whose scope or budget cannot be recovered resumes with
workspace-wide write + unbounded budget. Fail-open in exactly the place confinement lives.

## Requirements
R1. `stored_run_scope_root` errors: propagate (`?` or explicit `return Err`) — a resume
    that cannot READ its confinement never starts. The `None` (no scope recorded)
    case keeps its existing documented behaviour — only the Err path must fail loudly.
R2. Graph-lookup / budget recovery: on Err, fail the resume (no `u64::MAX` fallback).
    Only a genuinely recovered budget value proceeds.
R3 (TDD). Red-first tests using the store fault seam (the same seam spec 74 used):
    - resume with scope-store FAULT → returns Err, and NO run starts (assert no state
      transition happened);
    - resume with graph-lookup FAULT → returns Err (budget never defaults to unlimited).
R4. Full gate after (fmt/clippy/workspace tests). Report which files changed + the red
    evidence for both R3 tests.

## Exit Criteria
- [ ] `grep -n 'ok().flatten()' crates/pii-server/src/engine/runs.rs` is EMPTY
- [ ] `grep -n 'map_or(u64::MAX)' crates/pii-server/src/engine/runs.rs` is EMPTY (or shows the replaced bounded logic with a comment proving Err fails)
- [ ] `cargo test -p pii-server -q resume_fail 2>&1 | grep -c 'test result: ok'` at least 1 (the new fault tests exist + pass)
- [ ] `bash quality-gate.sh` exits 0 (full gate, includes the new tests)
- [ ] `cargo fmt --all --check` exits 0

## Guardrails
- Do NOT widen what a confined run may write — this spec only closes failure paths.
- Do NOT change run/scope wire formats, store schemas, or the redb layout.
- Do NOT commit; leave the tree for the orchestrator.
# Spec: 87 — reachability is stateful: test-only, string and comment references are not callers (N2)

**Project**: /projects/pi-improved
**Phase**: 045 re-review, candidate 3 — the check that is supposed to be the gate's caller detector.

## Context (measured by the reviewer, reproduced by the orchestrator)
`cargo xtask reachability-check` is satisfied by a reference that is not a caller. With the real
`pii_model::resolve_with_transport` caller muted in a scratch export, the check exits 1 (correct).
But it exits **0** — i.e. claims a real caller exists — when the only remaining reference is any of:
- `#[cfg(test)] mod X { fn f() { let _ = pii_model::resolve_with_transport("x", todo!()); } }`
- `#[cfg(test)] fn` , or `#[cfg(test)]` followed by `#[allow(..)]` and then `mod`
- a string literal containing the call
- a single-line or multi-line `/* */` block comment
- a multi-line raw string

`//`, `///` and `//!` lines ARE correctly ignored.

Root cause: `xtask/src/reachability.rs:178-186` — `effective_lines` calls the block-aware
`count_effective_lines(t)` on **one trimmed line at a time**, so `#[cfg(test)]` state can never
carry across lines, and string/comment stripping has no state either.

Also: the unit test `cfg_test_body_is_not_a_caller` is DECORATION — its fixture sits under
`declare_dir`, so it passes for the wrong reason and only goes red when the `declare_dir` skip is
disabled. No test catches disabling the `is_test_path` skip.

## Requirements
R1. `effective_lines` (or its caller) tracks state across the WHOLE file, at least:
    - inside a `#[cfg(test)]` item (mod, fn, or attribute stack before one), and
    - inside a `//`-line comment (already OK), a `/* */` block comment (single and multi-line),
    - inside a normal string literal, a raw string (`r#"..."#`), and a char literal.
    Lines inside those regions are NOT candidate caller lines.
R2. Keep the `declare_dir` rule (a match inside the declaring crate never satisfies the check) and
    the existing seed list; this spec changes only what counts as a line of code.
R3. Re-file the unit fixture: `cfg_test_body_is_not_a_caller` must FAIL when the cfg(test) skipping
    is disabled — i.e. its fixture must live OUTSIDE `declare_dir`. Add a unit test for
    `is_test_path` skipping too.
R4. TDD evidence: for each of the 8 stand-in variants listed above, produce the export run showing
    the check exits 1 with the real caller muted, and one showing HEAD exits 0. Record the commands
    and results (a script is fine; keep it under `_tmp/`).

## Exit Criteria
- [ ] `cargo test -p xtask 2>&1 | grep -c 'test result: FAILED' | grep -q '^0$'` and the reachability unit tests pass
- [ ] `cargo run -q -p xtask -- reachability-check` exits 0 on the current tree
- [ ] In a `git archive` export with the real caller muted AND each of the 8 stand-ins present, the check exits NON-ZERO (record all 8 runs + exit codes in the report)
- [ ] `grep -c 'cfg_test_body_is_not_a_caller' crates/xtask/tests/*.rs xtask/tests/*.rs 2>/dev/null | grep -v ':0' | wc -l` at least 1, and the report shows that test going RED when the cfg(test) skip is commented out
- [ ] a new unit test for the `is_test_path` skip exists and goes RED when that skip is disabled
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree` still prints `fresh-gate: PASS` (no regression)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0

## Guardrails
- Do NOT widen the seed list or relax any rule to make the check pass; the point is that it must FAIL
  on the stand-ins. A check that cannot fail is the defect.
- Do NOT touch `crates/` production code — `xtask/` only (plus its tests).
- Scratch exports under `/projects/_tmp/` only; the disk floor is enforced by fresh-gate, mind the
  space (one export ≈ 8G, and remove your own scratch when done).
- Do NOT commit; leave the tree for the orchestrator.
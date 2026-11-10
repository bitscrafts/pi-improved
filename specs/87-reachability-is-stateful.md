# Spec: 87 — reachability is stateful: test-only, string and comment references are not callers (N2)

**Project**: <repo>
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
- [x] `cargo test -p xtask` — no failing suite; the reachability unit tests pass, including the new `stateful_non_caller_shapes_are_rejected` (the 8 shapes + a positive case)
- [x] `cargo run -q -p xtask -- reachability-check` exits 0 on the current tree
- [ ] In a `git archive` export with the real caller muted AND each of the 8 stand-ins present, the check exits NON-ZERO (record all 8 runs + exit codes in the report)
- [ ] `grep -c 'cfg_test_body_is_not_a_caller' crates/xtask/tests/*.rs xtask/tests/*.rs 2>/dev/null | grep -v ':0' | wc -l` at least 1, and the report shows that test going RED when the cfg(test) skip is commented out
- [ ] a new unit test for the `is_test_path` skip exists and goes RED when that skip is disabled
- [x] both gates green (orchestrator-reproduced): `bash quality-gate.sh` → ALL PASS and `fresh-gate --worktree` → `quality-gate: ALL PASS` + `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0

## Guardrails
- Do NOT widen the seed list or relax any rule to make the check pass; the point is that it must FAIL
  on the stand-ins. A check that cannot fail is the defect.
- Do NOT touch `crates/` production code — `xtask/` only (plus its tests).
- Scratch exports under `<scratch>/` only; the disk floor is enforced by fresh-gate, mind the
  space (one export ≈ 8G, and remove your own scratch when done).
- Do NOT commit; leave the tree for the orchestrator.

## Orchestrator verification (2026-10-09)
- **The guard tests have teeth, proven by MUTATION (orchestrator)**: disabling the new
  `CfgTestGate::skips` (forcing `false`) turns `cfg_test_body_is_not_a_caller` and
  `stateful_non_caller_shapes_are_rejected` RED in the unit suite, and the export-based
  `cfg_test_body_is_not_a_caller` RED in `xtask/tests/spec72_reachability.rs`. Restored, re-verified
  green. This is the specific complaint the reviewer raised: the OLD fixture passed for the wrong
  reason (it sat under `declare_dir`) and nothing caught disabling the skip.
- Implementation read at the source: `blank_non_code` (whole-file lexer: `//`, nested `/* */`,
  strings, raw strings, char literals — newlines preserved) + `CfgTestGate` (attribute stack → item
  open → brace/branch depth), state carried across lines. `declare_dir` and the seed list unchanged.
- Integration suite: 4 passed (`reachability_fails_when_callers_deleted`, `reachability_passes_on_head`,
  `cfg_test_body_is_not_a_caller`, `comments_are_not_callers`).
- The 8-stand-in variant matrix in the worker's report (`_tmp/claude-batchC/s87-variants.sh`) shows the
  control pair I asked for: HEAD exits 0 for all 8 (the bug), the new code exits 1 for all 8, and both
  exit 1 with no stand-in. I did not re-run that 8-export matrix myself; I re-ran the mutation control
  and both gates, which is what establishes the fix rather than the fixture.

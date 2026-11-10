# Spec: 90 — the `doctest = false` comments stop overclaiming (N8)

**Project**: <repo>
**Phase**: 045 re-review, candidate 12 (docs half) — correcting a false claim this repo carries 18 times.

## Context
Batch A added `[lib] doctest = false` to 17 crates + xtask, each with a comment asserting the
shipped quality gate could never pass otherwise. The re-review PROVED that false:
`cargo test --doc` IGNORES the flag (a probe crate with a deliberately wrong doctest still failed
under `--doc`, exit 101), and `quality-gate.sh` runs its doc step as `--doc`. So:
- the flag cannot have affected `quality-gate.sh` at all;
- nothing was masked there (a future doctest in those crates is still compiled and run by the gate);
- its real effect is on PLAIN `cargo test` runs — spec exit criteria, `cargo test -p X`,
  `cargo test -j2` — where it avoids a cargo-level fingerprint problem in shared target dirs.

## Requirements
R1. Rewrite every such comment to state the true effect, in the shape:
    `# Zero doctests (verified); the flag affects plain \`cargo test\` only — an explicit
    # \`--doc\` (as the gate uses) ignores it. It avoids a cargo fingerprint problem in shared
    # target dirs; it is not a gate workaround.`
    Keep it to 2-3 lines per crate; do not remove the flag.
R2. There must be no remaining claim that the flag is needed for the gate to pass.
R3. Also fix the same overclaim where it appears in prose: `HANDOFF.md`'s batch-A row and any doc that
    repeats it (search, do not guess).

## Exit Criteria
- [x] `grep -rn 'can never pass' crates xtask --include='Cargo.toml'` → 0 (verified)
- [x] `grep -rln 'doctest = false' crates xtask --include='Cargo.toml'` → 17 files (16 crates + xtask; the spec said "17 crates" loosely — the worker flagged the wording, and the flag is intact everywhere)
- [x] `grep -rln -E 'explicit .*--doc|--doc.*ignores' crates xtask --include='Cargo.toml'` → 17 (verified **with `-E`**: the criterion as I wrote it used basic regex where `|` is literal — my criterion was wrong, not the work; the worker caught it and said so)
- [x] `grep -c 'can never pass' HANDOFF.md` → 0 (the E0463 bullet now says the claims "could not pass without it were FALSE (rewritten by spec 90)")
- [x] `cargo test -p xtask` 3 suites ok; `lint-files` 188 files, max 368 effective lines, exit 0
- [x] `bash quality-gate.sh` → ALL PASS (verified)
- [ ] the report lists every file touched

## Guardrails
- Comments and prose ONLY. Do NOT remove or change any `doctest = false` line itself, any dependency,
  or any code.
- Do NOT fix the 041 shared number: `000-TOPIC-MAP.md` T13 records it as a deliberate keep
  ("banners only, no renames"). The re-review's other numbering note (filing the report as 045) is
  already done.
- Do NOT commit; leave the tree for the orchestrator.

## Orchestrator verification (2026-10-09)
- Comment text verified in the tree (e.g. `crates/pii-abi/Cargo.toml`): 3 lines stating that the flag
  affects plain `cargo test` only, that an explicit `--doc` (which the gate uses) ignores it, and that
  it is not a gate workaround. The `doctest = false` lines themselves are untouched.
- 18 files changed, none outside `Cargo.toml`/`HANDOFF.md` (checked).
- Criterion 3 was mis-written by me (BRE `|` is literal); with `-E` it is 17. Noted rather than quietly
  fixed — the worker reported the discrepancy instead of "fixing" the check to match.
- Only the repo gate was re-run (comments cannot change compilation); the fresh-tree gate was left from
  spec 89's PASS on a tree that differs from this one only by comments.

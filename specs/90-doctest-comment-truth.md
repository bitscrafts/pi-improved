# Spec: 90 — the `doctest = false` comments stop overclaiming (N8)

**Project**: /projects/pi-improved
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
- [ ] `[ "$(grep -rn 'can never pass' crates xtask --include='Cargo.toml' | wc -l)" = 0 ]`
- [ ] `[ "$(grep -rln 'doctest = false' crates xtask --include='Cargo.toml' | wc -l)" -ge 17 ]` (the flag stays)
- [ ] `[ "$(grep -rn 'explicit .*--doc.*ignores it|--doc.*ignores' crates xtask --include='Cargo.toml' | wc -l)" -ge 17 ]` (each site states the true behaviour)
- [ ] `[ "$(grep -c 'can never pass' HANDOFF.md)" = 0 ]`
- [ ] `cargo run -q -p xtask -- lint-files` exits 0 and `cargo test -p xtask` exits 0
- [ ] `bash quality-gate.sh` exits 0 (comment-only change must not disturb the gate)
- [ ] the report lists every file touched

## Guardrails
- Comments and prose ONLY. Do NOT remove or change any `doctest = false` line itself, any dependency,
  or any code.
- Do NOT fix the 041 shared number: `000-TOPIC-MAP.md` T13 records it as a deliberate keep
  ("banners only, no renames"). The re-review's other numbering note (filing the report as 045) is
  already done.
- Do NOT commit; leave the tree for the orchestrator.
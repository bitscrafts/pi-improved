# Spec: 96 — kill the N1 class at the root: a scratch helper that creates its own directory

**Project**: /projects/pi-improved
**Phase**: 045 follow-through. The N1 class has now been found FOUR times (83 lanes, 85 heartbeat, 86 the
sweep, and now `engine/runs.rs`), and each time it was fixed per-file. This spec removes the shape.

## Context — why per-file fixes keep missing it
The pattern: `tempfile::Builder::new().tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"))` — a
temporary directory inside a crate-local `_tmp` directory that the TEST never creates. In a developer
tree the directory exists (left over from an earlier run) and everything is green; in a fresh clone, CI,
or a disposable worktree the tests panic with `NotFound`. It is order-dependent, which is why it keeps
slipping through: if ANY test in the same binary creates `crate/_tmp` first, the ones that don't need it
pass anyway.

Measured today (spec 95, orchestrator): `engine/runs.rs:472` and `:516` fail with
`NotFound … crates/pii-server/_tmp/heal-scope-zHiSmL` when that directory is absent, and pass when it is
present. ~31 files use `tempdir_in` with a `_tmp` path; several do not create the directory.

## Requirements
R1 (helper — the root fix). Introduce ONE sanctioned way for a test to get a scratch dir that guarantees
    its parent exists. Preferred: a small shared helper in an existing test-support location (e.g.
    `pii-store/src/testing.rs`, which the repo already uses for test seams) exposed as
    `tmp_scratch(prefix: &str) -> tempfile::TempDir` (or equivalent), which does `create_dir_all` on the
    crate-local `_tmp` then `tempdir_in`. If a per-crate copy is unavoidable for dependency reasons, say
    why in the report and keep the copies byte-identical.
R2 (sweep). Convert EVERY site that uses `tempdir_in(<something>_tmp…)` in `crates/` and `xtask/` to the
    helper (or to an explicit `create_dir_all` immediately before it where the helper cannot be used —
    say which and why for each). Report the full file list and the count of remaining bare sites.
R3 (enforcement — not a warning). Add a check that FAILS on a new bare site, in the repo's own gate:
    a cheap static check (extend `xtask lint-files` or add a sibling) that flags a `tempdir_in(` call
    whose argument names a `_tmp` directory and whose enclosing function has no `create_dir_all` /
    `tmp_scratch` before it. Wire it into `quality-gate.sh` as a failing step. It must be
    near-miss-tested: a deliberate re-introduction of the bare pattern in a scratch export must make the
    new step exit non-zero, and removing it must make it pass.
R4 (proof, two levels).
    - `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`.
    - The decisive control: in a `git archive` export, MOVE ASIDE (never delete) every `crates/*/_tmp`
      and `xtask/_tmp` directory, then run the gate's test step; it must be green with no `_tmp`
      directory present. Paste the commands and the result.

## Exit Criteria
- [ ] `fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] the no-`_tmp` control: with every `crates/*/_tmp` and `xtask/_tmp` moved aside in a fresh export, the workspace test step passes (verbatim output recorded)
- [ ] the acceptance record (`/projects/_tmp/spec96/SCRATCH-REPORT.md`) lists every converted file and every remaining bare `tempdir_in(..._tmp...)` site with the reason it stays
- [ ] the new lint step exists in `quality-gate.sh`, runs, and its teeth are proven: re-introducing the bare pattern in a scratch export makes it exit non-zero
- [ ] `bash quality-gate.sh` exits 0 with the new step present
- [ ] `cargo test --workspace --lib --bins --tests` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0 and `cargo fmt --all --check` exits 0
- [ ] no file passes the lint-files warn threshold (report the output)
- [ ] the report states the count of `_tmp`-scratch sites before and after

## Guardrails
- Do NOT delete anything: in an export, `mv` a `_tmp` directory aside; never `rm -rf` a path that existed
  before your run.
- Do NOT weaken a test's assertions while converting it — the conversion is about where the directory
  comes from, not about what the test checks.
- Do NOT add a global `_tmp` directory creation to `quality-gate.sh` as the fix: that would hide the
  class again (the point is that TESTS create what they need).
- Scratch exports under `/projects/_tmp/` only; delete your OWN exports when done (one ≈ 8G).
- Do NOT commit; leave the tree for the orchestrator.

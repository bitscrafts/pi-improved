# pi-improved — spec-61: fix hardcoded `_tmp` absolute paths + relocate orphaned docs

- **Project**: `<repo>`
- **Priority**: MEDIUM — repo hygiene, makes the codebase easier to
  review (explicitly requested ahead of an opus review pass). Two
  independent findings bundled because both surfaced from the same
  session-end audit:

  1. **50 hardcoded absolute-path references** to
     `_tmp` (or subpaths of it) across ~25 test
     and source files. These break the moment the repo is checked out
     anywhere else, or the container's mount path changes — the EXACT
     class of bug that caused a real test failure tonight
     (`crates/pii-server/tests/reattach.rs` hardcoded `_tmp/phase2` as
     a load-bearing directory; when stale scratch under `_tmp/` was
     archived elsewhere for disk cleanup, the test broke because the
     directory no longer existed — already fixed ad hoc for THAT one
     file, but the same problem exists in 24 more).
  2. **3 orphaned markdown files** at `_tmp/workhorse-spec-violations/
     05/06/07-*.md` — genuinely valuable historical Phase-1 planning
     specs (verified substantive, not throwaway — read one yourself
     before assuming otherwise), sitting inside a `.gitignore`d path
     yet somehow still git-tracked (a pre-existing contradiction from
     an earlier session, commit `83d41b2`), linked from nowhere, findable
     by nobody. They belong in `_docs/99-archive/` per this project's
     own SSOT documentation convention (see `AGENTS.md`'s `_docs/`
     layout description), not inside `_tmp/`.

## Overview

### Part A: hardcoded `_tmp` paths

Ground truth (re-verify the exact current list yourself — this is a
snapshot from `grep -rn '"_tmp' crates/*/tests/
*.rs crates/*/src/*.rs`, run once, do not assume it is unchanged):

Two distinct patterns exist, needing different fixes:

- **Pattern 1 — bare `tempdir_in("_tmp")`** (the
  majority, ~20 sites across `pii-server`, `pii-model`, `pii-mcp`,
  `pii-store`): these create a NEW tempdir inside `_tmp` itself, so
  they only need `_tmp` to exist (which it always does, gitignored but
  present) — but the absolute path is still wrong for portability.
  Fix: replace the literal string with a `CARGO_MANIFEST_DIR`-relative
  expression, e.g. `concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp")`
  (the exact relative depth — `../..` — depends on how deep the crate
  is nested under `crates/`; verify per-file, don't copy-paste blindly).
  This exact pattern is already established in this session's own
  `crates/pii-store/tests/mem_store_contract.rs` (spec-53) and
  `crates/pii-server/tests/reattach.rs` (fixed ad hoc tonight) — follow
  those two files as the reference implementation.
- **Pattern 2 — hardcoded SUBDIRECTORY paths** (e.g.
  `"_tmp/phase3/persist-test.redb"`,
  `"_tmp/pi-store-test"`, `"
  _tmp/phase3/scheduler-it"`): these assume a specific named
  subdirectory ALREADY EXISTS, which is exactly what broke tonight.
  Fix: same `CARGO_MANIFEST_DIR`-relative base, PLUS an explicit
  `std::fs::create_dir_all(...)` before first use if the code doesn't
  already ensure the directory exists (some sites may already do this
  via `tempdir_in`'s own directory-creation, in which case nothing
  extra is needed — check each site, don't blanket-add
  `create_dir_all` where it's redundant).

Full file list from tonight's grounding grep (re-verify counts per
file before starting — this may have shifted): `pii-memory/tests/
{persistence,consolidate,lanes}.rs`, `pii-server/tests/{slice_e_live,
peercred,failover_graph,scheduler,fail_port,approvals,channels,
memory_rpc,latency,slice_f_promotion,model_routing,session,handshake,
cancellation_brake}.rs`, `pii-server/src/engine/runs.rs`,
`pii-server/src/node_kinds/nodes/{gate,heal}.rs`,
`pii-server/src/node_kinds/agent/tests.rs`,
`pii-server/src/daemon/scheduler.rs`, `pii-model/tests/ollama_http.rs`,
`pii-model/src/lib.rs`, `pii-mcp/src/main.rs`, `pii-store/tests/
store_contract.rs`, `pii-store/src/lib.rs`, `pii-store/src/
{channel,timer}.rs`, `pii-store/src/heal.rs`.

### Part B: relocate orphaned docs

Move the 3 files from `_tmp/workhorse-spec-violations/` to
`_docs/99-archive/workhorse-spec-violations-2026-09/` (a new
subdirectory — keep the 3 files together as a named group since they
document a single historical episode, per the commit message that
originally exiled them: "remove workhorse-authored 05/06/07 (planner
owns specs/); keep 03 as source of truth"). Add ONE line to
`_docs/99-archive/`'s existing index (check whether `_docs/99-archive/`
has its own README/index file, or whether `_docs/INDEX.md` is the
single point of reference — follow whichever convention already
exists, do not invent a new one) pointing at the new location with a
one-line description of what it is (historical evidence of a workhorse
spec-authoring violation from an earlier session, kept for reference).

## Requirements

### Functional

- R1. For every Pattern-1 site (bare `tempdir_in("
  _tmp")`): replace with the `CARGO_MANIFEST_DIR`-relative form.
- R2. For every Pattern-2 site (hardcoded named subdirectory): replace
  the base path the same way, AND ensure the subdirectory exists before
  use (via `create_dir_all` if not already handled).
- R3. Move the 3 files from `_tmp/workhorse-spec-violations/` to
  `_docs/99-archive/workhorse-spec-violations-2026-09/` using `git mv`
  (they are git-tracked; a plain filesystem move would show as
  delete+add instead of a rename in the diff).
- R4. Add a one-line reference to the relocated files in whichever
  index/README file governs `_docs/99-archive/` per the existing
  convention (see Overview Part B).
- R5. Remove the now-empty `_tmp/workhorse-spec-violations/` directory
  if `git mv` leaves it empty (it should, since `_tmp/` itself is
  gitignored, so the directory's continued existence or non-existence
  on disk doesn't matter to git — but check there's nothing else in it
  first).

### Non-Functional

- N1. Every path fix must be verified by actually running that specific
  test/crate after the change — do not do a blind find-and-replace
  across 25 files and assume it compiles/passes. Run `cargo test -p
  <crate>` for each touched crate as you go.
- N2. Do NOT change any test's actual assertions/behavior — only the
  path construction.
- N3. Do NOT touch `crates/pii-store/tests/mem_store_contract.rs` or
  `crates/pii-server/tests/reattach.rs` — both already fixed (in an
  earlier pass tonight and via spec-53 respectively) and already use
  the correct pattern; they are the REFERENCE for this spec, not part
  of its scope.

## Architecture

```
Before:
  crates/*/tests/*.rs, crates/*/src/*.rs   (50 hardcoded absolute paths)
  _tmp/workhorse-spec-violations/05,06,07  (orphaned, gitignored-but-tracked)

After:
  crates/*/tests/*.rs, crates/*/src/*.rs   (CARGO_MANIFEST_DIR-relative)
  _docs/99-archive/workhorse-spec-violations-2026-09/05,06,07  (via git mv)
```

## TDD Contract

No new tests — this spec is a portability/hygiene fix. The "test" is
that every touched crate's existing test suite still passes after the
path-construction change (see Exit Criteria).

## Exit Criteria

- [ ] `! grep -rn '"/projects/pi-improved/_tmp' crates/*/tests/*.rs crates/*/src/*.rs`
- [ ] `[ ! -d _tmp/workhorse-spec-violations ] || [ -z "$(ls -A _tmp/workhorse-spec-violations 2>/dev/null)" ]`
- [ ] `[ -f _docs/99-archive/workhorse-spec-violations-2026-09/05-phase1-run-engine-part1-journal-caps-resume-groundwork.md ]`
- [ ] `[ -f _docs/99-archive/workhorse-spec-violations-2026-09/06-phase1-run-engine-part2-concurrent-dispatch.md ]`
- [ ] `[ -f _docs/99-archive/workhorse-spec-violations-2026-09/07-phase1-run-engine-part3-crash-resume.md ]`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`

## Guardrails

- G1. Do NOT change any file's PRODUCTION logic — only path
  construction in test setup code (and in the 4 non-test `src/*.rs`
  sites listed, where the hardcoded path appears inside their own
  `#[cfg(test)]` modules — verify each is genuinely test-only code,
  not a real runtime default, before touching it).
- G2. Use `git mv` for the doc relocation, not a plain filesystem move —
  preserve rename history in the diff.
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. If any site turns out to have a REAL reason to stay hardcoded
  (e.g. genuinely testing an absolute-path-specific behavior, not just
  using it as a scratch location), STOP and report it rather than
  forcing an inappropriate fix.

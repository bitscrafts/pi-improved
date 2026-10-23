# Spec: 245 — a commit builds at its own HEAD

**Project**: /projects/pi-improved
**Phase**: filed 2026-10-23T12:40Z by the orchestrator (defect 24 in `_docs/05-operations/agent-management-findings-2026-10-23.md`).
Premises read, not assumed:

- HEAD `1675120` does **not** compile. Measured on a detached worktree of that commit (clean, `git status` empty):
  `cargo check -q -p pii-server` reports **7 errors**:
  - `E0425` `cannot find function kill_group in crate pii_core` — `childproc.rs:380` declares it `fn kill_group`, and only the
    working tree has `pub fn kill_group` (`childproc.rs:383` + the re-export in `pii-core/src/lib.rs`);
  - `E0432` `unresolved import crate::node_kinds::nodes::coder_runtime::Listed`, `…::coder_runtime_api`, `super::coder_runtime_api`;
  - `E0599` `no method named list_all` / `force_remove` for `&dyn ContainerRuntime`;
  - `E0050` `method run has 4 parameters but the declaration in trait ContainerRuntime::run has 5`.
- The same files are `M`/`??` in the working tree, so the tree builds while its HEAD does not. Specs 241 (`85602d2`), 242
  (`1655e2d`), 243 (`8cc8ffe`) committed the call sites; the API they call was left uncommitted and their commit path lists
  never named it.
- The loop's gate step runs `bash quality-gate.sh --routine` (repo tree, warm cache — `cycle.sh:321-328`), so a green gate
  says "this working tree builds", never "this commit builds". `cargo xtask fresh-gate` exports HEAD and would have failed.
- The commit itself stages an explicit path list (`cycle.sh` column 6), so "the gate saw it" and "the commit carries it" are
  two different sets, and nothing compares them.

## Overview

Two facts have to become one: **the tree the gate measured and the commit that lands must be the same tree**, and a commit
that forgets a file it depends on must fail loudly instead of landing as done. This spec restores the API HEAD is missing,
adds a compile-time surface test that names it through the public path, and adds the missing check — a build of HEAD in a
throwaway export — as a post-commit step whose failure marks the item not-done.

## Requirements

- **R1** A clean export of HEAD compiles the workspace: `pii_core::kill_group` is public (or the caller stops calling it),
  `ContainerRuntime` declares `run` with the timeout parameter its implementors have, `list_all` and `force_remove` exist on
  the trait, `Listed` is exported where `orphans.rs` imports it, and the `coder_runtime_api` module the importers name
  resolves. Prefer the smallest set that makes the *committed* callers correct; do not restructure the trait.
- **R2** A new test file `crates/pii-server/tests/coder_runtime_surface.rs` names each of those through their public path —
  a stub implementor of `ContainerRuntime` that overrides `run`(with timeout), `list_all` and `force_remove`, and a call to
  `pii_core::kill_group` — so that removing any of them fails to compile at the caller's side, not only here. The test body
  asserts the stub's calls return what the trait promises (no `assert!(true)`).
- **R3** `scripts/autonomy/head-builds.sh` builds **HEAD's content** in a throwaway export and reports one line:
  - `head-builds.sh` (no argument): export `git archive HEAD` into a directory it creates with `mktemp -d` under `_tmp/`,
    run `cargo check -q --workspace` there with a pinned `CARGO_TARGET_DIR` under `_tmp/`, print
    `HEAD <short-sha> builds` (exit 0) or `HEAD <short-sha> does NOT build` plus the first error lines (exit 1).
  - `head-builds.sh --worktree`: the same, after overlaying the modified and untracked-not-ignored files
    (`git ls-files -m -o --exclude-standard`) — the pre-commit door, because the plain door can never see an uncommitted fix
    (spec 236).
  - No network, no `claude`, no gate: `cargo check` only, so it costs minutes and can run after every commit.
- **R4** The cycle calls it **after** the commit: on a non-zero reading the outcome is `head-broken` (exit 15), the row is set
  `needs-review` with `last=head-broken after <sha>` naming the first missing symbol, and the item is **not** reported
  `done`. A green reading prints `head-builds: HEAD <sha> builds` into the cycle log. The routine `quality-gate.sh --routine`
  reading stays as it is — it is still the step verdict, but it can no longer be the only evidence a commit is sound.
- **R5** The check itself is proven by a control: `scripts/autonomy/selftest.sh` gains rows that run `head-builds.sh` against
  a two-commit fixture repository under `_tmp/` — one commit that builds (GREEN, exit 0) and one that does not (RED, exit 1,
  the error named) — so a check that always passes or always fails is caught.

## Architecture

`head-builds.sh` is the only new moving part; it is deliberately *not* `fresh-gate`, which runs the eight-step gate and costs
45 minutes on this host. It answers one question in a few minutes: does the content of this commit compile? The export goes
through `git archive` (the same door `fresh-gate` uses, so an ignored file cannot sneak in), and the worktree overlay is
`git ls-files -m -o --exclude-standard` piped through `tar` — no `rsync` dependency, no copy of `.git`.

## TDD Contract

| test | given | expects |
|---|---|---|
| `coder_runtime_surface` (new file, `--test`) | a stub implementor of `ContainerRuntime` | overrides `run` (with timeout), `list_all`, `force_remove`; the calls return the trait's promises |
| `head-builds.sh` selftest row: a building commit | a fixture repo whose `crates/x` compiles | exit 0, line matches `^HEAD [0-9a-f]+ builds$` |
| `head-builds.sh` selftest row: a broken commit | the same repo plus a caller of a symbol that commit does not export | exit 1, line matches `does NOT build`, the missing symbol named (CONTROL for the check itself) |
| `head-builds.sh --worktree` over an uncommitted fix | broken HEAD + the fix only in the working tree | exit 0 (the overlay door sees it) — proves the pre-commit door is not the plain door |

The row a plausible implementation gets wrong is the last one: using the plain door for the pre-commit verify makes every
item whose fix is uncommitted read `head-broken`, and the loop parks correct work.

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test coder_runtime_surface 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `bash scripts/autonomy/head-builds.sh --worktree`
- [ ] `cargo check -q --workspace`
- [ ] `bash -n scripts/autonomy/head-builds.sh && grep -q "head-builds" scripts/autonomy/cycle.sh`
- [ ] `cargo clippy -q -p pii-server -p pii-core --all-targets -- -D warnings`

## Guardrails

- `specs/` are READ-ONLY for the implementer. If `ContainerRuntime`'s shape or the `orphans.rs` import path is not as this
  spec describes, STOP and report — do not invent a new trait.
- **Do not sweep another lane's work.** If the working tree already carries uncommitted changes to the files R1 names (seen
  2026-10-23: a module-split refactor in `coder_runtime*.rs`, `engine/runs.rs`, `pii-server/src/lib.rs`), the API half may
  already be satisfied there: read a `head-builds.sh --worktree` reading first, and if it is green, say so in the Closure and
  implement only R2-R5. Never `git add` a path another session is editing, and never commit a whole file to pick up a hunk.
- Editing `scripts/autonomy/cycle.sh` is permitted **only** if no cycle holds `_tmp/autonomy/lock` at that moment
  (`flock -n … -c true` returns 0) and the change is additive; a running cycle executing a rewritten script dies silently
  (DO-NOT-REPEAT 2.15).
- `head-builds.sh` may remove only the `mktemp -d` directory it created in the same run, and only after printing its verdict.
  Nothing else is deleted; anything to be kept moves aside with a date.
- Tests write under `_tmp/`, never `/tmp`. 450-effective-line file gate and `#![forbid(unsafe_code)]` apply. Do not commit.
- No `claude` call, no network, no spend: `head-builds.sh` runs `cargo check` only.

## Closure

**Orchestrator's independent reading, 2026-10-23T18:17–18:22Z** — tree at HEAD `7172770`, carrying this spec's
WIP (`M scripts/autonomy/selftest.sh`, `?? scripts/autonomy/head-builds.sh`, `?? crates/pii-server/tests/coder_runtime_surface.rs`):

| # | criterion | reading |
|---|---|---|
| 1 | `cargo test -q -p pii-server --test coder_runtime_surface` | **PASS** — `running 2 tests … test result: ok. 2 passed; 0 failed`, rc 0 |
| 2 | `bash scripts/autonomy/head-builds.sh --worktree` | **PASS** — `HEAD 7172770 builds`, rc 0 |
| 3 | `cargo check -q --workspace` | **PASS** — rc 0 |
| 4 | `bash -n scripts/autonomy/head-builds.sh && grep -q "head-builds" scripts/autonomy/cycle.sh` | **FAIL** — rc 1: `cycle.sh` contains no reference to `head-builds` at all |
| 5 | `cargo clippy -q -p pii-server -p pii-core --all-targets -- -D warnings` | **PASS** — rc 0 |

So R1, R2, R3 and R5 are delivered and measured; **R4 is not implemented** — the post-commit check exists as a
script and is called by nobody, which is the exact shape this spec was filed to remove (a green reading that
nothing enforces). The row stays `needs-review`.

`scripts/autonomy/selftest.sh` was run on this tree: **40 passed, 0 failed / VERDICT: ALL MECHANISMS PASS**,
including all four new `head-builds.sh` rows — the fixture's building commit exits 0 with `^HEAD [0-9a-f]+ builds$`,
a commit calling `missing_symbol()` exits 1 and names the symbol, the plain door cannot see an uncommitted fix
(exit 1) while `--worktree` overlays it (exit 0). Both controls fire, so the check is not one that always passes.

**Finding owed by the next pass:** criterion 4 is a `grep`. It certifies that a call exists, never that the
`head-broken` branch fires, that exit 15 is returned, or that the row is left `needs-review` instead of `done`.
R4 owes a reading of the **branch**, taken through the real `cycle.sh` — not a re-enactment of its lines.

# Spec 278: the build identity follows the branch, not only HEAD and the index

## Overview
**Project**: /projects/pi-improved
**Phase**: production — deploy identity. The daemon's `health` answers `revision`, and the deploy card and my own freshness checks quote it (`crates/pii-server/src/daemon/identity.rs:262`, `REVISION = env!("PII_GIT_REV")`). On 2026-10-25 a daemon refresh built at HEAD `b8358f7` and answered `222f4debf542-dirty`: the PREVIOUS commit. The binary was current (it held strings only spec 276 adds); only the label lied. A label that can be one commit behind makes "which build is this?" unanswerable by the very field built to answer it.

Reading `crates/pii-server/build.rs:43-45`, the script tells cargo to rerun only when `../../.git/HEAD`, `../../.git/index` or `build.rs` change. On a normal checkout `.git/HEAD` holds `ref: refs/heads/master` and a commit rewrites `refs/heads/master`, a file the script does not watch, so a commit that leaves the index untouched never reruns it and the revision cached by the last build is baked in again.

## Verified Premises
- **CONFIRMED** (read 2026-10-25) `build.rs` emits `rerun-if-changed` for exactly `../../.git/HEAD`, `../../.git/index` and `build.rs`; `.git/HEAD` here is `ref: refs/heads/master`; the repo has no `packed-refs`; the revision is `PII_BUILD_REVISION` when set and non-empty, else `git rev-parse --short=12 HEAD`, suffixed `-dirty` when `git status --porcelain --untracked-files=no` is non-empty, and `unknown` with no git.
- **NOT YET MEASURED** that the stale label really comes from the unwatched branch ref. The first test below is that measurement: it must FAIL on today's `build.rs`. If it passes on today's file, the cause is something else: STOP and report it, do not "fix" anything.
- **NOT CLAIMED**: that the `-dirty` suffix is current. It is computed when the script runs, and an edit to a tracked file does not rerun the script, so `-dirty` can be stale in BOTH directions. That is a separate defect with a separate fix (a build that re-reads status on every build) and is a follow-up here, not this spec.

## Requirements
### Functional
- **R1. Watch the resolved branch ref.** In `crates/pii-server/build.rs`, read `../../.git/HEAD`. When its text starts with `ref: `, emit `cargo:rerun-if-changed=../../.git/<the ref path>` for that ref (the text after `ref: `, trimmed). When HEAD is a bare commit id (detached) emit nothing extra: HEAD itself changes then. Emit `cargo:rerun-if-changed=../../.git/packed-refs` ONLY when that file exists. No new dependency; the existing three lines stay; the revision logic, `PII_BUILD_REVISION` and the `unknown` path are untouched. A ref path containing `..` or starting with `/` is ignored, never emitted.
- **R2. Tests, with controls.** New `crates/pii-server/tests/build_revision_follows_commits.rs`. Each test makes a scratch repo with the layout the script assumes (`<root>/.git`, `<root>/crates/pii-server/{build.rs,Cargo.toml,src/main.rs}`), where `build.rs` is a COPY of the live one (read it with `include_str!("../build.rs")` so the test cannot drift from the file it checks), and `main.rs` prints `env!("PII_GIT_REV")`. The scratch crate has no dependencies and builds offline with a scratch `CARGO_TARGET_DIR`, with `PII_BUILD_REVISION` removed from the child's environment. Cases: (a) THE BUG: commit once, build, assert the printed revision starts with that commit's 12-hex; then `git commit --allow-empty` (this moves the branch ref and touches neither `.git/HEAD` nor `.git/index`), build again WITHOUT any other change, and assert the printed revision starts with the NEW commit's 12-hex and differs from the first; (b) CONTROL: a commit made the ordinary way (a changed file, `git add`, `git commit`) is also followed; (c) CONTROL: `PII_BUILD_REVISION=abc123` set for the child still wins over git; (d) CONTROL: a scratch tree with no `.git` prints `unknown` and still builds; (e) CONTROL: after `git pack-refs --all` the next empty commit is still followed; (f) the ref-path guard, as a unit test of the function that turns HEAD's text into a watch path (put that function in a tiny module the build script and the test both include via `#[path]`, or inline it in the test as a copy ONLY if a comment says the two must match and a test asserts the live `build.rs` text contains the same guard strings `..` and `/`): `ref: refs/heads/master` gives `refs/heads/master`, a detached id gives nothing, `ref: ../x` and `ref: /etc/x` give nothing.

### Non-Functional
- Every file under the 450-line gate. No new dependency in any `Cargo.toml` of the workspace. Do not edit `identity.rs`, any other test, any script, or any crate except `pii-server`'s `build.rs` and the one new test.

## TDD Contract
| Test | Expected |
|---|---|
| `an_empty_commit_changes_the_baked_revision` | new commit's 12-hex, different from the first |
| `CONTROL_an_ordinary_commit_is_followed` | likewise |
| `CONTROL_an_explicit_revision_still_wins` | `abc123` |
| `CONTROL_no_git_reports_unknown` | `unknown` and a built binary |
| `CONTROL_packed_refs_are_followed` | new commit after `pack-refs` |
| `the_ref_path_guard` | the four cases of R2 f |

**The requirement a plausible implementation satisfies only in appearance:** adding a `rerun-if-changed` on the branch ref and testing only an ordinary commit. An ordinary commit already rewrites the index, so it passes on today's script and proves nothing. Test (a) uses a commit that moves nothing but the ref.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test build_revision_follows_commits 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `grep -q "packed-refs" crates/pii-server/build.rs`
- [ ] `grep -q "ref: " crates/pii-server/build.rs`
- [ ] `grep -q "PII_BUILD_REVISION" crates/pii-server/build.rs`
- [ ] `cargo test -q -p pii-cli --test deploy_card 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `git diff --quiet -- crates/pii-server/src Cargo.toml Cargo.lock`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit `identity.rs` or add a dependency. Do NOT change how the revision or the `-dirty` suffix is computed. Do NOT commit. If the first test passes on today's `build.rs`, STOP and report it.

## Follow-ups (not this spec)
1. A `-dirty` suffix that is current: the script reruns only on the watched paths, so an edit to a tracked file leaves it stale in both directions.
2. The deploy check and the freshness rule reading the label: until the label is trusted they verify by content (a string only the new commit adds).

# 182 — the two files that block the full gate

## Overview
MEASURED 2026-10-18 by the orchestrator on a clean tree (`git status --porcelain` empty, `dbe600a`):

```
cargo run -q -p xtask -- lint-files        -> rc=1
error: crates/pii-server/src/daemon/mod.rs: 468 effective lines exceeds 450
error: crates/pii-cli/src/cmd_daemon.rs:   461 effective lines exceeds 450
```

The 450-line rule is workspace-wide (`Cargo.toml:56`) and `lint-files` is a step of `gate-steps.txt`, the **commit
and release** boundary. The routine gate the loop runs per item uses `gate-steps-quick.txt` (fmt, clippy, build),
which does not include `lint-files` — so items closed green while the full gate was red, and no deploy card can
quote a `mode=fresh` PASS on this HEAD. Both files grew past the limit in today's commits: `crates/pii-cli/src/cmd_daemon.rs`
in `7f98624` (175) and `crates/pii-server/src/daemon/mod.rs` in `03e729a` (a cycle's leftover commit).

**Project**: <repo>

## Requirements
1. Reduce both files **below** the limit — `crates/pii-server/src/daemon/mod.rs` (468) and
   `crates/pii-cli/src/cmd_daemon.rs` (461) — by moving cohesive items into modules, not by deleting code,
   merging unrelated functions, or changing behaviour.
2. No public API change: every `pub` item that is reachable today stays reachable at the same path (a `pub use`
   re-export is acceptable; a rename is not).
3. The crates' test suites keep their counts: `pii-server`'s lib tests and the daemon test binaries must run the
   same number of tests after the split as before it, and both keep passing.
4. `cargo run -q -p xtask -- lint-files` returns 0 for the whole workspace.

## TDD Contract
- **RED (already red, measured):** `lint-files` exits 1 naming the two files and both counts. Record the two
  counts and the test counts you measured before touching anything.
- **GREEN:** `lint-files` exits 0; the same test counts pass.
- A split that lowers the count by removing tests or by making `rustfmt` reflow comments into code is not green:
  the criterion is the same tests, the same behaviour, fewer effective lines.

## Exit Criteria
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `cargo test -p pii-server --lib`
- [ ] `cargo test -p pii-server --test daemon_graphs`
- [ ] `cargo test -p pii-cli --test deploy_card`

## Guardrails
- Do NOT raise the 450-line limit, add a per-file exemption, or move `lint-files` out of `gate-steps.txt`.
- Do NOT delete or weaken a test to make the count drop; `pii-server --lib` must report at least the number it
  reports today (129 as of `c4f5770`), and the split may add tests but not remove them.
- Do NOT change behaviour: this item is a mechanical split. If a block cannot be separated without changing a
  signature that callers outside the crate use, **STOP** and report which one — do not redesign it here.
- Do NOT touch `specs/`.

## Closure
(to be filled by the orchestrator: `lint-files` before/after with the two counts, the test counts before/after,
and the full `gate-steps.txt` run over the tree the worker leaves.)

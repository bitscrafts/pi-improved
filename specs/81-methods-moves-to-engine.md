# Spec: 81 — `daemon/methods.rs` moves to `engine/methods.rs` (Decision 2 follow-up)

**Project**: <repo>
**Phase**: batch D — 043 §10 item 12.

## Context
043 Decision 2 moved the engine modules under `engine/`, but `methods.rs` stayed in
`daemon/` while its callers live in `engine/` — `crates/pii-server/src/engine/` carries
14 `crate::daemon` references, all of them for `methods`. The layering is inverted:
the engine's dispatch table is a daemon module.

## Requirements
R1. `git mv crates/pii-server/src/daemon/methods.rs crates/pii-server/src/engine/methods.rs`
    (history preserved — no copy+delete).
R2. Update every reference: `crate::daemon::methods` → `crate::engine::methods` (use
    `crate::engine::methods` or a crate-relative re-export, whichever the module's own
    callers already use), the `mod`/`pub mod` declaration, and any doc link or comment
    naming the old path. `grep -rn 'crate::daemon' crates/pii-server/src/engine` must be
    EMPTY afterwards.
R3. Keep `daemon/mod.rs` compiling: if it re-exported `methods`, the re-export must
    either point at the new location or be removed with its callers updated — say which
    in the report.
R4. No behaviour change: no method added, removed, renamed or reordered; the dispatch
    table's content is identical (prove with a diff that shows only path changes).

## Exit Criteria
- [ ] `[ "$(grep -rn 'crate::daemon' crates/pii-server/src/engine | grep -v '^\s*//' | wc -l)" = 0 ]`
- [ ] `test -f crates/pii-server/src/engine/methods.rs && echo present` prints present
- [ ] `git log --follow --oneline -1 crates/pii-server/src/engine/methods.rs` shows the file's pre-move history (proves `git mv`, not a re-create)
- [ ] `cargo test -p pii-server 2>&1 | grep -c 'test result: FAILED' | grep -q '^0$'`
- [ ] `cargo build --workspace` exits 0
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --all --check` exits 0

## Guardrails
- PURE MOVE. Do NOT refactor, reformat, rename functions, or split the file.
- Do NOT touch `_repos/`, `specs/`, or any other module's location.
- Do NOT commit; leave the tree for the orchestrator.
- Report the exact set of files you changed and the `git diff --stat` shape (rename
  detection should collapse it to one rename + N one-line path edits).
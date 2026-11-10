# Spec: a graph writes and updates the books

## Overview

The two manuals exist and are stamped with the revision they describe. They are now the largest
hand-maintained artifact in the repository, and they go stale the moment a chapter's subject
changes: a command gains a flag, a graph gains a node, a crate gains a module, and the chapter that
documents it keeps its old text while its title page still claims to describe `4f291c5`.

This row makes updating them a **graph**, composed from what already exists rather than a third
documentation mechanism: `graphs/docs.toml` is already the documentation coder and
`graphs/doc-code.toml` is already `inventory → select → call docs → verify → done | unverified`.
The books are the same shape with a different inventory: instead of "which files are undocumented",
the question is "which chapters no longer match the code".

**Project**: <repo>
**Phase**: composability

## Premises (measured 2026-11-04)

- `graphs/docs.toml`: `name = "docs"`, `exit = "docs"`, `caps = ["spawn", "proc"]`, one `coder` node.
- `graphs/doc-code.toml`: `exit = "done"`; nodes `inventory` (tool) → `select` (route, ports
  `nothing-to-do` | `write`) → `call docs` → `verify` (tool) → `pass` | `fail` → `done` |
  `unverified`, with a `refuse` node.
- The engine has **no iteration primitive**: a graph cannot "do this call once per chapter"
  (`graphs/COMPOSABILITY.md`, "What composition cannot do"). Repetition today lives inside a node.
- The books: `_docs/manuals/src/user-admin/src/chapters/` (9 files) and
  `_docs/manuals/src/arch-dev/src/chapters/` (11 files), one chapter per file, and a title page
  carrying `version 0.2.0 · build <date> · commit <sha>`.
- The commit stamp is written by hand at build time; nothing checks that a chapter still matches it.

## Requirements

- **R1 — the inventory is a pii verb, not a script.** `pii docs stale --book <user-admin|arch-dev>`
  prints one line per chapter whose stamp commit differs from HEAD, or whose documented paths
  changed since that commit. Exit 0 with output, 0 with `nothing-stale`, and non-zero on an unknown
  book. The logic lives in the binary (product rule: no companion scripts).
- **R2 — the graph is a composition.** `graphs/books.toml`:
  `inventory → select -(nothing-to-do)-> done`; otherwise `call docs → verify → done | unverified`,
  `exit = "done"`, and the call to `docs` is **pinned** by sha256. No second documentation coder.
- **R3 — verify proves claims, not presence.** Every backticked path a chapter names must exist in
  the repository, and every `pii <verb>` it names must be a verb the binary answers. A chapter that
  names a deleted file or an invented flag fails, whatever else it says.
- **R4 — the stamp moves only on a pass.** `verify` passing rewrites the chapter's stamp commit;
  `verify` failing leaves the old stamp untouched, so the chapter stays visibly stale.
- **R5 — the ceiling holds.** `books` holds `fs_read`, `fs_write`, `proc`, `spawn`; the `call` to
  `docs` grants only what `books` grants, and a host edge that widens it is refused (control).

## TDD contract

| Test | Input | Expected |
|---|---|---|
| `books_graph_lints` | `pii graph lint graphs/books.toml` | rc 0; `exit` present; the docs call pinned |
| `stale_inventory_names_a_chapter` | a chapter whose stamp is older than a touched path | that chapter printed |
| `nothing_stale_is_a_port_not_a_failure` | a tree where every chapter matches | `nothing-to-do`, run ends `done` |
| `a_false_claim_fails_verify` | a chapter naming `crates/does-not-exist.rs` | `unverified`, stamp unchanged |
| `the_call_cannot_widen` | host caps without `fs_write` | load refuses, naming the node and capability |

## Exit Criteria

- [ ] `test -f graphs/books.toml`
- [ ] `cargo run -q -p xtask -- graph lint graphs/books.toml`
- [ ] `grep -q '^exit = ' graphs/books.toml && grep -q 'graph = "docs"' graphs/books.toml`
- [ ] `cargo test -q -p pii-cli --test docs_stale_inventory 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-server --test books_graph 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`

## Guardrails

- Do NOT write a second documentation coder: `docs.toml` is reached by `call`, never copied.
- Do NOT invent an iteration primitive. If the book needs per-chapter work, the coder node is handed
  the list and loops inside it — and that limit is stated in the chapter, not hidden.
- Do NOT commit. Workers never commit; the graph leaves its work in the tree.
- Do NOT put the gate in a criterion: the gate is the orchestrator's act, not a row's.
- Do NOT weaken `verify` to "the file exists". A chapter that is present and false must fail.

## Files

- `crates/pii-cli/src/cmd_docs.rs`
- `crates/pii-cli/src/main.rs`
- `crates/pii-cli/tests/docs_stale_inventory.rs`
- `crates/pii-server/tests/books_graph.rs`
- `graphs/README.md`
- `graphs/books.toml`
- `xtask/src/main.rs`

# Spec: A graph documents the code

**Project**: /projects/pi-improved · **Phase**: 0.2.1 · **Source**: user directive 2026-11-04 — *"also a graph for
documenting the code!"*

## Why this exists

`graphs/docs.toml` is a **called** graph: one `coder` node that takes `agent`, `model`, `provider`, `spec`, `workdir`
and `brief` from whoever calls it, writes documentation for that brief, and returns. It has no inventory, no selection,
and no way to tell whether the document it produced is **true** — the host's `reverify` judges the spec's criteria,
not the prose.

So nothing in the library answers the question an operator actually asks: *which parts of this codebase are
undocumented, and is the documentation that exists still true?*

## Requirements

- **R1** `graphs/doc-code.toml` exists and answers it in four steps: `inventory` (the tree's modules and their doc
  state) → `select` (the ones that need documentation, or `nothing-to-do`) → `write` (**`call` of `graphs/docs.toml`**,
  pinned, so there is no second coder implementation) → `verify` → `done`, with `unverified` as the honest middle.
- **R2** It composes rather than copies: the writing step is a `call` of `docs`, exactly as `spec.toml` already does,
  and one `graph = "docs"` appears in the file.
- **R3** `verify` proves the document is true, not merely present: every path and symbol it names must exist, reusing
  the tree walk behind `xtask doc-drift` rather than a second implementation of it.
- **R4** It is idempotent: a second run over a documented tree writes nothing and ends on `nothing-to-do`, because a
  documentation graph that rewrites the same prose every night is a graph that cannot be trusted to leave a tree alone.
- **R5** It declares `exit = "done"` so a night can call it.

## Files

- `graphs/doc-code.toml`
- `graphs/docs.toml`
- `crates/pii-server/tests/doc_code_graph.rs`
- `xtask/src/doc_drift.rs`

## Exit Criteria

- [ ] `test -f graphs/doc-code.toml`
- [ ] `grep -q 'graph = "docs"' graphs/doc-code.toml`
- [ ] `grep -q 'doc-drift' graphs/doc-code.toml`
- [ ] `grep -q 'nothing-to-do' graphs/doc-code.toml`

## Guardrails

- Do NOT write a second coder node for documentation; R2's `call` is the whole point of the library.
- Do NOT let `verify` accept a document that names a symbol that does not exist — a confidently wrong document is worse
  than a missing one.
- Do NOT let a run modify a tree that needs no documentation; R4's `nothing-to-do` is the control.

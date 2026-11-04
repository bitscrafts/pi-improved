# Spec: a small vocabulary, and composition, build every graph

## Overview

The operator's objection, verbatim:

> we need a set of tools to build every graph! like a turing machine!
>
> — and every graph built from it, **plus composition**.

That is the correct standard, and the repository currently fails it in a measurable way. An audit of
the 19 shipped graphs (2026-11-04) found:

```
using ONLY generic kinds (12)   books · doc-code · night · sdd · tdd · spec · docs · preflight
                                coder-demo · coder-podman · coder-selffix · claude-probe
needing a BESPOKE kind (7)      code-index(memory,worker) · digest-now(recall) · discord-digest(recall)
                                discord-triage(classify) · intervention-report(memory,worker)
                                queue(select,record) · self-improve(plan,enqueue)
```

And the graph written yesterday shows the failure directly: `graphs/books.toml` uses **only**
`call, data, refuse, route, tool, transform` — yet the row also added
`crates/pii-cli/src/cmd_docs.rs`, a new Rust module whose only caller is that one graph. That is a
module per graph, which is exactly what the operator forbids.

Two separate defects:

1. **A per-graph module was added where composition would do.** The books graph already invokes a
   generic `tool` node; the inventory question can be answered by `tool` + `transform` + `route`
   without any new verb.
2. **The vocabulary has holes that were filled with specialised kinds.** Eight kinds exist to serve
   one or two graphs. A set that grows a kind per graph is not a set, it is an accumulation.

**Project**: /projects/pi-improved
**Phase**: engine vocabulary

## Requirements

- **R1 — the set is stated.** `docs/03-reference/graph-vocabulary.md` names the universal kinds and,
  for each, the one thing it does: `data` (read a value), `transform` (compute), `route` (branch),
  `call` (reuse a graph), `refuse` (terminal negative), `timer` (time), `tool` (run a command),
  `spawn` (start an agent), `post` (tell a human), `judge` (execute criteria). Nine effects, one
  composition operator; nothing else is primitive.
- **R2 — iteration exists, because without it the set is not sufficient.** A graph cannot currently
  do "this subgraph once per item" — `COMPOSABILITY.md` records it, and it is why `books` had to be
  handed a whole list at once. Add the primitive: run a subgraph per element of a list, bounded by
  `max_entries`, with the element bound to a name and the results collected. This is the one
  genuinely missing construct, and it is what "like a turing machine" requires.
- **R3 — every specialised kind gets a verdict.** For each of `select · record · queue · plan ·
  enqueue · recall · memory · classify · worker · gate`: either (a) it is **derivable** — rewrite
  its graphs using the universal set and delete the kind, or (b) it is **admitted**, with the
  reason written in the vocabulary doc and a test that proves the derivation is impossible. No kind
  stays by inertia.
- **R4 — the lint makes the rule enforceable.** `cargo run -p xtask -- lint-graph-kinds` reports
  every kind used by exactly one graph as `specialised`, so the next accumulation is visible in the
  gate instead of in an audit.
- **R5 — proof by the graph that started this.** `graphs/books.toml` keeps working with the
  inventory expressed as composition, and `crates/pii-cli/src/cmd_docs.rs` is deleted: a graph whose
  only reason to exist was to serve one graph has no place in the binary.


- **R6 — composition is the other half, and it is enforced.** A graph is built from the nine **and
  from other graphs**: `call` is the growth operator, and the library's only legitimate way to gain
  a capability is to call a graph that already has it. So `lint-graph-kinds` refuses three things,
  not one: a kind outside the vocabulary that the vocabulary doc does not admit; a kind one graph
  alone uses; and a graph that re-implements a flow another shipped graph already provides instead
  of calling it. The last check is why `queue` calls `spec` (row 364) and why this row exists at
  all: duplication is the accumulation that hides behind "it works".


- **R7 — everything is a graph, and `call` is how one thing uses another.** The operator's doctrine,
  verbatim: *"in pii our call is everything is a graph!"* So a capability is not a verb and not a
  module: **it is a graph**, and any other graph reaches it by `call` with a pin. The inventory
  question that produced `cmd_docs.rs` therefore becomes `graphs/docs-stale.toml` — callable by
  `books`, by `doc-code`, by `night`, or by a person — and `books.toml` calls it instead of owning
  the logic. A CLI verb may exist only as a thin invocation of that graph (`run start docs-stale`),
  never as a second implementation of it. This is the strongest form of R5/R6 and it is the one the
  operator asked for: one mechanism, one place to change it, one place to lint it.

## Exit Criteria

- [ ] `grep -q 'composition' docs/03-reference/graph-vocabulary.md`
- [ ] `test -f docs/03-reference/graph-vocabulary.md`
- [ ] `cargo run -q -p xtask -- lint-graph-kinds 2>&1 | grep -qE '(specialised|clean)'`
- [ ] `cargo test -q -p pii-server --test iteration_primitive 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `test -f graphs/docs-stale.toml`
- [ ] `grep -q 'graph = "docs-stale"' graphs/books.toml`
- [ ] `test ! -f crates/pii-cli/src/cmd_docs.rs`
- [ ] `cargo run -q -p xtask -- graph lint graphs/books.toml`

## Guardrails

- Do NOT add a kind to solve one graph. A new kind needs two graphs that cannot be written without
  it, or a derivation proof that it cannot be written at all.
- Do NOT keep `cmd_docs.rs` for convenience: convenience for one graph is the accumulation this row
  exists to stop.
- Do NOT make iteration a special case of `coder`. It is an engine construct, and a coder that loops
  internally is a coder that cannot be resumed, traced or bounded per item.
- Do NOT delete a graph to make the audit smaller; the verdict is about the kind, not the graph.

## Files

- `docs/03-reference/graph-vocabulary.md`
- `crates/pii-graph/src/iterate.rs` (or the node kind that carries it)
- `crates/pii-server/tests/iteration_primitive.rs`
- `xtask/src/lint_graph_kinds.rs`
- `xtask/src/main.rs`
- `graphs/books.toml`
- `crates/pii-cli/src/cmd_docs.rs` (deleted)
- `crates/pii-cli/src/main.rs`
- `graphs/COMPOSABILITY.md`

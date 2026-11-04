# Spec: the vocabulary refactor — fourteen kinds, and composition — an umbrella

## Overview

The operator's goal: *"find a minimal set of tools to build them all! so we must reach this set of
tools! then review and adapt the existing graphs!"* and *"everything is a graph"*.

The derivation is in `_docs/06-research/046-answer-base-set-20261104.md` (committed, with its five
acceptance tests answered). Its verdict:

```
14 kinds   transform · route · refuse · call · timer · gate · post · tool · judge · coder · agent ·
           recall · remember · ledger
12 ship today · `remember` is registered but unused · `ledger` is the ONE new name
the single genuine ADDITION is value computation inside `transform` (let/out) — nothing else is missing
23 kinds today → 14 · 8 kinds used by exactly one graph are derived away or admitted with a reason
```

This spec is the **umbrella**: it does not implement the refactor in one commit. It states the target,
splits the work into rows that can each be verified alone, and owns the lint that keeps the library at
the target once it is reached.

**Project**: /projects/pi-improved
**Phase**: graph vocabulary

## Target (do not restate it elsewhere)

| Kind | What it is | Touches | Capability |
|---|---|---|---|
| `transform` | value computation: path, literal, `let`/`out` | nothing | none |
| `route` | takes one port by comparing a value (absorbs `classify`) | nothing | none |
| `refuse` | fails the run by name, or returns the verdict to a host | the run's outcome | none |
| `call` | inlines a sha256-pinned graph at load | nothing at runtime | the host's |
| `timer` | suspends durably until a time or cron fire | the clock | none |
| `gate` | suspends durably until a human decides | approval store, a human | none |
| `post` | one idempotent message to a channel | an external channel | `post` |
| `tool` | one entry from a CLOSED surface: `fs_read`, `glob`, `grep`, `fs_write`, `run` (literal argv0, no shell) | files, one declared process | per node |
| `judge` | runs a pinned spec's criteria — the only way to run a command the graph does not name | processes it does not name | `proc` |
| `coder` | spawns an external agent with exact env, bounded, charged to the run | process, container, model, budget | `spawn`, `proc` |
| `agent` | in-process model loop whose tool calls go through `Ctx` | a model | as listed, `net` |
| `recall` | reads the memory store | memory | `memory_read` |
| `remember` | writes the memory store: entity · relationship · doc · consolidate | memory | `memory_write` |
| `ledger` | one transaction on the work ledger: `queue.claim` · `queue.set_status` · `queue.put` · `heal.record_run` | queue, heal | a new bit (R5) |

**The rule that decides what may share a kind** (from the derivation, and it settles op-multiplexing):
the kind is the unit of **capability and effect class**; ops may share a kind only when they share what
they touch, their capability and their effect class.

## Requirements

- **R1 — the target is enforced, not documented.** `xtask lint-graph-kinds` reads every graph in
  `graphs/`, extracts node kinds from `[[node]]` tables (never a grep — see the lesson below), and
  refuses: (a) a kind outside the fourteen; (b) a kind used by exactly one graph, unless the doc
  admits it with a reason; (c) a graph that re-implements a flow another shipped graph already
  provides. Its own test proves each of the three refusals with a planted case.
- **R2 — the addition lands**: `transform` gains `let`/`out` so a value can be computed (arithmetic,
  concatenation, comparison) without a shell. A graph that needs a stamp compared to HEAD must be
  writable with it; the test is that graph.
- **R3 — the derivations land, in an order where each is verifiable alone**: `worker` → `coder`;
  `classify` → `route`; `plan` → `agent`; `memory` → `remember`; `enqueue` + `record` + `select` +
  `heal_record` → `ledger`; `debug_crash` → a test fixture; `heal_detect` → `heal_record`'s flow.
- **R4 — `tool` stops being the escape hatch**: its command surface becomes the closed list above and
  a `run` entry takes a literal argv0 with no shell. The graphs that today pass a `bash -c` string
  (`books`, `doc-code`) are rewritten to `tool` entries or to a `call` of a graph that does it.
- **R5 — the ledger gets its capability**: one bit in `crates/pii-abi/src/cap.rs`. Today any graph can
  claim a queue row, because `select`, `record` and `heal_record` hold no capability at all.
- **R6 — the library is adapted, not left behind**: after R1–R5, every graph in the environment lints
  at the target and the 11-graph system set (spec 384) is the manifest.

## The lesson this spec is built on

An earlier audit of mine counted 23 kinds by grepping `kind =` in the TOML. That matches **edge** kinds
too: it invented `data` (120 edge declarations) and missed `coder` and `agent`. Every number in this
spec comes from parsing `[[node]]` tables with `tomllib`. Any check in R1 must parse, never grep.

## Exit Criteria

- [ ] `cargo run -q -p xtask -- lint-graph-kinds`
- [ ] `cargo test -q -p xtask --test lint_graph_kinds 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `grep -q 'fn same_value\|let_value\|"let"' crates/pii-graph/src/expr.rs`
- [ ] `test ! -f graphs/code-index.toml || ./target/debug/pii graph lint graphs/code-index.toml`
- [ ] `grep -q 'ledger' crates/pii-server/src/engine/registry.rs`
- [ ] `grep -q 'CapKind::Ledger\|ledger' crates/pii-abi/src/cap.rs`

## Guardrails

- Do NOT implement this in one commit. Each of R2–R5 is its own row, its own gate, its own verification.
- Do NOT delete a kind before every graph that uses it has moved; the lint's refusal (b) exists to make
  the last user visible rather than to be silenced.
- Do NOT merge `coder` and `agent`: they differ in exactly what the ABI measures — env, argv grant,
  budget charging, and whether tool calls are mediated by `Ctx`.
- Do NOT widen `tool` to keep a graph working. That is the move that made an earlier analysis of mine
  "prove" a two-kind vocabulary.
- Do NOT count kinds: count what each kind touches, what capability it needs, and its effect class.

## Files

- `xtask/src/main.rs`
- `xtask/src/lint_graph_kinds.rs`
- `xtask/tests/lint_graph_kinds.rs`
- `crates/pii-graph/src/expr.rs`
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-abi/src/cap.rs`
- `docs/03-reference/graph-vocabulary.md`

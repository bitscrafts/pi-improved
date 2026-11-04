# 046 — Derivation brief: one minimal set of node kinds that expresses every graph

The operator's framing, verbatim: *"a minimal set to rule them all"* — **and everything is a graph.**
The doctrine he stated: *"in pii our call is everything is a graph!"*

**One set. Minimal. Sufficient for every graph that ships today and for every capability the system
needs to have.** Not a set per family, not a set plus an escape hatch — and no capability that exists
only as a CLI verb or a Rust module, because in pii a capability is a graph and `call` is how one
thing uses another.

## What exists (measured from `[[node]]` tables, 2026-11-04)

32 graph files under `graphs/`, 23 node kinds:

```
transform 26 · refuse 11 · timer 8 · call 7 · coder 7 · route 6 · agent 5 · gate 5 · post 5 ·
judge 4 · tool 4 · memory 3 · heal_detect 2 · recall 2 · worker 2 ·
classify 1 · debug_crash 1 · enqueue 1 · heal_record 1 · plan 1 · record 1 · select 1 · wait 1
```

Implementations: `crates/pii-graph/src/nodes.rs` (control kinds), `crates/pii-server/src/node_kinds/`
and `crates/pii-server/src/node_kinds/nodes/` (effects and operations), registered in
`crates/pii-server/src/node_kinds/registry.rs`.

Two facts that constrain any answer:

- **`transform` cannot compute.** Expressions are member access only: no functions, no arrays, no
  arithmetic over values from two nodes. Anything that needs arithmetic or string building today
  reaches for `tool` (bash) instead.
- **`tool` runs bash.** That makes "the set is sufficient" trivially true and useless, and it is how
  an earlier analysis of mine fooled itself. Any base set must say what happens to `tool`: keep it
  unrestricted with the consequence stated, or constrain it to a declared command surface.


## The prior analysis (review 045) — pass it in, do not repeat it

An earlier analysis of mine was reviewed and partly demolished. Its *conclusions* are given below as
**hypotheses to confirm or refute against your derivation**. Do not re-derive them from scratch; use
them as the starting point and say, for each, confirmed / refuted / refined, with the evidence.

What the review established as **measured fact** (keep these):

- `data` is not a node kind. An earlier grep of `kind =` matched *edge* kinds; 120 of those hits were
  edges. Any count must come from parsing `[[node]]` tables.
- `coder` and `agent` are the two kinds that call a model, and `plan` calls the provider directly as
  well, so the model call currently exists in three places.
- `spawn` starts a child graph run and returns its id without waiting; it is not an agent call.
- `route` and `tool` each have few users, so "used by one or two graphs" cannot by itself be the test
  for a specialised kind.
- Loops already exist: budgeted cycles work (`verify-fix.toml`, `state.rs`). "Iteration is missing"
  is not true as stated; a bounded per-item construct may still be needed, but not because graphs
  cannot loop.

Hypothesised **missing primitives**, most blocking first (confirm or refute each, and re-rank):

1. Value computation — `transform` does member access only; no functions, no arrays, no arithmetic
   across two nodes' values. Anything needing arithmetic or string building reaches for `tool`.
2. Durable await — `gate` suspends on a resume key and survives restart.
3. A typed effect over a daemon-owned resource — `recall`/`memory` are the working examples, guarded
   by `proc` rather than by a capability of their own.
4. An agent/model call as one primitive — see `coder`/`agent`/`plan` above.
5. Error or handler scope — a refusal a host does not wire fails the run.

Hypothesised **per-kind verdicts** (confirm, refute, or refine, and fold into your derivation):

```
select        admit — one transaction, four ports, a lease by run id
record        admit the write, derive the interpretation
enqueue       split: the store write joins select/record; the validation is a graph
plan          derive — a model call plus string building
recall        admit — a capability-checked read
memory        derive — one op (consolidate) whose only loadable user is a starter graph
classify      widen `route` rather than add a kind
worker        derive — both graph uses mean `coder`
gate          admit as a primitive (durable human decision)
queue         not a kind at all; the leftover is a `judge` item mode
```

And one normative finding to carry forward, because it changes what a "per-graph verb" costs:

> A per-graph verb with tests is the **lesser** failure compared with a shell string in the run input.
> Judge logic by whether it is visible, testable and pinned — not by how many callers it has.

## Your task

**1. Derive the base set.** A kind belongs in the base if and only if it is an *effect* (it touches
the world: time, a process, a model, a store, a human, a durable await) or a *control* that cannot be
expressed by the others (`call`, `route`, `refuse`, and the computation you specify). Say for each
kind in the base: one line of semantics, what it touches, and the capability it must hold.

**2. Give a derivation for every one of the 32 graphs.** A table: graph → the base kinds it would use
→ the kind(s) it uses today that disappear. Where a graph cannot be expressed, say precisely what is
missing and why the missing thing is a primitive rather than a graph. `graphs/starter/` and
`graphs/fragments/` are in scope; include them.

**3. Specify the computation primitive**, which is the one thing that must be *added*. Choose between
(a) an expression language inside the graph file, (b) a `compute` node kind, (c) a small typed
language with its own parser — and justify the choice against: pinnability (a graph is reviewed by
reading it), the capability ladder, resumability, and what a workhorse model can write correctly on a
first attempt.

**4. Order the adaptation.** Which graphs to rewrite first so that each step is independently
verifiable; which deletions are safe only after which rewrites; and what the lint must refuse at each
step so a half-migrated library cannot ship.

**5. Name what this refactor cannot fix.** Specifically: the defects in the current library that are
not vocabulary problems — two files that do not load, verdicts no host wires, copies
(`digest-now` ≈ `discord-digest`), op-multiplexing, logic in `brief` strings, untyped port values.


## The acceptance test for your answer

Your set passes only if **all four** hold. Answer each explicitly, as a line that says PASS or FAIL
with the count:

1. **Every shipped graph is expressible.** For all 32 files under `graphs/`, the derivation table has
   a row and no row says "cannot be expressed".
2. **Nothing in the set is derivable.** For each kind in your set, one sentence says why the other
   kinds cannot produce it. A kind whose sentence is weak is not in the set.
3. **Nothing outside the set is needed.** For each of the 23 kinds that exist today, your derivation
   says which base kinds replace it, or which graph cannot be expressed without it.
4. **`tool` cannot carry the proof.** Show the derivation with `tool` restricted to a declared,
   reviewable command surface. If a graph then cannot be expressed, that graph — not `tool` — is the
   evidence that a primitive is missing, and it belongs in the set.

5. **Everything is a graph.** The set must be sufficient to express a *capability* as a graph, not
   only to express the graphs that exist. Demonstrate it twice: (a) derive `pii docs stale` — today a
   Rust module (`crates/pii-cli/src/cmd_docs.rs`) written for one graph — as a graph over your base
   set; (b) state the rule that follows, in one sentence, for when a capability may be a verb and
   when it must be a graph. If (a) is impossible, that impossibility is a missing primitive and it
   belongs in the set, named with the capability it blocks.

If any of the tests fail, say so plainly and name the smallest set that does pass. A set of eleven
that fails test 4 is worse than a set of fourteen that passes it; a set that expresses every graph
today but cannot express one new capability tomorrow has failed test 5.

## Structure your answer as

```
A. THE BASE SET           table: kind | semantics | touches | capability
B. DERIVATION             table: graph | base kinds | kinds that disappear | notes
C. THE COMPUTATION PIECE  choice + justification + a worked example of a graph that needs it
D. ADAPTATION ORDER       numbered steps, each with its own verification
E. THE LINT               what it must refuse, in the order the refactor needs it
F. NOT-VOCABULARY         the defects the refactor does not fix
G. RISKS                  what breaks silently while the library is half-migrated
```

## Constraints

- Cite file paths and lines for every claim about the code. If you cannot check something, say so.
- Do not propose engine changes; if a base set is impossible without one, say which and why.
- Prefer deleting a kind to adding one. Every addition must be justified by a graph that cannot exist
  without it — name the graph.
- Do not assume `tool` is available to derivations: show what a graph looks like if `tool` is
  constrained, and say which graphs could not be expressed then.

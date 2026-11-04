# 046 — Derivation brief: the minimal set of node kinds that builds every shipped graph

You are not reviewing a claim this time. You are **deriving** a base set, and you will be held to the
proof. Read only; do not edit the tree. Answer in English, in the structure requested at the end.

## The operator's goal, verbatim

> the idea is to find a minimal set of tools to build them all! so we must reach this set of tools!
> then review and adapt the existing graphs!

So the deliverable is a **base set**, a **derivation per existing graph**, and an **adaptation order**.
Nothing about the engine's composition machinery is to change: the splice, the exit rule, the pin
model and the capability ladder work and are tested. The question is only *what the node kinds are*.

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

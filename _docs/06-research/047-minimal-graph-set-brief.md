# 047 — Derivation brief: the minimal set of GRAPHS pii needs

Companion to 046 (which derives the node-kind vocabulary). This one asks the other question: given
that vocabulary, **which graphs must pii ship**. Read only; do not edit the tree.

## The question

> a minimal set of graphs is really needed to pii

pii's purpose, from its own documentation: a personal-information index whose work is expressed as
graphs; it must detect a gap, write the spec, implement it, judge it, review it and report — with only
the commit left to a human. Today `graphs/` holds 32 files: some are that environment, some are
samples, some are one-offs, two do not load at all.

Derive **the set a pii host needs**, and only that.

## What exists today (measured 2026-11-04)

```
calling others:  night → sdd · queue → spec · sdd → tdd, docs · spec → docs · self-improve → queue ·
                 books → docs · doc-code → docs
never called:    books · doc-code · night · self-improve · preflight · digest-now · discord-* ·
                 coder-demo · coder-podman · coder-selffix · claude-probe · code-index ·
                 intervention-report
do not call:     docs · tdd · preflight · digest-now · discord-triage · coder-* · claude-probe ·
                 code-index · intervention-report
```

Two documented disagreements you must resolve rather than inherit:

- `_docs/05-operations/the night loop` says the night **fills the queue, drains it, reviews, refills**
  — but the shipped `night` calls `sdd` and never touches the queue.
- `spec` and `sdd` both implement a spec (judge-based vs `tdd`-based). Spec 385's position, for you to
  accept or reject: `sdd` is not a duplicate but a *methodology*, and the driver should choose one.

## Your task

**1. State pii's required flows**, each as one sentence of behaviour an operator would ask for. No
implementations, no graph names: what must a host be able to do?

**2. Map flows to graphs.** For every flow: exactly one graph in the set, or a composition of the set.
Where two graphs serve one flow, say which is the duplicate and which survives.

**3. Give the minimal set.** For each graph in it: purpose (one line), entry, exit, what it `call`s,
capabilities, and **the flow that breaks if it is removed**. A graph nobody's flow needs is not in the
set.

**4. Close it.** Every `call` target of every graph in the set is itself in the set, and each pin
matches the file that ships.

**5. Classify today's 32 files** into: **environment** (in the set) · **sample** (kept in the repo,
not installed) · **obsolete** (delete, with the reason) · **broken** (does not load; fix or drop).

**6. State the rule for a new graph.** One paragraph: what must be true before a graph is added to the
environment rather than to samples, written so the next person can apply it without asking.

## Acceptance tests — answer each PASS or FAIL, with a count

1. **No gap.** Every flow from task 1 has an owner in the set.
2. **No duplicate.** For each flow, exactly one owner; where a second graph exists, it is classified
   obsolete with a name.
3. **No passengers.** Every graph in the set has a named flow whose absence breaks it.
4. **Closure.** Every `call` target inside the set, pins matching.
5. **Everything is a graph.** Any operator-facing capability that today lives in a CLI verb or a
   script — the stale-chapter inventory, the queue fill, the digest — is either in the set, derivable
   by composing it, or named as blocked with the primitive it needs.

## Constraints

- Do not size the set by taste. "Minimal" means: remove any member and a flow from task 1 loses its
  owner.
- Do not keep a graph because it exists. Do not delete one because it is small.
- `sdd`/`spec` duplication and the night-without-a-queue disagreement must be decided, not described.
- Cite files and lines. If a flow has never run in production, say so — that is evidence about the
  flow, not a reason to keep the graph.

## Structure your answer as

```
A. FLOWS               numbered, one sentence each
B. FLOW → GRAPH        table: flow | owner graph | composition if any | duplicate found
C. THE SET             table: graph | purpose | entry | exit | calls | caps | flow it serves
D. CLOSURE             the call edges, with pins
E. THE 32 FILES        environment | sample | obsolete | broken, with reasons
F. THE RULE            when a new graph joins the environment
G. WHAT IS BLOCKED     capabilities that cannot be a graph today, and the primitive each needs
```

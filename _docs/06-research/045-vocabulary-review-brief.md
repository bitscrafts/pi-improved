# 045 — Review brief: is the graph vocabulary actually sufficient?

You are reviewing an **analysis**, not code. Read only; do not edit the tree. Answer in English, as
a numbered response to the questions at the end, and state plainly where the analysis is wrong.

## The system under review

`pii` is a single Rust binary whose work is expressed as **graphs**: TOML files of nodes with typed
ports, composed by `call`. Repository: `<repo>`. Start with
`graphs/COMPOSABILITY.md`, `graphs/README.md`, and the node kinds under
`crates/pii-server/src/node_kinds/nodes/`.

## The analysis you are reviewing (my claims, to be attacked)

An audit of the 19 shipped graphs (measured 2026-11-04) found:

```
12 of 19 use only "generic" kinds: books · doc-code · night · sdd · tdd · spec · docs ·
                                    preflight · coder-demo · coder-podman · coder-selffix · claude-probe
 7 of 19 need a kind that exists for one or two graphs:
      code-index(memory,worker) · digest-now(recall) · discord-digest(recall) ·
      discord-triage(classify) · intervention-report(memory,worker) ·
      queue(select,record) · self-improve(plan,enqueue)
```

From that I concluded, and wrote into `specs/383-a-small-vocabulary-builds-every-graph.md`:

- **C1** The primitive set is nine effects: `data` (read a value), `transform` (compute), `route`
  (branch), `call` (reuse a graph), `refuse` (terminal negative), `timer` (time), `tool` (run a
  command), `spawn` (start an agent), `post` (tell a human), `judge` (execute criteria). Everything
  else must be *derivable* from these by composition.
- **C2** The one genuinely missing primitive is **iteration**: the engine cannot "run this subgraph
  once per item of a list", which is why `books` had to be handed a whole chapter list and loop
  inside a coder node — a loop inside a coder cannot be resumed, traced or bounded per item.
- **C3** The eight specialised kinds should each be either *derived away* (their graphs rewritten
  with the nine) or *admitted* into the vocabulary with a written reason and a proof that derivation
  is impossible.
- **C4** A CLI verb whose only caller is one graph is a defect. I hit this myself: writing
  `graphs/books.toml` (which uses only `call, data, refuse, route, tool, transform`) I also added
  `crates/pii-cli/src/cmd_docs.rs` to answer "which chapter is stale". That is a module per graph.
  Spec 383 R5 deletes it.
- **C5** The rule is enforceable by a lint refusing three things: a kind outside the vocabulary not
  admitted by the doc; a kind exactly one graph uses; a graph that re-implements a flow another
  shipped graph already provides instead of calling it.

## Questions — answer each explicitly

1. **Is C1 right?** Is the nine-effect set actually sufficient to express the seven bespoke graphs,
   or is a primitive missing from the list (state carried across iterations, parallel/join,
   error/handler scope, a read of the store, a decision on a *set* rather than a value)? Name the
   primitive, not a feature.
2. **Is C2 the right shape?** If iteration is the gap, what is the correct construct — bounded
   `for-each` with per-item exits collected, or something else — and what does it do to resumability,
   tracing and the `max_entries` bound?
3. **Verdict per specialised kind.** For each of `select · record · queue · plan · enqueue · recall ·
   memory · classify · worker · gate`: derive it away, or admit it — with the reason. Read the kinds
   before answering; `queue` is the row-claiming pair, `plan`/`enqueue` are the self-improve pair.
4. **Is C4 right?** Is "no per-graph verb" the correct rule, given the alternative it pushes people
   toward: a shell string in the graph config, which is logic that no lint, capability check or
   operator can see. Which of the two is the lesser failure, and is there a third option?
5. **What breaks first at 50 graphs?** What does the C5 lint *not* catch — specifically, which kinds
   of duplication or accumulation would still pass it?
6. **Where did I overreach?** List the claims in C1–C5 that are opinion dressed as measurement, and
   say what measurement would settle each.

## Constraints

- Read-only. No edits, no commits, no test runs that mutate the store.
- Cite file paths and line numbers for anything you assert about the code.
- If a claim of mine cannot be checked from the repository, say so and name what would check it.
- Prefer naming the failure mode to proposing a redesign: the operator will decide the design.

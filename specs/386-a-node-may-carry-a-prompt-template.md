# Spec: a node may carry a prompt template, with its values bound and its placeholders checked

## Overview

The operator asked the format question directly: *"could a graph item have an associated prompt or
template?"*

Today it cannot, and both alternatives are bad:

```
brief = "Write chapter 4 of the user manual"      a literal: visible and pinnable, but no values
brief = "$.nodes.inventory.stdout"                a path: values, but the instruction lives elsewhere
```

Measured: a `coder` node requires `agent`, `model`, `spec`, `workdir`, `brief`, and accepts
`provider`, `session`, `sid`, `max_budget_usd` (`crates/pii-server/src/node_kinds/nodes/coder/input_cfg.rs:8`).
There is no `prompt` field and no placeholder substitution: `brief` is resolved as one expression and
handed over whole.

So an instruction that needs *both* text and values has nowhere to live. It ends up in a shell string
(a `tool` node), in a file outside the graph, or in a verb written for one graph — the three ways
logic escapes the review that a pinned graph provides.

**Project**: /projects/pi-improved
**Phase**: graph format


## The motive: why an instruction belongs to the node

Six reasons, and each one is a defect that exists today because the instruction lives elsewhere:

1. **Reviewability.** A graph is reviewed by reading it. If the instruction is a `brief` path or a CLI
   argument, the reviewer approves a flow without seeing what the model will actually be told — and the
   words are most of the behaviour of a coder node.
2. **Pinning.** A graph's `sha256` covers its text. A prompt *in* the graph is covered by the same
   digest, so "the flow that ran" includes the words it ran with. A prompt in a separate file, an env
   var, or a `run` argument is not pinned by anything reviewable.
3. **Composability.** A subgraph's instruction is part of its contract. With templates, a caller passes
   values and the callee keeps its own words; without them, every caller restates the brief — which is
   the same duplication row 364 fixed for *flows*, one level down, for *instructions*.
4. **One graph, many cases.** "Write chapter {{input.chapter}} of {{input.book}}" is one node and one
   review, not one graph per chapter. The engine has no per-item iteration primitive, so a template is
   what makes a list-shaped job expressible at all.
5. **Audit.** The resolved prompt can be journalled, so "what was it told" is answered from the record.
   A brief assembled by a shell string in the run input leaves no trace but the string.
6. **Testability.** A placeholder naming a path the graph cannot supply is checkable at load. A brief
   that arrives from outside is checkable by nothing — which is how `brief = "$.nodes.inventory.payload.stdout"`
   shipped empty on every run, with every test and gate green.

The motive in one sentence: **if the instruction is not in the pinned artefact, the reviewed thing and
the executed thing are different things.**

## Requirements

- **R1 — a template field.** A node may declare `prompt`, a literal string with placeholders of the
  form `{{path}}`, where `path` is an expression resolved against the same eval context the node
  already uses (`input`, `payload`, `nodes`). Example:

  ```toml
  [node.write]
  kind = "coder"
  prompt = """
  Write chapter {{input.chapter}} of the {{input.book}} manual.
  Chapters still stale: {{nodes.inventory.stdout}}
  """
  ```

- **R2 — `prompt` and `brief` are exclusive.** Declaring both is refused at load, naming the node. A
  node says where its instruction comes from exactly once.
- **R3 — the placeholders are checked at load, not at spawn.** A placeholder naming a path the graph
  cannot supply is a refusal at load with the placeholder and the node named. A typo that today fails
  as an empty prompt mid-run fails in one second instead.
- **R4 — the resolved prompt is journalled.** The run's journal records the text the model was
  actually given (bounded by the existing output cap), so "what was it told" is answerable from the
  record rather than from a reconstruction.
- **R5 — a prompt is bounded data.** `prompt` is capped (64 KiB) and refused above it, so a graph
  cannot smuggle a file into an instruction; the cap is named in the refusal.
- **R6 — no capability change.** A prompt does not grant anything: the node's caps and the graph's
  ceiling are untouched, and a coder still cannot write without `fs_write`.

## TDD contract

| Test | Input | Expected |
|---|---|---|
| `a_placeholder_resolves` | `prompt = "x {{input.n}}"` with `n = 3` | the model receives `x 3` |
| `an_unknown_placeholder_is_refused_at_load` | `prompt = "{{input.nope}}"` | load refuses, naming `input.nope` |
| `prompt_and_brief_together_are_refused` | both fields | load refuses, naming the node |
| `the_resolved_prompt_is_journalled` | any run with a template | journal carries the resolved text |
| `an_oversized_prompt_is_refused` | 65 KiB | refusal naming the cap |
| `a_tool_node_with_a_prompt_is_refused` | `kind = "tool"` + `prompt` | load refuses: prompts belong to agent nodes |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test prompt_template 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-graph --test placeholder_checks 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `grep -q 'prompt' crates/pii-server/src/node_kinds/nodes/coder/input_cfg.rs`
- [ ] `./target/debug/pii graph lint graphs/books.toml`

## Guardrails

- Do NOT allow placeholders to be resolved by running a command. A template is data resolved from
  values the graph already has; it must not become a second `tool`.
- Do NOT let an unresolved placeholder become an empty string: absent values are refusals, not blanks
  (an empty prompt is how a coder spends money writing nothing).
- Do NOT extend this to `tool` nodes. A command's text is not a prompt, and conflating them recreates
  the `tool`-carries-the-proof problem.
- Do NOT skip the journalling: a prompt nobody can read after the fact is a prompt nobody can audit.

## Files

- `crates/pii-server/src/node_kinds/nodes/coder/input_cfg.rs`
- `crates/pii-server/src/node_kinds/nodes/coder/graph_node.rs`
- `crates/pii-graph/src/nodes.rs` (placeholder checking at load, if it belongs with the expression evaluator)
- `crates/pii-server/tests/prompt_template.rs`
- `crates/pii-graph/tests/placeholder_checks.rs`
- `docs/03-reference/graph-vocabulary.md`

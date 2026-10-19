# Spec: graph fragments — a skill or hook graph attachable to any other graph

## Overview
User directive 2026-10-19: "so we could have a generic skill or hook graph that can be attached on any other graph!" This is the
composition step that makes skills and hooks worth having: written once as a fragment, attached by name wherever the surface
is, resolved and woven at load time. Without it every graph re-implements its own guard, and "one graph per concern" decays
into "one graph per occurrence of a concern" — which is exactly the sprawl this project is trying to reverse.

**Project**: /projects/pi-improved

## Verified premises
- spec 113's `sop` node already weaves bookkeeping nodes around steps — the weaving primitive exists.
- spec 211 fixes hooks to be woven nodes with `woven-from: hook/<id>` provenance printed by `graph check`.
- the graph loader resolves one file per graph today; there is no include/attach mechanism.

## Requirements
### Functional
1. GRAPH FILES may attach fragments:
   `[[attach]]` · `id = "guard"` · `fragment = "hooks/podman-guard"` · `at = "run.start"|"run.end"|"node.before"|"node.after"` ·
   `on = ["*"]` or `on = ["build", "deploy"]` (node ids) · `pin = "<sha256 of the fragment file>"`.
2. A FRAGMENT is a normal graph file under `graphs/fragments/` declaring its own contract header:
   `[fragment]` · `name` · `inputs = ["…"]` · `outputs = ["…"]` · `may_block = true|false`. Only fragments with `may_block = true`
   may attach at a PRE surface; the loader refuses the rest (`fragment.block.invalid`).
3. Resolution is at LOAD time: the loader inlines the fragment's nodes with ids namespaced `<attach-id>.<node-id>` and stamps
   every inlined node with `woven-from: <fragment>#<node-id>`. `graph check` prints the woven nodes and their origin.
4. Pins are verified: a fragment file whose sha256 differs from `pin` is REFUSED (`fragment.pin.drift`), naming the file, the
   expected and the actual hash. No silent upgrade of a guard.
5. BUDGET: inlined nodes count against the run's existing budget. A fragment may not declare its own timer, its own store or
   its own budget (`fragment.autonomy.forbidden`) — a fragment is work, never a scheduler.
6. ORDER is deterministic: multiple attaches at the same surface run in a declared `order` integer, ties broken by id, and
   `graph check` prints the resulting sequence.
7. NO RECURSION: a fragment may not attach fragments (depth 1). Nested attach is refused (`fragment.nesting.forbidden`) —
   the same rule as a macro that may not expand a macro, and it keeps a run's node graph readable.
8. FAILURE: a fragment node failing routes to ITS OWN `err` port first; if unhandled, the run fails naming fragment, node and
   the original reason. A blocking hook that refuses at `run.start` produces `run.refused{hook=…, reason=…}` with no node
   executed.

### Non-Functional
- A fragment is one file, hashed, reviewable in a diff; it is not generated at runtime.
- `graph check` output stays human-readable: one line per woven node, `origin` column.
- Removing an attach must be a one-line edit with no orphan nodes in the compiled graph.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| fragment_inlines_at_run_start | attach at run.start | woven node present, namespaced id, origin stamped |
| fragment_pin_checked | wrong pin | `fragment.pin.drift`, graph refused |
| block_only_where_allowed | may_block=false attached at node.before | `fragment.block.invalid` |
| no_nesting | fragment attaching a fragment | `fragment.nesting.forbidden` |
| budget_is_shared | fragment with 3 nodes | run budget covers them; fragment declares none |
| order_is_deterministic | two attaches, same surface | sequence matches order then id, printed by check |
| refusal_names_the_hook | blocking hook returns 1 | `run.refused{hook=…}`, zero nodes executed |

## Exit Criteria
- [ ] `cargo test -p pii-graph --test fragments_attach`
- [ ] `cargo test -p pii-graph --test fragment_pin_and_nesting_refused`
- [ ] `cargo run -q -p pii-cli -- graph check graphs/fragments/podman-guard.toml` shows the fragment's own nodes
- [ ] `cargo run -q -p pii-cli -- graph check graphs/coder-demo.toml` prints the woven node with `woven-from`

## Guardrails
- Do NOT let a fragment declare a timer, a store or a budget.
- Do NOT allow nesting; one level, always.
- Do NOT inline without a pin check.
- Do NOT let inlining reorder nodes the operator wrote — woven nodes sit at the declared surface only.
- Do NOT generate fragments at runtime: a fragment that appears during a run is a code path nobody reviewed.

## Depends on
- 211 (hooks as woven nodes) — the fragment mechanism is the general form of what 211 needs for hooks.

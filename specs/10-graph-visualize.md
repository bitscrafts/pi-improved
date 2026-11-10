# Spec: `pii graph show` — human-readable ASCII graph visualization

**Project**: <repo>
**Phase**: post-3.6 — developer-ergonomics feature (independent of the
memory work in `specs/09-global-memory.md`)
**Status**: DRAFT — spec only, no implementation authorized yet.

## Overview

Today, understanding a graph's shape (branches, loops, fan-in/fan-out)
means reading raw TOML by hand and mentally tracing `[[edge]]` blocks.
`pii graph lint` only validates; `pii graph trace` only replays a
COMPLETED run's JSON event log. Neither shows the graph's STRUCTURE.

This spec adds `pii graph show <file.toml>`: a static, terminal-native
ASCII rendering of a graph's topology — nodes, branches (`route`
if/then/else), fan-in/fan-out (join policies), and loops (`loop_budget`
cycles) — readable directly in an SSH session or `pii tui`, no external
viewer needed.

### Research basis (why this design, not a generic force-directed graph)

`git log --graph` (and descendants like `tig`) solve the same problem —
rendering a DAG as readable ASCII in a terminal — via a **lane
allocation algorithm**: walk items in topological order, maintaining an
array of "active lane" slots; each item claims the slot expecting it
(continuing a lane), the lowest free slot (a new branch), or opens
extra slots per additional parent (a merge/fan-in fanning IN, drawn
`├─╯`) or additional children (a fork/fan-out fanning OUT, drawn
`├─┐`). This is a well-proven, exact fit for pii's `route`
if/then/else branches and `JoinPolicy::All`/`Any` fan-in joins, because
those map onto the same "one row forks into N, or N rows converge into
one" shape a git merge/branch does.

**Where it does NOT port directly: loops.** Git's commit history is a
DAG (no cycles) — the lane algorithm assumes strictly-forward flow.
pii's graphs genuinely cycle (`loop_budget`, `sched::loop_helpers::
reset_loop`, e.g. `verify-fix.toml`'s `fix → test → gate → retry →
fix`). Forcing a back-edge through lane columns produces exactly the
diagonal-crossing mess that even git's own rendering is criticized for
on merge-heavy histories. **Design decision**: lay out nodes in
topological order (ignoring back-edges for layout purposes — a cycle's
re-entry edge is identified via `sched::loop_helpers::is_in_cycle`,
already implemented), and render each such edge as a separate `^--
loops back to "<id>" (budget=N)` annotation line after the loop body,
not as a lane.

### Why git never actually draws a cycle (and how that reframes "loops")

The deeper reason git's lane algorithm has no cycle case isn't that
cycles are hard to draw — it's that git never renders *structure*, it
renders *history*. A commit that redoes yesterday's work ("repeat the
day again") isn't a back-edge to yesterday's commit; it's a NEW commit,
one row further down, at a later point in linear time. Time only moves
forward, so a history of arbitrarily-repeated work still has zero
cycles to draw.

This maps onto pii's loops directly, and changes what "the right
answer" is depending on what's being rendered:

- **Rendering the graph's DEFINITION (this spec's `graph show`, no run
  has happened yet)**: there is no "day 2" to show — `loop_budget=3`
  says "up to 3 iterations may happen," not "3 iterations did happen."
  A structural back-edge is unavoidable here, so the `^-- loops back
  to` annotation (above) is correct AND NECESSARY for this case.
- **Rendering a run's TRACE (a completed or in-flight run, using the
  seq/event journal `pii graph trace` already reads)**: the cycle
  disappears entirely, for the exact git reason. Each loop iteration is
  a distinct set of node-execution events at strictly increasing `seq`
  numbers — `fix`#1, `test`#1, `gate`#1, `retry`#1, `fix`#2, `test`#2,
  ... A trace rendering lists EXECUTIONS (one row per event), not
  DEFINITIONS (one row per node-id) — so it can reuse the exact same
  linear, cycle-free git-log-style row format from `V2` below with zero
  special-case loop handling, because what actually happened has no
  cycles by construction, same as a git history that repeats work.

**This elevates the "v2 live overlay" idea from a nice-to-have deferral
to the RIGHT long-term answer for loops specifically**: this spec (v1)
still ships the static, no-run-yet definition view with the annotation
(there's genuinely no substitute for it before a run exists), but a
follow-up spec should add `pii graph trace --show`-style rendering that
lists real executions in seq order, unrolling every loop iteration as
its own block of rows — exactly like `git log` unrolls repeated work as
new commits instead of drawing a cycle. That follow-up is intentionally
NOT part of this spec's scope (see Guardrails), but is now recorded
here as the correct direction rather than a vague "maybe later."

## Requirements

### Functional

#### V1 — Parse & layout
- Input: a graph TOML path (same acceptance as `pii graph lint`) OR a
  registered graph name (same resolution as `Engine::load_spec` — file
  path, then `graphs/<name>.toml`, then the `default` built-in).
- Reuse the EXISTING parser/validator (`pii_graph::toml_load` +
  `pii_graph::validate`) — `graph show` runs validation first and
  reports the same V0-style errors `graph lint` does if the graph is
  malformed; it never renders an invalid graph.
- Topological order via the existing `GraphSpec`/`petgraph` dependency
  (already a transitive dep via `loop_helpers.rs`) — for layout only;
  loop edges are set aside per the cycle rule above (identify via
  `is_in_cycle`, already implemented, do not reimplement).

#### V2 — Row rendering (git-log-graph-inspired, ASCII only)
- One row per node, in topological order, format:
  `<id> [<kind>]<extra>` where `<extra>` surfaces kind-specific
  detail already on `NodeSpec`/`cfg` a human needs to read the graph
  without opening the TOML:
  - `agent`: `(loop_budget=N)` if set
  - `route`: `[route: <on-expr>]`
  - `tool`: `[tool: <tool-name>]`
  - `transform`: `[transform: <expr>]`
  - anything else: bare `[<kind>]`
- Fan-out (a node's outbound edges split to >1 distinct `to`): render
  as `|-- <port> --> <to>` lines, one per edge, git-branch style (no
  claim of exact `├─┐` box-drawing glyphs in v1 — see Guardrails on
  ASCII-only).
- Fan-in (`JoinPolicy::All`/`Any`): annotate the CONVERGING node's row
  with `(join: all)` / `(join: any, cancel_losers=<bool>)` when it
  differs from the default (`All` with no cancel — omit the annotation
  when it's the default, to avoid noise on every plain node).
- Loops: after the last row of a cycle's body (in topo order), emit
  `^-- loops back to "<id>" (budget=N)` referencing the re-entry
  node's id and its `loop_budget` (from `NodeSpec.loop_budget`).
- `entry`/terminal nodes: mark the entry node's row with `(entry)`;
  nodes with no outbound edges get no fan-out lines (implicitly
  terminal — no separate "(terminal)" marker needed, absence of
  `-->` lines already says it).

#### V3 — CLI wiring
- `pii graph show <file-or-name>` — new subcommand alongside
  `lint`/`run`/`resume`/`trace` in `cmd_graph` (`crates/pii-cli/src/
  main.rs`).
- No flags in v1 (no `--format=mermaid`, no live-run overlay — see
  Guardrails). Exit code 0 on success, matching `graph lint`'s
  2-on-usage-error / 1-on-validate-error convention.

### Non-Functional
- Pure, offline, no daemon/socket call required — `graph show` reads
  and renders a TOML file directly, exactly like `graph lint` does
  today (no live daemon needed to inspect a graph's shape).
- Deterministic output (same TOML in -> byte-identical output out) —
  required for any future golden-file test.
- No new dependencies — `petgraph` is already a transitive dependency
  used by `loop_helpers.rs`.

## Architecture

```
pii graph show verify-fix.toml
  |
  v
toml_load::load(path) -> GraphSpec        (existing)
  |
  v
validate::validate(&spec)                 (existing; abort+report on error)
  |
  v
topo_order(&spec) using petgraph,          (NEW, small: reuse the same
  back-edges (is_in_cycle) excluded         DiGraph construction pattern
  from the ordering pass                    already in loop_helpers.rs)
  |
  v
render_rows(&spec, &order) -> String      (NEW: the only real new logic)
  |
  v
print to stdout
```

Worked example, using the ALREADY-COMMITTED `graphs/starter/
verify-fix.toml` as the reference case (chosen because it is the only
starter graph exercising loop_budget + route + join in one place):

```
fix [agent] (entry) (loop_budget=3)
 |-- ok --> test

test [tool: bash]
 |-- ok --> gate

gate [route: $.payload.exit_code]
 |-- pass --> done
 |-- fail --> retry

retry [transform: $.payload]
 |-- ok --> fix
 ^-- loops back to "fix" (budget=3)

done [transform: $.payload]
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `show_renders_linear_graph_with_no_branches` | a 2-node `a -> b` graph | two rows, `a`'s row has one `-->` line to `b`, `b`'s row has none |
| `show_renders_route_fanout_as_multiple_arrows` | `verify-fix.toml`-shaped `gate` node with 2 rules | `gate`'s row emits exactly 2 `|-- <port> -->` lines, one per rule/port |
| `show_annotates_non_default_join_policy` | a node with `join = "any"` | row includes `(join: any, ...)`; a plain `All`-join node shows no annotation |
| `show_marks_entry_node` | any graph | the `entry` node's row (and only that row) has `(entry)` |
| `show_renders_loop_back_reference` | `verify-fix.toml` (real file, already in repo) | output includes a line `^-- loops back to "fix" (budget=3)` after `retry`'s row, and does NOT put `fix`/`retry` in a lane-crossing mess |
| `show_rejects_invalid_graph_like_lint_does` | a graph with a V0-class error (unknown kind, dangling edge) | non-zero exit, same class of error message `graph lint` would give, no partial render |
| `show_output_is_deterministic` | same file rendered twice | byte-identical stdout both times |

## Exit Criteria
- [ ] `cargo test -p pii-graph graph_show::` (or module of choice) — all
      pass, covering the TDD contract above
- [ ] `cargo test --workspace` — 0 failures
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors
- [ ] `pii graph show graphs/starter/verify-fix.toml` run manually
      against the live the host install, output visually matches the
      Architecture section's worked example (modulo exact spacing)
- [ ] `pii graph show` against EVERY starter graph in `graphs/starter/`
      (except the known-bad `crash.toml`, same exclusion `mvp-verify.sh`
      already uses) succeeds with exit 0

## Guardrails
- ASCII-only in v1 — no Unicode box-drawing glyphs (`├─┐`/`├─╯`/`●`).
  This matches the standing "no emojis, plain-ASCII" convention already
  applied to `pii tui`, and sidesteps the exact terminal-font/glyph
  rendering concern already discussed and resolved for the TUI. A
  Unicode "pretty" mode can be a LATER opt-in flag, never the default.
- Do NOT attempt to force loop back-edges through the git-style lane
  algorithm — render them as a separate annotation line, per the
  Research basis section. This is a deliberate, considered design
  choice, not a shortcut: cramming cycles into lanes is the exact
  failure mode the research surfaced in git's own graph rendering on
  merge-heavy histories.
- Do NOT add a live run-state overlay or a trace-unrolling renderer in
  this spec — v1 is static-structure-only, no daemon/socket dependency.
  Per the "Why git never actually draws a cycle" section above, a
  follow-up trace-mode renderer (unrolling loop iterations as
  sequential rows, exactly like `git log` unrolls repeated work instead
  of drawing a cycle) is the RIGHT eventual answer for viewing loops
  post-run, not just a deferred nice-to-have — but it is still a
  separate spec, kept out of this one to keep this change small and
  self-contained.
- Do NOT add a `--format=mermaid`/other output format in v1 — one
  format, done well, first. Revisit only if real usage shows the ASCII
  form is insufficient for a specific need (e.g. pasting into
  documentation).
- Do NOT reimplement cycle detection — reuse `sched::loop_helpers::
  is_in_cycle`, already correct and tested.
- Do NOT touch `pii-server`, `pii-model`, or any agent/memory code —
  this is entirely contained to `pii-graph` (new rendering module) and
  `pii-cli` (new subcommand wiring), independent of the global-memory
  work in `specs/09-global-memory.md`.

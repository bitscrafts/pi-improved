# Porting vscode-git-graph to Pii's Web Viewer — Feasibility Study

**Date**: 2026-09-10 · **Author**: research agent (delegated) · **Project**: /projects/pi-improved
**Status**: DRAFT — research only, no implementation authorized.

## 0. Summary / Recommendation

Build a **server-rendered graph view inside the existing `pii-web` crate**:
a new `GET /api/runs/:id/graph` endpoint that turns a run's journal
(`run.trace`) into a small JSON graph-description (rows + lanes + edges,
computed in Rust, reusing the lane-allocation logic already designed in
`crates/pii-graph/src/render.rs` and `specs/10-graph-visualize.md`), and a
vanilla-JS `<canvas>` or SVG renderer added to `index.html` that draws it —
no WASM framework, no new crate, no build step. Configurable columns and
level-of-detail are client-side view state (localStorage), not new wire
protocol: the existing `run.trace` payload already carries every field a
column could show. This fits Pii's existing architecture (axum + embedded
single-file SPA, viewer stays read-only) with the least new surface area.
Do not reach for a Rust/WASM frontend framework (yew/leptos/dioxus) — see
§2.2 for why that is over-engineering for this problem.

## 1. Feature mapping — vscode-git-graph → Pii's data model

vscode-git-graph renders a **repository's commit history** as a webview
panel: one row per commit, columns for the graph lanes, message,
date/author, commit hash; commits are laid out via a lane-allocation
algorithm so branches fork into new lanes and merges converge lanes back
together; clicking a commit opens a detail pane (diff, parents, refs);
there's a search/filter box, a "load more" pagination cutoff, and settings
controlling which columns show, date format/type (author vs commit date),
commit ordering, and graph line style.

Pii has **no single analogue of "commit history"** — it has two nested
levels of event-sourced state, and the honest mapping is a two-level graph,
not a flat one:

| vscode-git-graph concept | Pii analogue | Source |
|---|---|---|
| Commit (one row) | A journal event at one `seq` (`NodeStarted`, `NodeFinished`, `NodeFailed`, ...) | `pii-abi::RunEvent`, `crates/pii-abi/src/journal.rs` |
| Branch (a lane) | A node's repeated executions across loop iterations, OR a parallel batch member within one `TickStarted` | `crates/pii-graph/src/run.rs` tick loop; `loop_budget` cycles |
| Merge (lanes converge) | A `JoinPolicy::All`/`Any`/`Quorum` fan-in — multiple `NodeFinished` events feeding one downstream `NodeStarted` | `crates/pii-graph/src/spec.rs::JoinPolicy` |
| Fork (lanes diverge) | A `route` node's fan-out, OR a parallel batch (`TickStarted { batch, width }`) dispatching >1 node | `render.rs::render_block` (`|-- <port> --> <to>` lines) |
| A second repository / submodule | A **child run** spawned by a `spawn` node (`RunEvent::RunSpawned { child_run }`) — an entirely separate journal, own `seq` space | `crates/pii-server/src/nodes/spawn.rs`, `pii-abi/src/journal.rs:157` |
| Commit hash | `value_hash` on `NodeFinished` (content-addressed) | `RunEvent::NodeFinished.value_hash` |
| Author | N/A directly — closest is the node's `kind` (`agent`/`tool`/`route`/...) or, for `agent` nodes, the resolved model (`RouteDecision` wire event carries `step`/`model`) | `pii-proto/src/event.rs::Event::RouteDecision` |
| Date/time column | `at_ms` (ms since run start) on `NodeStarted`/`NodeFinished`; `dur_ms` for elapsed | `RunEvent::NodeStarted.at_ms`, `NodeFinished.dur_ms` |
| Commit message | For `agent`/`tool` nodes, the `value` payload (JSON) or, once wired, `ToolCall`/`ToolResult`/`AssistantDelta` wire events | `pii-proto/src/event.rs::Event` |
| Tags/refs decorations | Run status pills (`Running`/`Suspended`/`Succeeded`/`Failed`) and terminal disposition (`Terminal::Success/Failure/Suspended`) | `pii-abi::RunStatus`, `Terminal` |
| Uncommitted changes row | The live tail of a `Running` run (events not yet terminal) — rendered the same way, distinguished by "still growing" | n/a — purely a UI state, not a new data concept |

**What "a git graph of the implementation process" concretely looks like**:
one row per journal event, in `seq` order (Pii already orders by `seq`, the
same anchor the TUI's `[seq 0x…]` indicator and `pii graph trace` use), with
a lane column showing which node the event belongs to, fan-out/fan-in
rendered as lane splits/merges exactly as `pii graph show`'s ASCII renderer
already does for the STATIC graph definition — except this view is of the
**live/historical trace**, i.e. exactly the "v2 trace-mode renderer" that
`_docs/06-research/006-pii-graph-engine.md` §8.3-8.4 and
`specs/10-graph-visualize.md` already identified as the *correct* long-term
answer, and deliberately deferred as a separate spec. **This research task is,
concretely, that deferred follow-up spec's design phase**, extended to a
web (not terminal) rendering surface, plus a second axis vscode-git-graph
doesn't have at all: child-run spawning, which Pii needs and git doesn't.

Crucially — and this is the part git-graph's model does NOT need to solve —
Pii's trace has **no cycles to draw**, by the exact git-history argument
already recorded in `006-pii-graph-engine.md` §8.3 ("git never renders
structure, it renders history"): each `loop_budget` iteration is a *distinct*
set of events at strictly increasing `seq` (`fix#1, test#1, retry#1, fix#2,
...`), so a trace-mode renderer needs zero special-case loop handling — only
the STATIC graph-definition view (`pii graph show`, already shipped) needs
the `^-- loops back to` annotation. This materially simplifies the port:
vscode-git-graph's lane algorithm handles forks/merges only (never cycles)
because git history never cycles either — and neither does Pii's trace.

## 2. Rust implementation options

### 2.1 Where layout math should live

Regardless of rendering technology, the **lane-allocation algorithm**
(which column each event's node claims, when a lane opens for a fork, when
two lanes converge on a join) should be computed **once, in Rust, on the
server**, not duplicated in JS. Two reasons:
- `crates/pii-graph/src/render.rs::topo_order` + the backward-edge/loop
  detection already exists and is unit-tested (`graph_show` module) — reuse
  it for the static definition view, and its sibling logic (row-per-`seq`
  instead of row-per-`node-id`) is a small, mechanical extension for the
  trace view.
- Keeping layout in Rust means the JS side only ever draws pre-computed
  `{row, lane, kind, ...}` records — it never needs graph algorithms, which
  keeps the embedded SPA (currently ~145 lines, `pii-web/src/index.html`)
  small and dependency-free, matching R4's "no external assets" contract
  (`crates/pii-web/src/main.rs` test `spa_has_no_external_asset_references`).

### 2.2 (a) Server-side SVG/HTML render vs (b) Rust/WASM frontend vs (c) JS frontend fed by a Rust API

**Recommendation: (c) — a small vanilla-JS renderer (canvas or SVG) fed by
a Rust JSON API**, NOT (a) full server-side SVG generation and NOT (b) a
Rust/WASM framework. Reasoning:

- **(b) WASM framework (yew / leptos / dioxus / sycamore) — reject for this
  feature.** All four are reasonably mature for CRUD-style apps, but every
  one of them means: a new build toolchain (`trunk`/`wasm-pack` + a JS glue
  loader script), a `.wasm` binary asset the SPA would have to embed or
  fetch (breaking the current "one self-contained `index.html`,
  `include_str!`, no external script src" invariant tested by
  `spa_has_no_external_asset_references` — a `.wasm` file IS an external
  asset unless inlined as base64, which bloats the binary and is exactly
  the kind of complexity `pii-web`'s design deliberately avoided), and a
  second language-toolchain surface for a project whose stated philosophy
  (`crates/pii-web/src/main.rs:1-6`) is a minimal, dependency-light proxy
  viewer. None of the four WASM frameworks has a canned git-graph/DAG
  canvas widget — you would hand-roll canvas/SVG drawing in Rust-via-WASM
  either way, which is strictly more machinery than hand-rolling the same
  ~150 lines of drawing code in JS that already runs natively in every
  browser with zero build step. WASM buys type-shared-with-backend safety;
  it does not buy graph-drawing capability neither side already lacks.
- **(a) Full server-side SVG generation — reject as the primary mechanism,
  but keep as an option for a "static snapshot" / print view.** Pii could
  render the whole graph to an `<svg>` string in Rust and serve it as
  `GET /api/runs/:id/graph.svg`. This is simple and gets column/detail
  toggles wrong for free (every toggle = a new render request = a round
  trip), which is a poor interaction fit for something meant to be
  interactively filtered/hovered/clicked (vscode-git-graph's whole value is
  fast client-side interaction: hover a commit, click to inspect, filter
  by branch). Reasonable as a *secondary* "export static image" affordance
  later, not the primary interactive view.
- **(c) Rust API + vanilla JS — recommended.** Matches the existing
  `pii-web` pattern exactly: a new `/api/runs/:id/graph` endpoint returns a
  small JSON structure (rows, lanes, edges — already lane-assigned by Rust
  per §2.1), and ~150-250 lines of new JS in `index.html` draws it on a
  `<canvas>` (git-graph-style: circles for events, lines for lanes) the
  same way vscode-git-graph itself draws its graph column (it also uses an
  HTML5 canvas, not SVG, for the lane graphics, with an HTML table for the
  columns). Interaction (hover, click-to-inspect, column visibility,
  detail-level toggle) is trivial client-side JS/CSS — no new dependency,
  no build step, keeps the "self-contained SPA" test green by construction.

### 2.3 What changes in `pii-web`

- New route: `GET /api/runs/:id/graph?detail=summary|full` — calls
  `run.trace` (already exists, `Method::RunTrace`) then a NEW pure Rust
  layout function (not a new RPC method — `pii-web` already has the full
  trace; lane assignment is a pure function over it, same shape as
  `pii-graph::render::topo_order`, but it can live in `pii-web` itself
  since it's presentation logic, not execution logic — or in `pii-graph`
  as a `trace_render` sibling module to `render.rs` if the lane math is
  judged reusable by `pii graph trace --show` too, which spec 10 §"Why git
  never draws a cycle" already flagged as the natural follow-up CLI
  feature; sharing one implementation between the future CLI feature and
  the web view is worth doing once both exist).
- No daemon/server (`pii-server`) change needed at all — `run.trace`
  already returns the full ordered event list; everything else is a pure
  transform of data `pii-web` already has in hand after R2's existing
  proxy call. This keeps the change inside `pii-web` + optionally
  `pii-graph`, matching spec 19's guardrail pattern of not touching
  `pii-server`/`pii-net` for viewer-only features.

## 3. Configurable columns + level of detail

**Key insight: no new wire protocol is needed.** `run.trace`'s existing
`RunEvent` payload already carries every field a candidate column needs
(node id, port, status/kind implied by variant, `at_ms`, `dur_ms`,
`value`/`value_hash`, `error`/`retryable`). Column selection is therefore
a **pure client-side concern**: a checkbox row (à la vscode-git-graph's
column-visibility right-click menu) toggling which computed fields render
per row, persisted in `localStorage` (no server round trip, no new state
in `pii-web`, which currently has zero persistent state of its own by
design — `AppState` is just the socket path).

Candidate columns (all already present in `RunEvent`/`WireRunHeader`, no
new data needed): **node id**, **kind** (agent/tool/route/...), **port**
taken, **status** (finished/failed/cancelled/dead), **at_ms** / **dur_ms**,
**value preview** (truncated JSON), **model** (for `agent` nodes, sourced
from the `RouteDecision` wire event when live-streaming, or absent in a
pure `run.trace` replay since `RouteDecision` is currently push-only — see
gap below). One real gap: **`RouteDecision` (model routing) is a live SSE
event (`pii-proto/src/event.rs::Event::RouteDecision`), not a journal
event** — it does not appear in `RunEvent`/`run.trace`'s replay. If a
"model" column is wanted for COMPLETED runs (not just live ones), either
(a) accept the column is blank for old runs (simplest, honest about what
data exists), or (b) a future spec adds a journal-persisted
`RouteDecision`-equivalent `RunEvent` variant — out of scope here, flag
only.

**Level of detail** maps directly onto vscode-git-graph's own "load more
commits" pagination concept, adapted to Pii's two natural granularities:
- **Summary**: one row per node id (like `pii graph show`'s static view) —
  collapses all of a node's loop-iteration executions into one row showing
  final status + iteration count. Cheap, matches the "structure" view.
- **Full**: one row per journal event at its own `seq` (the trace-unrolled
  view from §1) — every loop iteration, every intermediate `NodeStarted`/
  `NodeFinished` pair, shown separately. This is the literal git-log
  analogue.

Both views are computable from the exact same `run.trace` payload
client-side (or server-side per §2.1) — detail level is a rendering mode
switch, not a different query.

## 4. Recommendation — concrete architecture

```
Browser (index.html, vanilla JS, unchanged build-free model)
   |
   | GET /api/runs/:id/graph?detail=summary|full   (NEW)
   v
pii-web (axum, crates/pii-web/src/main.rs)
   |  calls run.trace (Method::RunTrace, EXISTING) over the UDS proxy
   |  runs a NEW pure layout fn: RunEvent[] -> {rows, lanes, edges} JSON
   v
daemon UDS socket (pii-server) — no server-side change
```

1. **New endpoint** `api_run_graph` in `crates/pii-web/src/main.rs`,
   sibling to the existing `api_run_trace` — same `call_daemon(..,
   Method::RunTrace, ..)` call, then a pure Rust transform (lane
   assignment via a `trace_render`-style function, extending the
   topo/backward-edge logic already proven in `pii-graph/src/render.rs`)
   before returning JSON. No new `Method`, no `pii-server` change.
2. **Rendering**: add a `<canvas>`-based graph column plus an HTML table
   for the row data to `index.html`'s `renderRun` view (currently just a
   `<pre>` dump of the raw trace JSON, `crates/pii-web/src/index.html:82-86`)
   — draw lanes/nodes/edges from the new endpoint's JSON, keep the raw-JSON
   `<pre>` as a fallback/debug toggle.
3. **Columns + detail**: pure client-side controls (checkboxes +
   summary/full radio), `localStorage`-persisted, operating on the same
   payload — no new RPC surface, no daemon change.
4. **Read-only preserved**: the new endpoint is a `GET`, calls only
   `run.trace` (an existing read RPC), and adds no write path — fully
   consistent with spec 19's G7 VIEWER-ONLY guardrail. No auth model change
   needed since nothing new is mutable.
5. **Live runs**: for a `Running` run, `RunSpawned`/`NodeFinished` etc.
   already flow over `event.subscribe`; spec 19 designed (but the current
   `crates/pii-web/src/main.rs` does not yet implement) an `/api/events`
   SSE bridge — wiring that up is a prerequisite for a "live-updating"
   graph view and should land as its own small spec before or alongside
   this one, since today's `pii-web` only polls `/api/health` every 2s and
   has no SSE/live-event route at all (**verify this gap directly**: `grep
   -n "EventSubscribe\|sse\|/api/events" crates/pii-web/src/main.rs`
   returns nothing as of this writing).
6. **Child runs** (`RunSpawned`): render as a link/button on the spawning
   node's row to navigate to `#/run/<child_run>` (its own separate graph
   view) rather than trying to draw two runs' journals as one merged
   lane-set — keeps the lane algorithm simple (one `seq` space per view)
   and matches how a user actually thinks about a spawned sub-run (a
   separate history, like a git submodule, not an inline branch).

## 5. Suggested phased scope (not a spec — a sizing note)

- **Phase 1** (small): summary-detail graph view for one run, no columns
  UI yet, no SSE — a static `pii graph trace --show`-equivalent rendered
  as canvas instead of ASCII. Proves the lane-transform + canvas drawing
  end to end.
- **Phase 2**: full-detail (per-`seq`) mode, column visibility toggles,
  click-to-inspect a row (expand the raw `NodeFinished.value` JSON inline
  — vscode-git-graph's commit-detail pane equivalent).
- **Phase 3**: SSE live-updating view for `Running` runs (needs the
  `/api/events` bridge spec 19 designed but that the current `pii-web`
  does not implement yet — confirm before starting, do not assume it
  exists), child-run navigation links.

Each phase is independently shippable and viewer-only throughout.

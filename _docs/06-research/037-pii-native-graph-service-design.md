# A native "pii graph" service — design answer (2026-09-19)

Answers: *can pii maintain a graph + services like graphify internally, as a
pii graph?* Companion to 035 (whether) and 036 (external wiring, now applied).

**Short answer: yes, and pii is unusually ready for it** — because the
machinery is not hypothetical. `memory` is already a graph service in exactly
this shape, registered node kind + daemon method + shared tool. A
code-knowledge graph is the same pattern over a different corpus.

---

## The three graphs pii already has (do not conflate them)

| | what it is | where |
|---|---|---|
| **Execution graph** | the DAG being *run* — spec, TOML loader, expression language, scheduler, journal, static validator | `crates/pii-graph` |
| **Concept graph** | concept co-occurrence + label-propagation communities over **memory docs**, fused into retrieval by RRF. "Zero model calls anywhere." | `crates/pii-memory/src/graph_lane.rs` (F4.8, 102 lines) |
| **Code graph** | what graphify produces. **Does not exist in pii today.** | — |

So the question is not "can pii host a graph" — it hosts two — but whether the
**third** belongs inside. The answer is yes, and the seam is already built.

---

## The precedent that makes this cheap: `memory`

`memory` is the proof that a graph/service node belongs *in the daemon*, not in
`pii-graph`. Its own module doc states the rule:

> Registered alongside `agent`/`tool` in `crate::engine::registry` because both
> need a resource the engine owns (the approval table, the memory store) that
> `pii-graph` itself must not know about.

That is precisely the code-graph's situation: it needs an **index the engine
owns and keeps fresh**, which `pii-graph` (a pure, stateless scheduler library)
must not know about.

The pattern has four parts, all already exercised by `memory`:

1. **A `NodeFactory`** registered in `engine/registry.rs` —
   `reg.register(Arc::new(nodes::memory::MemoryFactory { store: memory }))`
   (line 55; the factories list is `agent`, `tool`, `gate`, `heal`,
   `heal-record`, `memory`, `crash`, `worker`, `spawn`).
2. **`effects()` and `required_caps()`** — `MemoryFactory` declares
   `Effects::MUTATES_EXTERNAL` and `CapKind::empty()`; a code-graph reader would
   declare read-only effects and no caps.
3. **`ports()`** — it returns named ports (`ok`/`err`), which is what makes a
   node *branchable* in a graph.
4. **A daemon method** in the closed wire vocabulary (`pii-proto/src/method.rs`):
   `memory.search`, `memory.put`, `memory.consolidate` — a code graph would add
   `graph.code.query` / `.path` / `.explain` in the same list.
5. **One shared tool implementation** — `pii-tools/src/memory.rs` exists so
   *"the ONE implementation shared by every `ToolSet::builtin` consumer:
   `pii-mcp` … and `pii-server`'s own agent loop"*. Both dial the daemon over
   UDS rather than reaching into `Arc<MemStore>` — *"one code path, one
   capability gate, one bound."* A code graph tool follows that exactly.

**Nothing about that list is new work.** It is the existing contract.

---

## What makes a code graph a *good* fit rather than a bolt-on

### It is a third corpus for the same retrieval question

`pii-memory` already fuses BM25 + i8 vectors + a graph lane by RRF over
*memory docs*. A code graph is the same shape — a corpus plus a graph lane —
over *source files*. amem already carries `petgraph` in-workspace, so the graph
representation is not a new dependency class.

### The critical lesson from the failover work applies directly

The stored architectural correction is explicit:

> A graph cannot branch on a reason `Pii` never produces … **the model node must
> return a successful `Outcome` carrying `{status, reason, provider}` instead of
> `Err`.** Proof this works: the escalation ladder already exists because the
> **tool node reports `exit_code` as payload** rather than failing.

So if a code-graph node is to be *branchable* in a workflow — "if the symbol is
unreachable, route to the fix lane" — it must **return a successful `Outcome`
with the finding in the payload**, not `Err`. That is a design constraint
discovered *before* writing the node, and it comes from a mistake we already
made once.

Corollary: a code-graph node that merely answers questions is a **tool**
(effect-free, read-only). A node that makes a *judgement* the graph branches on
is a **route payload**. Those are different designs; pick deliberately.

### `Everything is a graph` means this is not foreign

The same note: *"in pii as everything is a graph!"* — and that the static,
statically-validated graph makes safety rules *rule order*, so
"a spec arguing itself onto the cheap model" is *"STRUCTURALLY IMPOSSIBLE, not
merely discouraged."* A code graph extends the same discipline to code
structure: instead of an agent grepping and *asserting* a symbol is unused, the
graph lane answers it, and — crucially — **the answer is evidence, not
opinion** (`EXTRACTED` vs `INFERRED` edge tags give exactly the provenance
discipline our gates want).

---

## Two concrete shapes, and why the second is the one worth building

### Shape 1 — graphify-in-a-box (low value, do not do this)

Wrap `graphify-out/graph.json` in a pii node kind and call it a service. Cheap,
but it makes pii depend on a Python tool's artifact schema, its extractor's
quirks (the empty `Cargo.toml` node, the duplicate-node notes I measured), and
its 15 MB output. **This is the wrong answer** — it imports a foreign format
and a foreign bug surface into our verification seams.

### Shape 2 — a pii-native code graph, scoped to what our gates need (the one)

Start from the **gate**, not the browsing. `xtask reachability-check` already
exists and its own doc admits its weakness:

> *"Pure text search is sufficient: we check 'does this string appear as a call
> outside tests', **not graph structure**."*

It exists for the **B1 class** (`ollama::provider_from_env` with zero non-test
callers) and the **M3 class** (`Supervisor::invoke` called only from tests) —
and a string search cannot tell a **call** from a string literal, a doc
comment, or a same-named field. That failure mode — *"a check that looks
rigorous while testing the wrong surface"* — is already documented **twice** in
`pidag/docs/FINDINGS.md`.

A real call graph turns that gate into: *"is there an `EXTRACTED` `calls` edge
from a non-test file to this symbol?"* Same living-list interface, strictly
stronger oracle, and it is the **same class of upgrade as F-11** (a typed
structural fact replacing a string-matching approximation).

That is a *much* smaller and more defensible first step than "a code graph
product", and it earns its place by strengthening a verification seam — the
thing pii actually is.

### Then, if it proves out

Expose the same graph as `graph.code.*` daemon methods + a shared tool. Because
the seam is the `memory` seam, an agent node, an MCP client, and a CLI all get
it through one code path with one capability gate — which is how `pii-mcp`'s
wholesale-builtin-registration concern (fable Finding 6/`ToolSet::builtin`) gets
answered too: **tool-set narrowing per turn** becomes possible when the corpus
is queryable instead of enumerated.

---

## Cost and policy fit (the honest constraints)

- **No LLM required.** graphify's own model proves a code-only corpus needs no
  model calls at all (it refuses to read `ANTHROPIC_API_KEY`/`OPENAI_API_KEY`).
  A Rust AST pass is deterministic and free — which matters for a project that
  rations its Anthropic budget.
- **Dependency discipline is the real risk.** tree-sitter drags in grammar
  crates, against a repo with a strict `deps-check` and a **450-line effective
  size gate**. Mitigations, in order: scope **Rust only** (not graphify's ~40
  languages); reuse `petgraph` (already in-workspace via amem); and if the
  grammar crates still fail policy, keep the parser in **`xtask`** (which is
  outside `crates/` and already hosts every gate) rather than in a new crate.
- **`#![forbid(unsafe_code)]` is workspace-wide** (`Cargo.toml:54`); a
  tree-sitter Rust crate must be checked against that before committing to it.
- **It must not become a browsing toy.** Veto condition, restated: if it cannot
  make a **gate** measurably stronger, it does not belong in pii. pii's value is
  its verification seams.

---

## Applied today (external wiring, both repos)

Done and committed, per the 036 recommendations:

| | pii | pidag |
|---|---|---|
| `graphify-out/` gitignored | ✅ | ✅ |
| `CLAUDE.md` + `.claude/` (rules + PreToolUse soft nudge) | ✅ `22152e7` | ✅ `d93a6a4` |
| `AGENTS.md` always-on rules | ✅ additive | ✅ new file (points to `CLAUDE.md` as SSOT) |
| Hermes skill | ✅ | ✅ (global) |
| pi skill | ✅ | ✅ (global) |
| Graph built, local, zero LLM | ✅ 4187 nodes / 9646 edges / 216 communities | ✅ |
| Git hooks | **not installed** (deliberate — clean-tree discipline) | same |

Note on pidag: it had **no tracked `AGENTS.md`** — graphify created one
containing only its own block. I rewrote it to point at `CLAUDE.md` as the
single source of truth, so the new file is a signpost rather than a second,
competing instruction set.

---

## Recommended sequence

1. **Now** — external tooling applied (done). Use `graphify query/path/explain`
   as a reader during this week's Opus-review work. Free, no gate impact.
2. **Next** — use it for one real decision: feed the pii god-node table
   (`PiiError` 406 edges, 2.4× the runner-up) into the Opus package
   `_docs/06-research/030-OPUS-START-HERE.md` as evidence for the harness-graph
   and F-13-scope questions.
3. **Then** — spec the **`reachability-check` upgrade** (Shape 2, step one):
   Rust-only, `petgraph`-first, `EXTRACTED`-edges-only, same living list. This
   is the piece that earns native code because it strengthens a gate.
4. **Later** — if step 3 holds, expose `graph.code.*` methods + a shared tool
   via the `memory` seam, and let it answer `pii-mcp`'s tool-set-narrowing.
5. **Never** — a pii node kind that wraps graphify's `graph.json` (Shape 1), or
   any wiring into an exit criterion that the graph can satisfy vacuously.

## Open questions

- Does a Rust-only tree-sitter parser pass `deps-check` + the 450-line gate, or
  must it live in `xtask`? (Decides crate placement.)
- `EXTRACTED`-only edges for the gate (hard oracle) or allow `INFERRED` with a
  confidence threshold (broader, weaker)? I would take `EXTRACTED`-only first.
- Is the code graph a **new lane in `pii-memory`** or a **sibling store** in
  `pii-store`? `graph_lane` argues for the former; the 450-line gate may argue
  for a new module.
- Should the graph be **queryable at runtime by agents** (a tool) or **only at
  gate time** (xtask)? Runtime is more useful; gate-only is far cheaper and
  cannot drift into a toy.

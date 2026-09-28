<!-- MERGED-CORPUS 2026-09-28T20:00:57Z: this file is the canonical document for its subject.
     It absorbed 2 document(s) which are now sections below, kept verbatim.
     Why: the two graphify docs are the research that the design doc decides on.
     Originals archived at _docs/99-archive/06-research-merged-2026-09-28/ (nothing deleted). -->

# A native pii graph service — the design answer, with the tooling research behind it

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


---

## [merged] Graphify — research: (A) tooling for our pii work, (B) a native pii feature (2026-09-19)

<!-- Absorbed from `037-pii-native-graph-service-design.md` on 2026-09-28T20:00:57Z during the SSOT corpus merge.
     Content preserved verbatim. Original moved to 99-archive/06-research-merged-2026-09-28/. -->

Status: research findings + a **measured** trial. No spec, no code committed.
Repo: `github.com/Graphify-Labs/graphify` (YC S26, Apache-2.0-ish CLI on PyPI
as `graphifyy`). Every number below is from a run I did on pii, not the README.

The question has **two distinct situations**, kept separate throughout:

- **(A) graphify as external tooling** on our pii work — would it help us and
  the Opus review?
- **(B) a native pii solution** — is a code-knowledge-graph a worthwhile
  *feature* inside pii itself?

---

## What graphify is

Turn a codebase (plus docs, PDFs, images, video) into a **queryable knowledge
graph**, then **query it instead of grepping**. Three output files:

```
graphify-out/
├── graph.html       interactive, clickable, communities coloured
├── GRAPH_REPORT.md  highlights: god nodes, surprising connections, questions
└── graph.json       the full graph — query without re-reading files
```

Key design claims, and whether they held up:

| Claim | Verdict |
|---|---|
| Code parsed with **tree-sitter AST, no LLM, fully local** | **True** — `--code-only` ran zero LLM calls and made zero network calls |
| **Every edge is tagged** `EXTRACTED` (explicit in source) or `INFERRED` (resolved) | **True** — visible in real output |
| **Not a vector index** — a real graph you traverse | **True** — `explain` / `path` / `query` are graph traversals, not similarity |
| 20+ assistants: Claude Code, Codex, Cursor, Gemini, **Hermes**, **Pi** | **True** — `graphify install --platform hermes` and `--platform pi` both exist |
| MCP stdio server (`graphifyy[mcp]`) | **True** — `graphify-mcp` binary was installed |

Commands that matter: `extract --code-only` (local), `explain <sym>`,
`path A B`, `query "<question>" --budget N`, `cluster-only`, `--update`
(incremental), `--watch`, `--mcp`, `hook install` (post-commit/post-checkout),
and `reflect` (write lessons back).

---

## (A) graphify as tooling on our pii work — I RAN IT

Installed `uv tool install graphifyy`, then on `/projects/pi-improved`:

```
graphify extract . --code-only
```

**Measured result, fully local, zero LLM cost, no API key:**

| | |
|---|---|
| Code files parsed | **269** |
| Nodes | **4,187** |
| Edges | **9,646** |
| Communities | **216** |
| Non-code skipped | 135 (131 docs, 4 papers) — correctly out of `--code-only` scope |
| Wall time | a few minutes, 32 parallel tree-sitter workers |

Its **god nodes** on pii (most-connected concepts):

```
1. PiiError   406 edges      6. Response      42
2. Ctx        168            7. GraphNode     42
3. GraphSpec   60            8. Engine        37
4. Store       44            9. NodeRegistry  30
5. RunEvent    42           10. CapKind       29
```

**This independently validates our own priorities.** `PiiError` at 406 edges —
2.4x the runner-up — is the single most connected thing in the codebase, which
is exactly why F-11 (structured error taxonomy) was the right call and why
spec-66's `store_op` constructor matters. That is a **machine-checked
confirmation of a judgement we made by hand.**

It also surfaced *surprising connections* with confidence tags, e.g.
`eval_query() --calls--> rrf()` `[INFERRED]` (`xtask/src/memory_eval.rs` →
`crates/pii-memory/src/rrf.rs`).

### Does it help the Opus review? — Yes, materially, and here is the honest why

The Opus review's job is judgements on `_docs/06-research/{029,030,032}`: the
coding-harness graph, F-13 scope, modularity, the Chromecast module, the
`_docs/` reorg. Those are **architecture questions about a 20-crate
codebase**. Today Opus answers them by reading files, and the review docs
themselves had a **measurement error** (the raw `wc -l` size list, corrected
only when fable re-derived the real metric). A code graph attacks exactly
that failure mode:

1. **The god-node list is evidence, not opinion.** "Which abstractions are
   load-bearing?" becomes a number, not an impression. `PiiError` 406 edges
   vs `GraphSpec` 60 is the kind of fact a review should argue from.
2. **`path A B` answers "what would this touch?"** — the question behind
   F-13 (module split) and the Chromecast-module decision, without a
   grep-and-read sweep.
3. **Inference vs extraction is disclosed**, so a reviewer can tell a real
   call edge from a resolved guess — which is precisely the discipline their
   reviews demand of the *code*, applied to the *analysis*.
4. **It is local and free**, so it costs nothing in the Anthropic budget that
   we are rationing. That is the decisive practical point this week.

### Honest limitations I measured (do not oversell)

- **One run produced an empty node.** `Cargo.toml` yielded zero nodes and was
  absent from the graph (the tool warned and asked for a bug report). Minor,
  self-reported.
- **Duplicate-node notes.** Two files (`pii-memory/tests/persistence.rs`,
  `pii-worker/src/control.rs`) produced a node twice under different labels;
  handled by dropping one, but it shows the AST extractor is not perfect.
- **Ambiguity is real.** `graphify explain "Supervisor"` matched **3 nodes in
  different files** and refused to guess, asking for `path::symbol`. Correct
  behaviour, but it means queries need disambiguation on a codebase this size.
- **Community labelling needs an LLM.** `cluster-only` warned that labelling
  needs the `openai` extra; it fell back to `Community N` placeholders. So the
  *local-only* path gives you the graph but **not** readable community names —
  that specific feature is not free.
- **`query` truncates by default.** The spec-65 query returned 102 nodes and
  showed 31 under an ~800-token budget, warning that 71 were cut. Useful only
  with a raised `--budget` or a narrower query.
- **`path` is directed.** My first `path` query returned *no path* and
  demanded `--undirected`. Direction-blind traversal is not the default.
- **It writes 15 MB into the repo.** `graphify-out/` was created inside
  `/projects/pi-improved`, untracked and **not gitignored** — squarely against
  our clean-root rule. I moved it to
  `/projects/_tmp/graphify-out-pii-2026-09-19/` and left the repo clean.
  **Any real adoption must write outside the tree or gitignore it.** The tool's
  own README flags the sibling hazard: output files invalidate Claude Code's
  prompt cache unless added to `.claudeignore`.
- **Independent comparisons are mixed.** A published head-to-head against
  CodeGraph found graphify used *more* tokens on targeted work on a known
  codebase (BFS ~1,500 tok vs semantic search 100-350 tok per hit) and lost on
  one timing test; another reviewer called the difference "quite small, almost
  indifferent". The pattern from both: **graphify is for orientation on
  unfamiliar ground; semantic search is for precision on known ground.**

**Verdict (A): adopt as optional, out-of-tree tooling.** Highest value for
the Opus review and for onboarding to unfamiliar parts of pii. Free and local,
so no budget objection. Do **not** wire it into the gate, and do **not** let
it write into the repo.

---

## (B) A native pii feature — the case is real, and partly already built

This is where the answer changes, because **pii already has most of the
ingredients**, which makes a native solution far cheaper than it looks.

### What pii already has

- **`pii-memory/graph_lane.rs` (F4.8)** — concept co-occurrence expansion +
  label propagation communities, "zero model calls anywhere". A *concept*
  graph over memory, fused into retrieval by RRF.
- **`pii-graph`** — a graph engine, but for **execution graphs** (spec, TOML
  loader, expression language, scheduler, journal, validator), not code.
- **`xtask reachability-check`** — a **module-reachability gate**: for each
  "declared capability → expected non-test caller" pair, assert the caller
  exists. Its own doc comment is the tell: *"Pure text search is sufficient:
  we check 'does this string appear as a call outside tests', **not graph
  structure**."*
- **`pii-mcp`** — an MCP server via `rmcp`, already re-exporting the builtin
  tool set.
- **`xtask` gates** — `deps-check`, `net-check`, `lint-files`,
  `reachability-check`, `doc-drift-check`, `silent-noop-check`, `proto-check`,
  `bench-gate`, all reachable through `cargo xtask ci`.
- **amem** (`/projects/amem`) already proves the pattern in Rust:
  `petgraph` + `graph.rs` + a concept graph, in a pure-Rust service.

### The sharpest native opportunity: upgrade `reachability-check` from text to graph

`reachability-check` exists precisely because of the **B1 class**
(`ollama::provider_from_env` with zero non-test callers) and the **M3 class**
(`Supervisor::invoke` called only from tests). Today it answers that with
**string search**, which cannot distinguish a call from a mention in a string
literal, a doc comment, or a struct field of the same name — and *that is
the exact "check that looks rigorous while testing the wrong surface" failure
mode already in `pidag/docs/FINDINGS.md`*, which bit us repeatedly.

A tree-sitter call graph would turn that gate from a heuristic into a real
edge query: *"is there an `EXTRACTED` `calls` edge from a non-test file to
this symbol?"* That is the same class of upgrade as F-11 (`StoreOp` replacing
free-text `Internal(format!)`): **replace a string-matching approximation with
a typed, structural fact.** It also directly serves the reachability gate's
stated purpose, with the same living-list interface it already has.

Note the honest trade: tree-sitter adds a dependency tree (grammar crates) to
a repo with a strict `deps-check` and a **450-line effective-size gate**. Any
native adoption must justify the dependency and stay inside the size policy —
which argues for reusing `petgraph` (already in-workspace via amem) and
scoping to **Rust only**, not graphify's ~40 languages.

### Should it be a *product* feature (agent-facing)?

The case: pii is an agent harness whose agents navigate a codebase with
`fs`/`search`/`bash`/`grep`. A structural "where is X defined / who calls X /
what changes if I edit X" tool is a *better primitive for a coding agent than
grep*, and pii-mcp already exposes tools to external clients. But be honest
about the sizing:

- **The Rust ecosystem is young here.** The alternatives found are early:
  `rustcodegraph` (1.2.x, Rust port of a TS CodeGraph, SQLite + MCP),
  `codegraph-rust` (deterministic tree-sitter → SQLite/FTS5, "no AI/LLM
  anywhere in the binary"), `code2graph` (still `0.0.0-beta.x`), and
  `codegraph-rust` by Jakedismo (surrealDB backend). None is a settled,
  obvious choice — which cuts both ways: no turnkey dependency to adopt, but
  also no incumbent to beat locally.
- **amem is the better host for a graph service.** It already has
  `petgraph`, a concept graph, a REST + MCP surface, and it is *designed* as
  a shared multi-agent service. A code-knowledge graph is arguably an
  **amem capability** (another retrieval lane over a different corpus) rather
  than a new pii crate.
- **The cheapest first step is not a new crate** — it is using graphify
  out-of-tree to answer one real question (the Opus review), and letting the
  reachability-gate upgrade be the first piece of native code if it proves
  worth it.

### Veto conditions (what would make (B) not worth it)

- If the only win is "nice for navigation" and it does not make a **gate**
  measurably stronger, skip it — pii's value is its verification seams, not
  its browsing.
- If it cannot stay inside the 450-line/`deps-check` policy, it fights the
  repo's own constitution.
- If amem can host it (it can), a new pii crate needs a reason amem is wrong.

---

## Ranked recommendations

**Situation (A) — tooling (cheap, do now):**
1. Keep `graphifyy` installed; run `graphify extract . --code-only` **into
   `_tmp/`** (or gitignore `graphify-out/`) against pii and pidag.
2. Feed the **god-node table + surprising-connections list** into the Opus
   review package (`_docs/06-research/030-OPUS-START-HERE.md`) as evidence for
   the coding-harness-graph and F-13-scope decisions.
3. Optionally `graphify install --platform hermes` / `--platform pi` so the
   skill is available in-session — but treat `/graphify` as a *reader* tool,
   never a gate.

**Situation (B) — native feature (sequenced):**
4. **First**: upgrade `xtask reachability-check` from string search to a
   tree-sitter call-graph edge query. Same interface, same living list,
   strictly stronger oracle — and it fixes an instance of a failure mode we
   have already documented twice.
5. **Then**: if that proves out, scope a Rust-only code-graph lane in
   **amem** (it has `petgraph`, REST + MCP, and is the intended shared
   service), rather than a new pii crate.
6. **Only then**: expose it as an agent-facing tool via `pii-mcp`, sized
   against the existing tool surface (`bash`/`fs`/`search`/`memory`/
   `worktree`/`sandbox`) so it earns its place rather than duplicating grep.
7. **Not now**: the coding-harness graph (`030`) stays a separate decision —
   a code-knowledge graph does not answer it.

## Open questions

- Does a tree-sitter **Rust-only** parser fit the `deps-check` policy and the
  450-line gate, or does it need to live in `xtask` (which is outside
  `crates/` and already hosts the gates)?
- Would the reachability gate accept `EXTRACTED`-only edges (no inference), to
  keep it a hard oracle rather than a best-effort one?
- Is amem's concept graph the same abstraction at a different corpus, or a
  genuinely different structure? (Decides 5 above.)
- Does graphify's `graph.json` schema map onto `petgraph` cleanly enough to
  reuse its output instead of writing our own extractor?


---

## [merged] Graphify integration design — discussion notes (2026-09-19)

<!-- Absorbed from `037-pii-native-graph-service-design.md` on 2026-09-28T20:00:57Z during the SSOT corpus merge.
     Content preserved verbatim. Original moved to 99-archive/06-research-merged-2026-09-28/. -->

Companion to `037-pii-native-graph-service-design.md`. That document
asked *whether* to use graphify; this one answers *how* it would be wired in.
**Every mechanic below was tested on this machine**, not read from the README.
No integration is committed yet — this is the discussion input.

The four questions, answered in order:

1. Self-contained via uv, with a gitignored `.graphify/`?
2. Tooling declared in **both** `CLAUDE.md` and `AGENTS.md`?
3. A hook that makes the agent read the graph first?
4. A skill in Hermes?

---

## 1. Self-contained with uv + a gitignored `.graphify/` — YES, verified

**Isolation (uv).** `uv tool install graphifyy` puts the whole thing in its own
environment at `/root/.local/share/uv/tools/graphifyy/` with shims in
`/root/.local/bin/`. It does not touch any project's `Cargo.toml`, lockfile, or
venv. Nothing to add to `deps-check`. Self-contained by construction.

**Output location — there is a first-class knob.** `GRAPHIFY_OUT` (and
`GRAPHIFY_OUT_NAME`) override the output directory. Defined once in
`graphify/paths.py` so the security/callflow path guards honour the same
override. **Tested:**

```bash
GRAPHIFY_OUT=/projects/pi-improved/.graphify graphify extract . --code-only
# -> wrote /projects/pi-improved/.graphify/graph.json: 4187 nodes, 9646 edges, 216 communities
# -> no graphify-out/ created (confirmed)
```

`graphify/paths.py:11` documents the intent verbatim: *"override with
GRAPHIFY_OUT env var for worktrees or shared-output setups."* So it was
designed for exactly this.

**Git hooks honour it too.** `.git/hooks/post-commit` line 45:
`_GFY_OUT="${GRAPHIFY_OUT:-graphify-out}"`, with a comment noting a literal
`graphify-out/` there *"let a [worktree] ..."* (the #1423 fix). So the
auto-rebuild follows `GRAPHIFY_OUT` correctly.

**But the always-on text does NOT — this is the one real gotcha.**
`graphify/always_on/agents-md.md` **hardcodes `graphify-out/` five times**, and
the generated `CLAUDE.md` section hardcodes it too. Worse, the PreToolUse hook
message is a fixed string: *"MANDATORY: graphify-out/graph.json exists."* So if
we relocate the graph to `.graphify/`, **the instructions tell the agent to
query a path that does not exist, while the hooks (correctly) use the real one.**

Three ways to resolve it, with honest trade-offs:

| Option | How | Trade-off |
|---|---|---|
| **A. Default name, gitignored** | keep `graphify-out/`, add to `.gitignore` | Zero conflict — text and hooks agree. Directory name is not dot-prefixed, so it is visible in the root listing (our clean-root rule tolerates it if gitignored, but it is not hidden). |
| **B. `.graphify/` + patch the text** | set `GRAPHIFY_OUT`, then edit the `graphify-out/` strings in `CLAUDE.md`/`AGENTS.md` to `.graphify/` | Hidden/self-contained as you wanted, **but the patch is overwritten by the next `graphify install` or upgrade** — a standing maintenance cost, easy to forget, and the failure is silent (agent reads a stale path). |
| **C. `.graphify/` + compat symlink** | real dir `.graphify/`, plus `graphify-out -> .graphify` symlink, both gitignored | Both the hardcoded text and the env var resolve. Self-contained and hidden-ish (one symlink in root). Slight complexity; path guards honour `GRAPHIFY_OUT` so they follow it. |

**My recommendation: A for now, C if the hidden dir matters.** Option B is the
one to avoid — a silent, upgrade-reset divergence between the instructions and
reality is exactly the "check that looks rigorous while testing the wrong
surface" failure mode we have already documented twice in pidag.

**Gitignore lines for option A** (also prevents the Claude Code prompt-cache
invalidation the tool's own README warns about):

```
graphify-out/
```

---

## 2. Both `CLAUDE.md` and `AGENTS.md` — YES, and they need two different commands

Correct that both are needed: Claude Code (and **its subagents**) read
`CLAUDE.md`; other agents — Codex, Cursor, Hermes, pi — read `AGENTS.md`. The
installer splits them accordingly, verified:

| Command | Writes | Hook? |
|---|---|---|
| `graphify install --project` | **`CLAUDE.md`** (root) + `.claude/CLAUDE.md` + `.claude/settings.json` (PreToolUse) + `.claude/skills/graphify/` | **Yes** — `Bash\|Grep` → `graphify hook-guard search`; `Read\|Glob` → `graphify hook-guard read` |
| `graphify agents install` | **`AGENTS.md`** (appends a `## graphify` section, preserving existing content) | **No** — its own output says: *"unlike Claude Code, there is no PreToolUse hook equivalent for Agents — the AGENTS.md rules are the always-on mechanism."* |

Tested against a scratch `AGENTS.md`: it appended cleanly and **left the
existing content intact** — important, because our repos have substantial
`AGENTS.md` files. The section is marker-delimited (`## graphify`), so it is
idempotent and removable (`graphify agents uninstall`).

**The always-on rules it injects are the right rules** — query before reading,
`path`/`explain` for relationships, `GRAPH_REPORT.md` only for broad review,
and `graphify update .` after code changes (AST-only, no API cost).

**One caveat worth knowing:** the `Bash|Grep` matcher fires on a `grep` too, so
with the hook live, *every* grep in the project emits a "query the graph first"
nudge. That is the intended nudge behaviour, but on our workflow (which greps
constantly) it will be noisy. `--strict` is worse: it *blocks* the first raw
source read per session (`GRAPHIFY_HOOK_STRICT=1/0`). Given how much we rely on
direct file reading for verification, I would run **soft nudge, not strict**.

---

## 3. A hook to read the graph first — YES, two distinct hooks

They are **different mechanisms** and both were tested:

**(a) PreToolUse hook (agent-level, Claude Code only).** Registered in
`.claude/settings.json`. Verified output on a `Read` of
`crates/pii-worker/src/control.rs`:

```json
{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":
 "MANDATORY: graphify-out/graph.json exists. You MUST run graphify before reading
  source files. ... This rule applies to subagents too — include it in every
  subagent prompt involving code exploration."}}
```

Note two things: it explicitly covers **subagents** (which is why the same
guidance must also live in `CLAUDE.md` — subagents get the file, not the
session), and it is **conditional** — it emitted *nothing* until a graph existed
at the path it checks. My first test was silent precisely because the graph was
at `.graphify/` and it was probing `graphify-out/`. That is the gotcha from §1
showing up as a real behaviour, not a theory.

**(b) Git hooks (repository-level, all agents).** `graphify hook install`
writes `post-commit` and `post-checkout` (13 KB each) and registers a
`graphify-out/graph.json merge=graphify` merge driver in `.gitattributes`. They
auto-rebuild the graph after commits and branch switches — AST-only, no API
cost. Per §1 they correctly honour `GRAPHIFY_OUT`. I installed and then cleanly
uninstalled both, and removed the `.gitattributes` line; repo verified clean.

**Honest warning about the post-commit hook in *our* workflow.** Our rule is
*workers never commit; the orchestrator commits.* A post-commit hook firing a
graph rebuild on the orchestrator's commit is fine — but it changes what a
"clean tree" means, and our pidag effect-verification seam depends on tree
state. The tool's own docs already flag the sibling problem: graph output
invalidating the Claude Code prompt cache. I would **start without the git
hooks**, using explicit `graphify update .` at deliberate moments, and add them
later only if the manual step proves to be the thing we forget.

---

## 4. A skill in Hermes — YES, already installed and loading

`graphify install --platform hermes` wrote `/root/.hermes/skills/graphify/`
(SKILL.md + 8 `references/`). `--project` variant targets
`<project>/.hermes/skills/graphify/`. I loaded it with `skill_view('graphify')`
and it is valid and available in this session.

What is actually in it (worth knowing before adopting): the skill is not a thin
wrapper — it is the **full pipeline runbook**, with Steps 0-9, an interpreter
guard, and its own honesty rules ("Never invent an edge", "Always show token
cost"). Its design intent is that the *host agent* is the LLM for semantic
extraction when no `GEMINI_API_KEY` is set — it explicitly refuses to read
`ANTHROPIC_API_KEY`/`OPENAI_API_KEY`. Relevant to our budget policy: for a
**code-only corpus it needs no LLM at all**, which is the mode we would use.

It also has a documented fast path: if `graphify-out/graph.json` exists and the
question is a codebase question, **skip the entire rebuild and go straight to
`graphify query`**. That is the behaviour the PreToolUse hook enforces, and it
is why the skill and the hook are complementary rather than duplicate.

Note for our stack: besides `hermes` it also supports **`--platform pi`**
(`graphify pi install`) — directly relevant, since pi is our workhorse.

---

## What I would actually do (proposal for your call)

Sequenced, reversible, nothing committed yet:

1. **Keep `graphify-out/` as the name; gitignore it.** (§1 option A — no
   text/hook divergence.)
2. `graphify install --project` for **CLAUDE.md + `.claude/`** (hooks and the
   Claude/subagent path) **+ `graphify agents install` for AGENTS.md** (the
   Codex/Cursor/Hermes/pi path). Commit both — they are the shared contract.
3. `graphify install --platform hermes` + **`--platform pi`** so the skill
   travels with the harnesses we actually run.
4. **Soft nudge, not `--strict`.** Build the graph once with
   `graphify extract . --code-only`; use `graphify update .` deliberately rather
   than installing git hooks at the start.
5. **Do not wire it into the quality gate or any exit criterion.** It is a
   reader/orientation tool. §B of document 035 covers the one place it *would*
   earn gate status — upgrading `xtask reachability-check` to a real call graph.

### Decisions I need from you

- **§1: option A (default name, gitignored) or option C (`.graphify/` + compat
  symlink)?** I recommend A. B is the one I would avoid.
- **§2: accept the grep-noise of the soft PreToolUse nudge?** It will fire on
  every `grep`/`Read` in the project. If that is too loud for our
  verify-by-reading workflow, we install the skill and the always-on rules but
  skip the hook.
- **§3: git hooks on or off to start?** I recommend off, explicit
  `graphify update .` instead — because our clean-tree/effect-verification
  discipline depends on knowing exactly what writes to the repo.
- **Scope:** apply to **pii only** first, or also **pidag**? pidag is a much
  smaller tree (one crate) where the payoff is likely lower.

---

## 5. Must graphify be called at the START of a session? — NO (measured)

**Short answer: no. Nothing needs to run at session begin.** The graph is
persistent on disk; the design is *build once, query many, refresh
deliberately.* Measured on pidag:

| what | evidence |
|---|---|
| The graph persists across sessions | `graphify-out/graph.json` (3.6 MB) sits on disk; nothing rebuilds it on open |
| The hook points at a **query**, not a rebuild | fires only when `graph.json` **exists**, and instructs *"run `graphify query …` before reading source files"* — it never asks for a rebuild |
| The skill has an explicit **fast path** | *"Before doing anything else, check whether `graphify-out/graph.json` exists … **skip Steps 1–5 entirely and jump straight to `## For /graphify query`**. Do not run detect. Do not check corpus size. The graph is already built — use it."* |
| A refresh is cheap **if** you want one | `graphify update .` → **2.9 s** as a no-op (*"No code-graph topology changes detected"*), **4.8 s** with real changes |

So the correct session-begin behaviour is **do nothing** — query on demand. The
nudge exists precisely so the agent reaches for the graph *instead of* a
grep-and-read sweep, not so it rebuilds anything.

### The right moment to refresh is AFTER code changes, not at session start

That is what the tool's own design assumes: it offers a `post-commit`/
`post-checkout` hook (which we declined) **and** an explicit `graphify update .`
for the manual case. The natural trigger is "I changed code", not "I opened a
session". `update` is idempotent and cheap, so running it at session start is
*harmless* — but it is not required, and if nothing changed it is 3 s of
nothing.

### CAVEAT I hit while measuring this — `update` WIDENS the corpus

`graphify extract . --code-only` on pidag produced **3,101 nodes**. A later
`graphify update .` produced **5,098** — because update re-scanned more than
code:

```
.rs 2142   .md 1997   (none) 835   .sh 122   .toml 2
top dirs: src 1191, specs 1156, tests 1126, deploy 275, .claude 140, .orchestration 133
```

So **`update` ≠ `extract --code-only`** — it is not corpus-narrow. It pulled in
`specs/` (1156 nodes) and agent-config dirs (`.claude/` 140,
`.orchestration/` 133). Two consequences worth a decision later:

- **Good:** connecting `specs/*.md` to code in one graph is arguably *valuable*
  for us — pidag's specs are its contract.
- **Noise:** `.claude/` and `.orchestration/` (our own agent config) are not
  code and should probably be excluded.
- **Cost:** the graph grew 3,101 → 5,098 nodes; more nodes means a larger
  `graph.json` and noisier traversal.

**No LLM spend occurred** — the AST path is deterministic, and `cost.json`
records no tokens. (A `GEMINI_API_KEY` is present in the environment; the
code/structural path did not use it.)

**Recommendation:** leave session start alone; refresh deliberately after code
changes; and if we adopt the broader corpus, add an exclude for
`.claude/`/`.orchestration/` so the graph stays "the codebase", not "everything
in the tree".

### For pidag specifically

**Moved to the pidag repo** — pidag-specific content belongs there:
`/projects/pidag/docs/graphify-integration.md`. Summary of what it covers: the
graph is already built and persists; nothing runs at session open; and
`graphify update .` writes only inside the gitignored `graphify-out/`, so it
cannot dirty the tracked tree or disturb that repo's workers-never-commit
effect-verification seam.

---

## 6. Could the commit-hook idea work? — YES, measured (and it is safe here)

**Answer: yes, and it does not threaten the effect-verification seam — but I
would still wait.** Tested in a throwaway repo, twice.

**Mechanics, measured:**

| | |
|---|---|
| Hook latency | **0.093 s** — `post-commit` *launches a detached background rebuild* and returns immediately (`[graphify hook] launching background rebuild (log: /root/.cache/graphify-rebuild.log)`) |
| Actually rebuilds? | **Yes.** ~12 s later the graph contained the new symbol (`gamma present: True`, 3 → 4 nodes) |
| Rapid consecutive commits | **Handled.** Two commits 1 s apart → both rebuilds ran, graph ended with **both** new symbols (5 nodes) |
| Writes go where? | **Only inside `graphify-out/`** |

**The seam question — answered by measurement, not reasoning.** pidag's rule is
*workers never commit; the orchestrator commits*, and the effect-verification
seam depends on tree state. Concern: does a background rebuild mid-run dirty
the tracked tree?

**No.** With `graphify-out/` gitignored (as we have it), the tracked tree was
**clean immediately after the commit and still clean after the background
rebuild finished** — verified twice, including the two-rapid-commits case. The
hook writes only into the ignored directory.

**The one real caveat:** `graphify hook install` also writes a tracked
`.gitattributes` (`graphify-out/graph.json merge=graphify`) and registers a git
merge driver (`merge.graphify.driver`). In my first test, before I gitignored
the output, `.gitattributes` **and 15 MB of `graphify-out/` got swept into the
next commit**. With the output ignored, `.gitattributes` is the only new
tracked file — harmless (a union-merge rule for a file we ignore), but it *is*
a tracked-file change, so it must be committed deliberately rather than
appearing in a worker's diff.

**Why I would still not install it yet** — three reasons, all about *when*, not
*whether*:

1. **The orchestrator commits mid-run.** pidag's flow is orchestrator-commits
   after each spec. A rebuild firing on every one of those during a long
   session is churn we do not need while a worker is editing the same tree.
   (Cheap, but not free, and it competes with the worker for CPU/disk.)
2. **It is asynchronous, so "the graph is current" is eventually-true, not
   immediately-true.** A query issued 1 s after a commit may read the previous
   graph. For a gate that must be a *hard* oracle, that is a bad property —
   fine for orientation, wrong for anything we might later gate on.
3. **Nothing is broken without it.** `graphify update .` is 2.9 s as a no-op
   and idempotent; running it deliberately, right before we want to query, is
   strictly more predictable than a background race.

**Recommendation:** keep it off for now. If we later want it, the prerequisite
is committing `.gitattributes` deliberately and confirming no worker diff can
sweep it. Revisit when the graph becomes load-bearing (i.e. if Decision 5's
`reachability-check` upgrade makes a query part of a gate — at which point
asynchronous freshness becomes a real design problem to solve, not a
convenience to add).

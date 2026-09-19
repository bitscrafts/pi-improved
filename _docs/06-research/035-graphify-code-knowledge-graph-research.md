# Graphify — research: (A) tooling for our pii work, (B) a native pii feature (2026-09-19)

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

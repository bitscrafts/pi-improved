# 024 — Pii memory + MCP: verified status, caveats, and what's next

**Date**: 2026-09-11
**Scope**: Is Pii's memory subsystem fully working? Is MCP working, and can
Hermes connect to it? What are the honest gaps, and what is the next
implementation work?
**Verdict**: Both work end-to-end today. Neither is exposed where it would
matter most (the agent loop / first-class MCP tools). Caveats below are
capability boundaries, not bugs.

---

## 1. Verified working (live evidence, 2026-09-11)

### 1.1 MCP: Hermes ↔ Pii connected

- `hermes mcp list` → `pii → /root/.local/bin/pii-mcp`, status `✓ enabled`
  (alongside `chromecast`).
- Binary present: `/root/.local/bin/pii-mcp` (1.8 MB, built 2026-09-09).
- `pii-mcp` is *server + client* (spec 21): it exposes pii's native tools
  (fs/edit/bash/grep/glob) as an MCP server over stdio, and can consume
  external MCP servers as a client.
- Tools exercised live through Hermes' MCP client:

  | Tool | Call | Result |
  |---|---|---|
  | `glob` | `cwd=pii-mcp/src`, `pattern=*.rs` | `main.rs` |
  | `fs_read` | `path=.../pii-mcp/src/main.rs` | real file content, 258 lines |
  | `bash` | `echo MCP_PII_OK && pwd` | `MCP_PII_OK`, exit 0, cwd `/projects` |
  | `grep` | `cwd=pii-mcp/src`, `pattern="MCP server"` | 8 `path:line:content` rows |

- Remaining tools available but not exercised here: `edit`, `fs_write`,
  `multi_edit`.

### 1.2 Memory: full round-trip works

Driven through MCP (`mcp__pii__bash` → pii CLI → daemon UDS):

1. `pii memory put 'hermes can drive pii memory over mcp bash'`
   → `{"id":"6cbbe7bc638eeeaae8104cc8f7367042"}`
2. `pii memory search 'hermes mcp memory' --k 3`
   → `{"degraded":false,"hits":[{"doc":"6cbbe7bc…","lanes":["bm25","vector"],"score":0.0328…}],"lanes":["bm25","vector","graph"]}`
3. `pii memory consolidate`
   → `{"archived":0,"candidates_capped":false,"candidates_evaluated":0,"merged":0}`
   (clean run, nothing to merge), and the doc **persisted** afterwards.

- Daemon: `pii daemon --foreground --sandbox off --workspace /projects/pi-improved --store /root/.pii/store.redb`
- Socket: `/root/.pii/runtime/control.sock`
- All three RPCs are real handlers (`memory_search`, `memory_put`,
  `memory_consolidate` in `pii-server/src/daemon/methods.rs`), not stubs.
- `pii-memory` tests: **15 passing** (4 unit + 3 consolidate + 7 lanes + 1
  persistence) covering BM25 reference scores, quantisation roundtrip <1%,
  RRF fusion, honest degradation without an embedder, decay ordering,
  `write_fact_stores_vector`, and vector lane surviving a restart.

---

## 2. Caveats (capability boundaries — verified, not speculation)

### C1 — Memory is NOT a first-class MCP tool
`pii-mcp` exposes only `fs_read`, `fs_write`, `edit`, `multi_edit`, `grep`,
`glob`, `bash` (asserted by its own `server_lists_builtin_tools` test). There
is no `memory.search` / `memory.put` MCP tool. Memory is reachable over MCP
only *indirectly*, by driving the CLI through the `bash` tool. Any MCP client
that expects memory as a tool will not find it.

### C2 — Memory is NOT exposed to Pii's own agent loop
The agent uses memory internally (`pii_memory::doc::write_fact` in
`pii-server/src/agent.rs`), but there is no `memory_search` / `memory_store`
tool in the model's `ToolSet`. The model cannot decide to recall or record
anything mid-task; recall is not part of the turn loop.

### C3 — The vector lane is lexical, not semantic
The active embedder is `TrigramEmbedder` — deterministic, offline, 256-dim.
There is no neural embedder (B1 correctly deleted the dead `OllamaEmbedder`,
which had zero non-test call sites). So "vector" recall is trigram similarity,
not meaning. Paraphrase recall ("car" ↔ "automobile") will miss.

### C4 — Ranked hits carry doc ids, and the CLI does not print the text
`memory.search` returns `{doc, lanes, score}`. Text retrieval requires a
follow-up; the hit struct carries the text internally (`memory_hit_carries_doc_text`)
but the CLI output is ids+scores only. Fine for machines, poor for a human or
a model that wants the recalled content inline.

### C5 — Graph lane is present but unexercised here
The `lanes` list reports `graph`, and the lane exists in
`pii-memory/src/graph_lane.rs`; this round-trip matched on `bm25` + `vector`
only. Graph-lane recall behaviour has not been positively demonstrated
end-to-end.

### C6 — MCP tool ergonomics have sharp edges (found live)
- `glob` with an absolute `pattern` and no `cwd` returns **empty**; it needs
  `cwd` + a relative pattern.
- `grep`'s `path` parameter is a **filename filter**, not a directory. Passing
  `path="."` silently yields empty matches; omit `path` and set `cwd`.
- `bash` runs with cwd `/projects` (the server's launch root), not the
  workspace a caller might assume.

### C7 — MCP server is feature-gated (default off)
`pii-mcp` is feature-gated with `rmcp` quarantined (spec 21 / R6), and the
server has a bounded lifetime. Distribute/installed binaries must be built with
the feature on; a default `cargo build` does not produce it.

### C8 — No automated end-to-end coverage
Memory RPCs are covered at the unit/integration level inside `pii-memory`;
there is **no** test that drives `memory.put` → `memory.search` through the
daemon socket, and no test that drives the MCP server's tool list end-to-end
through a real MCP client. The evidence above is manual.

---

## 3. What's next (ordered; smallest useful step first)

**N1 — Native memory MCP tools (closes C1).**
Add `memory_search` / `memory_put` to the `pii-mcp` server's `ToolSet`, mapping
to `Method::MemorySearch` / `Method::MemoryPut`. Small: the RPC methods already
exist and are tested. Delivers C1 and makes memory usable by any MCP client
(Hermes included) with no CLI detour.

**N2 — Memory tools in the agent loop (closes C2).**
Add `memory_search` / `memory_store` to Pii's builtin `ToolSet` so the model
can recall/record during a turn, gated by capability (`memory:read` /
`memory:write`) and bounded like every other tool. This is what turns memory
from a store into a cognitive layer. Larger: touches `pii-server::agent.rs`,
the tool registry, and the system prompt that advertises tools.

**N3 — Return recalled text inline (closes C4).**
Include the doc text (or a truncated head) in the CLI/JSON hit payload so a
caller does not need a second round-trip.

**N4 — Prove or drop the graph lane (closes C5).**
Either write an end-to-end test that exercises graph-lane recall and document
when it fires, or state plainly in the docs that it is dormant.

**N5 — End-to-end memory + MCP tests (closes C8).**
A daemon-socket round-trip test (put → search → consolidate) and an MCP-client
test that lists and calls a tool. These move the manual evidence above into CI.

**N6 — Revisit semantic recall (closes C3), deliberately later.**
If paraphrase recall matters, evaluate a small real embedding model (e.g. a
tiny GGUF embedder through the existing in-process `llama-cpp-2` path — spec 35
proved that route works) rather than reintroducing a remote embedder. Keep the
trigram embedder as the always-available fallback and keep `degraded` honest.

Dependency shape: **N1 and N5 are independent and small** — do them first.
**N2 depends on N1's surface staying stable.** N3/N4 are polish. N6 is a
research decision, not a task, until recall quality is actually felt as a
problem.

---

## 4. Bottom line

- MCP: **working**, Hermes connected, tools responding.
- Memory: **working** end-to-end (put/search/consolidate, 3 real RPCs, 15 tests).
- The gap is **exposure, not function**: memory is a working store that neither
  MCP nor the agent loop can reach natively, and its vector lane is lexical.
  C1 + C2 are the two changes that matter; everything else is polish.

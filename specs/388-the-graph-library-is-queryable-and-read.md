# Spec: pii keeps a catalogue of graphs, derived from the files, and the agents read it

## Overview

The operator's requirement: *"we should keep a graph database to help users with common flows"*.

The need is concrete. Today `graphs/` holds 32 files and the only way to learn what exists is to read
them — which is how two files that never loaded survived for months, and how four graphs ended up
doing one job. A user who wants "write the books" or "drain my queue" has no way to ask *does this
already exist, and what do I compose with it*.

The catalogue must be **derived**, never maintained. A hand-written index is a book that lies the first
time a graph changes; `docs/03-reference/graph-vocabulary.md` and `graphs/DOCS/` both exist because
reading the files was the only honest source.

**Project**: /projects/pi-improved
**Phase**: graph vocabulary / discoverability

## Requirements

- **R1 — `pii graph index` derives the catalogue from the files.** It walks the system graph directory
  and the user graph directory, parses every `[[node]]` table, and writes one catalogue entry per
  graph: purpose, entry, exit, the kinds it uses, the graphs it `call`s, its capabilities, and the
  revision it was built from. Idempotent: running it twice changes nothing.
- **R2 — the catalogue lives in the store pii already has.** Each graph is a memory document
  (`kind = skill`) whose concepts are derived from its kinds, ports and call targets; each `call` is a
  relationship. No new storage engine, no second index — the existing RRF lanes (bm25, graph, vector)
  are the query engine. A graph nobody can find is a graph nobody uses: the point of routing this
  through memory is that **the existing lanes become production readers of the graph library**, which
  is the failure the memory store has suffered from before.
- **R3 — `pii graph find <question>` answers in incident form.** For each candidate: its name, one line
  of purpose, the kinds it uses, and **its outgoing `call`s colocated with it** — not a flat list of
  edges. (Measured elsewhere in this workspace: incident encoding moved structural-reasoning accuracy
  from 19.8% to 53.8% versus flat edge lists on the same graph, arXiv:2310.04560. The reader here is a
  model choosing what to compose, so the encoding is load-bearing, not cosmetic.)
- **R4 — the agent can ask.** `graph_find` joins the tool surface an `agent` node may use, next to
  `memory_search` and `memory_put`. A coder asked to write a graph can therefore ask what exists
  before writing, with no shell and no new capability: it reads the catalogue, which holds nothing the
  graph files do not already publish.
- **R5 — consumption is certified, not assumed.** A coder run that writes or edits a graph records
  whether `graph_find` was called and what it returned. A new graph whose call shape duplicates a
  shipped graph is reported by the gate as `duplicate-flow`, naming both. Injection without
  consumption is what made the concept graph worthless once before; the counter here is
  `graphs_written_without_a_lookup` and `duplicate_flows_found`.
- **R6 — stale is visible.** The catalogue records the revision it was built from. If a graph file has
  changed since, `graph find` says so in one line and still answers — a catalogue that silently
  describes the past is worse than none.

## TDD contract

| Test | Input | Expected |
|---|---|---|
| `index_covers_every_loadable_graph` | the shipped directory | one entry per graph, and the two broken files are reported, not skipped silently |
| `index_is_idempotent` | run twice | the store's counts are unchanged after the second run |
| `find_names_the_graph_that_does_the_flow` | "write the books" | `books` in the top three, printed with its calls |
| `find_reports_calls_colocated` | any hit with calls | the calls appear on the hit's own lines, not in a separate section |
| `a_stale_catalogue_says_so` | index, then touch a graph | the answer carries a stale line naming the file |
| `control_a_question_with_no_flow_answers_none` | "translate this to Japanese" | no candidate, and no invented one |
| `the_agent_can_call_graph_find` | an agent node with the tool listed | the call is dispatched and journalled |

## Exit Criteria

- [ ] `./target/debug/pii graph index --dir graphs 2>&1 | grep -qE 'indexed [1-9][0-9]* graphs'`
- [ ] `./target/debug/pii graph find "write the books" 2>&1 | grep -q books`
- [ ] `cargo test -q -p pii-cli --test graph_catalogue 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `grep -q 'graph_find' crates/pii-tools/src/lib.rs`
- [ ] `grep -q 'graphs_written_without_a_lookup' crates/pii-cli/src/cmd_graph*.rs`

## Guardrails

- Do NOT hand-maintain the catalogue. If a fact is not derivable from a graph file, it does not belong
  in the catalogue — it belongs in the graph.
- Do NOT add a second storage engine. One store, one search path; the catalogue is documents and edges
  in the store that already exists.
- Do NOT let `graph find` answer without saying which revision it describes when it is behind.
- Do NOT count an index run as consumption. The metric is a coder that asked and then composed.
- Do NOT make the catalogue a capability: reading it grants nothing a file read would not.

## Files
- `crates/pii-cli/src/cmd_graph_index.rs`
- `crates/pii-cli/src/cmd_graph_installed.rs`
- `crates/pii-cli/src/main.rs`
- `crates/pii-cli/tests/graph_catalogue.rs`
- `crates/pii-cli/tests/graph_catalogue/agent.rs`
- `crates/pii-cli/tests/graph_catalogue/fixture.rs`
- `crates/pii-mcp/src/main.rs`
- `crates/pii-memory/src/catalogue.rs`
- `crates/pii-memory/src/lib.rs`
- `crates/pii-proto/src/method.rs`
- `crates/pii-proto/tests/wire_roundtrip.rs`
- `crates/pii-server/src/engine/conn.rs`
- `crates/pii-server/src/engine/dispatch.rs`
- `crates/pii-server/src/engine/graph_methods.rs`
- `crates/pii-server/src/engine/methods.rs`
- `crates/pii-server/src/engine/mod.rs`
- `crates/pii-server/src/lib.rs`
- `crates/pii-server/src/node_kinds/agent/tools.rs`
- `crates/pii-tools/src/lib.rs`
- `crates/pii-tools/src/memory.rs`
- `docs/03-reference/graph-vocabulary.md`

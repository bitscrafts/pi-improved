# Memory entities and relationships (spec 126)

The memory pillar's **structure**: typed entities and typed relationships with provenance, a
bounded query surface, and a `remember` / `recall` node pair the engine can use. Until this
spec `pii-memory` held notes and documents; there was no way to say *"this run was about the
ESP32 board, and that board is in the office"*. Tiering (spec 127) is **not** built: nothing here
ages, demotes or archives a record, and nothing deletes one.

No model is involved. Records are written by code from run data, and everything in a name or an
attribute is **data**, never an instruction.

## The types

| | fields |
|---|---|
| `Entity` | `id`, `kind`, `name`, `attributes`, `provenance` |
| `Relationship` | `id`, `from`, `to`, `kind`, `attributes`, `provenance` |

* **Ids are content-derived.** `entity id = "e-" + 32 hex of blake3("pii.entity.v1" 0x00 kind 0x00 name)`;
  `relationship id = "r-" + 32 hex of blake3("pii.relationship.v1" 0x00 from 0x00 to 0x00 kind)`.
  The attributes and the provenance are **not** hashed, so describing the same thing twice
  yields the same id and the same bytes on disk.
* `kind` is `a-z 0-9 _ - .`, 1..=64 bytes. `name` is non-empty text, at most 256 bytes. An
  attribute key is at most 64 bytes and a value at most 1024.
* **Provenance is mandatory**, and is one of `run:<id>`, `file:<path>` or `actor:<name>` (non-empty,
  at most 256 bytes). It is stored sorted and deduped.

## Writes

* **Upsert is idempotent and merges.** The same entity described twice is one entity whose
  provenance lists both sources and whose attributes are the union. For a clashing attribute key the
  *greater* value wins, so the result does not depend on write order. The write reports
  `created`, `merged` or `unchanged`.
* **Refusals are named, counted and reported** (`memory.write_refused`), and write nothing:
  `no_provenance`, `bad_provenance`, `invalid`, `too_large`, `entity_cap`, `relationship_cap`,
  `provenance_cap`, `unknown_entity`.
* **Referential integrity: refuse.** A relationship whose `from` or `to` names no entity is refused
  as `unknown_entity`, naming which end. No stub entity is ever created, so no dangling link exists. Nothing
  deletes an entity, so none can appear later either.

## Bounds (the lesson of spec 115)

| key (`[memory_graph]`) | default | when reached |
|---|---|---|
| `max_entities` | 10000 | a **new** entity is refused (`entity_cap`); merging into an existing one still works |
| `max_relationships` | 50000 | a **new** relationship is refused (`relationship_cap`) |
| `max_provenance` | 32 | a write that would put more sources on a record is refused (`provenance_cap`) and the record is left as it was |
| `max_attributes` | 32 | refused as `too_large` |
| `max_depth` | 3 | a deeper `neighbours` walk is refused (`depth_cap`) |
| `default_depth` | 1 | used when a call names no depth |
| `max_results` | 200 | a query with more matches is refused (`result_cap`) |

The policy is one rule everywhere: **refuse the write with a counted reason; never evict.** A
cap-reached store keeps what it has. Order matters only at a cap (which write got in first); below
the caps the same writes in any order leave byte-identical state (`graph.json`).

## Queries

By id (`get`), by kind (`by_kind`), by exact name (`by_name`; substring search is deliberately not
offered), and `neighbours` of an entity, following links in either direction up to a depth. Results
are sorted (by id; neighbours by depth then id). **Every query is capped and the cap is a refusal,
never a silent truncation:** a query that matches more than the limit is `result_cap`, a depth
above the cap is `depth_cap`, depth 0 is `bad_depth`, an unknown start is `unknown_entity`, an empty
name or kind is `bad_query` (all counted, and reported as `memory.query_refused`).

Cost: the store is loaded whole, which is bounded by `max_entities`/`max_relationships`, so
`by_kind`/`by_name` are one pass over at most `max_entities` records and `neighbours` builds an
adjacency over at most `max_relationships` links. Each write rewrites `graph.json`; that is
the price of one canonical, comparable file.

## Where it lives

`<runtime dir>/memory/graph.json` (the records, compact JSON, sorted maps) and `counters.json`
(write and refusal counters, kept apart because how many writes *changed* something depends on
order). `graph.lock` serialises writers across processes, so the daemon and the CLI can both
write. This is **not** a redb table: `pii-memory` may reach redb only through `pii-store`.

## The CLI (offline; `--dir` defaults to the runtime dir)

```sh
pii memory entity add --kind device --name esp32-board --source actor:operator --attr location=office
pii memory entity show <id>
pii memory entity list --kind device        # or --name <exact>   [--limit N]
pii memory link <from-id> <to-id> --kind located_in --source run:r7
pii memory neighbours <id> [--depth N] [--limit N]
pii memory compact                          # refuses while a daemon holds the store
```

A refusal exits 2 and names the reason. `compact` rewrites `graph.json` in its canonical form and
removes the leftovers of an interrupted write; it never drops a record, and it reports measured
before/after bytes (a store written by this code is already canonical, so the pair is usually equal;
the case it reclaims is a hand-edited or pretty-printed file). It refuses, as `pii store compact`
does, while a daemon holds the redb files.

## The nodes (the point of the pillar)

| node | needs | `op` |
|---|---|---|
| `remember` | `memory_write` | `entity` (`entity_kind`, `name`, `attributes`?, `provenance`?), `relationship` (`from`, `to`, `rel_kind`, `attributes`?, `provenance`?) |
| `recall` | `memory_read` | `get` / `neighbours` (`entity_id`, `depth`?, `limit`?), `by_kind` (`entity_kind`), `by_name` (`name`) |

`from`/`to` are an entity id or `{ kind, name }`; every argument may be a `$.` expression. The
writing run's id is **always** added to the provenance as `run:<id>`. A refused call leaves on the `err` port as
`{error, reason}`.

**Capability.** The graph must list the capability (`caps = ["memory_write"]`) *and* the node must hold it
(a node's caps default to the kind's, and `caps = []` removes them). A graph that omits it fails
validation (`V2`); a node without it fails the run with `capability denied: memory: write not granted ...`
(or `read`). A run's root capability set now carries `memory_read` and `memory_write`, so that
the graph's `caps` are what narrow them, like every other capability.

Examples: `examples/memory-remember.toml` writes a device, a room and the link between them;
`examples/memory-recall.toml` reads them back in a later run.

## Observability

Events: `memory.entity_written`, `memory.relationship_written`, `memory.write_refused`,
`memory.query_refused` (all in `names::ALL`). Metrics (`pii metrics`, from a live daemon):
`pii_memory_entities`, `pii_memory_relationships`, `pii_memory_writes_total{kind}`,
`pii_memory_write_refusals_total{reason}`, `pii_memory_query_refusals_total{reason}` - printed once
the store has been used. `pii health --json` carries a `memory_entities` subsystem with the counts
and the caps.

## The injection stance

Names and attributes come from run data. They are stored verbatim, are never interpreted, and never
influence the id, the kind or the outcome of a write. *"ignore previous instructions"* in an
attribute is an attribute; in a name it is a name; as a provenance source it is refused by the
ordinary `bad_provenance` rule. The same control `pii-learn` carries. A consumer that puts a recalled
value into a model prompt must treat it as data too: that boundary is the consumer's, not the store's.

## Not verified

No soak of a store at its caps; no concurrent-writer stress beyond the lock being exercised by the
daemon-plus-CLI case; no fuzzing of the JSON reader (it is size-bounded, and a corrupt file is
reported, never overwritten).

# Memory entities, relationships and tiers (specs 126, 127)

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

## Tiers (spec 127)

Time is part of memory. Every record (entity or relationship) carries an access record in
`<dir>/memory/access.json`: `last_access_secs` and `access_count`. It is written at creation and by
**every read** through the query surface (`get`, `by_kind`, `by_name`, `neighbours`, `entity show|list`,
`neighbours`, the `recall` node, all via the same store methods): one transaction and one small file write per
query, not per attribute, and `graph.json` is never touched by a read. A refused query reads nothing.

**A tier is derived, not stored.** `tier = f(last_access_secs, access_count, now, thresholds)`:

| tier | when |
|---|---|
| `hot` | last read within `hot_secs`, **or** read at least `hot_count` times and within `warm_secs` |
| `warm` | not hot, last read within `warm_secs` |
| `cold` | last read longer ago than `warm_secs` (a record with no access entry counts as never read) |

Defaults and why: `hot_secs` = 86400 (a day: what a working session just touched), `warm_secs` = 604800 (a week:
what is still part of the current working set), `hot_count` = 10 (something read ten times is a habit, not an
accident, and stays hot for the warm window; it cannot hold a record hot past `warm_secs`). Keys:
`[memory_graph] hot_secs / warm_secs / hot_count` in `pii.toml`, or `--hot-secs --warm-secs --hot-count` on
the CLI. Because the tier is arithmetic over two stored numbers, it cannot disagree with them; the same
numbers under different thresholds give a different tier (tested).

**Ranking.** `entity list --rank` (and `rank: true` on a `recall` `by_kind`/`by_name`) orders by tier (hot,
warm, cold), then last read (latest first), then read count (most first), then id: a total order, so the same
state returns the same list. `pii memory show <id>` prints the record with `tier` = `{tier, last_access_secs,
age_secs, access_count, why}`; `why` names the two numbers and the threshold that decided it. `show` explains the tier the
record had just before this read, and the read then counts.

**Demotion is not deletion.** A record that goes cold is *demoted*: counted (`hot_to_cold` and so on), reported
(`memory.tier_changed`), ranked last, still returned by every query. One read makes it hot again (`cold_to_hot`).
Nothing deletes a fact without an audited command: there is no timer that removes anything.

**The budget policy is refuse.** `max_bytes` (default 16 MiB, `--max-bytes`, `[memory_graph] max_bytes`) bounds
the graph file, beside the entity and relationship caps. A write that would pass it is refused as `bytes_cap`,
counted, and nothing is stored. It is **not** "demote the coldest": no record changes tier, moves or leaves to
make room (tested: at the budget the tier counters have not moved and nothing was evicted).

## Archive and restore (the only removal path)

`pii memory archive --before <epoch-secs | Ns|Nm|Nh|Nd>` selects records last read before the cut (a relationship
is taken with an entity it touches, so no link dangles), writes `<dir>/memory/archive-<ts>.jsonl`, **re-reads
it and checks the bytes and the line count**, and only then removes them from the graph. It prints
`archived records=N bytes=M entities=.. relationships=.. file=..`. If the dump cannot be written and
verified (the name is taken, the disk refuses), nothing is removed and the refusal is `archive_failed`, counted,
and `memory.archive_refused`. Nothing old enough prints `records=0` and writes no file.

The dump is plain JSONL, sorted by id (entities `e-...` then relationships `r-...`), one record per line:
`{"type":"entity|relationship","record":{...with provenance...},"access":{"last_access_secs":N,"access_count":N}}`.
It holds no timestamp inside, so two archives of the same state are byte-identical (tested with `cmp` semantics).
`pii memory restore <file>` replays it through the ordinary validation, caps and byte budget (a line whose id
does not match its content, or that breaks a rule, is refused and listed; the rest are restored), prints
`restored=N unchanged=N refused=N`, and gives restored records a fresh read (hot) with their dumped read counts.

## Tier observability

Events: `memory.tier_changed` (`id`, `from`, `to`), `memory.archived`, `memory.archive_refused`,
`memory.restored` (all in `names::ALL`). Metrics from a live daemon: `pii_memory_tier{tier}` (gauge),
`pii_memory_tier_changes_total{change}`, `pii_memory_archive_ops_total{op}` (`archived`, `restored`, `refused`).
`pii health --json` `memory_entities` carries `tier_hot|warm|cold`, `bytes`, `max_bytes`. The daemon's status
read sweeps the tiers, so a record that only aged still moves the counters. `pii memory tiers` prints the same.
Hostile text in an attribute is data here too: it cannot change a tier, a rank or the shape of a dump line.

## Not verified (spec 127)

No soak at the byte budget; no clock-skew or clock-goes-backwards testing (ages saturate at zero); the tier
counters are sweep-based, so a transition is observed at the next read or status poll, not at the instant of
the threshold; no concurrent archive-while-writing stress beyond the store lock; archive of a store larger
than the 64 MiB read bound is not exercised. Pillar 3 still has no model-driven inference and no cross-instance sync.

## Not verified (spec 126)

No soak of a store at its caps; no concurrent-writer stress beyond the lock being exercised by the
daemon-plus-CLI case; no fuzzing of the JSON reader (it is size-bounded, and a corrupt file is
reported, never overwritten).

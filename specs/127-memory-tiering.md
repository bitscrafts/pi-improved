# Spec: 127 — memory tiering: what is hot, what is dormant, and what is never silently deleted

**Project**: <repo>
**Phase**: **Stage D** of `057`, the last stage — vision pillar 3 completed. Spec 126 gives memory its shape
(entities, relationships, provenance, bounded queries); this spec gives it **time**: what is in active use, what
has gone quiet, and what to do about it without ever destroying a fact by accident.

## What exists (measured 2026-10-10)
- `pii-learn` (123/124/125) built the discipline this reuses: typed records, provenance, caps that refuse rather
  than truncate, determinism, data-not-instructions, counters that must **render** through the daemon.
- Spec 126 (queued) adds `Entity`/`Relationship` with content-derived ids, mandatory provenance, merged provenance
  on upsert, a bounded query surface, caps with a documented policy, and the `remember`/`recall` node pair.
- Nothing anywhere in the tree ages, ranks, demotes or archives anything.

## Requirements
R1. **Access is recorded**: every read of an entity/relationship through the query surface or the `recall` node
  updates an access record (`last_access_secs`, `access_count`) — bounded (state the cap) and cheap (no write per
  attribute). State where the write happens and prove it happens on both paths (CLI query and node).
R2. **Tiers are a function of the record, not a field someone sets by hand**: define hot / warm / cold by
  **stated thresholds** over `last_access_secs` and `access_count` (pick the numbers, state why, make them
  configurable). A record's tier is derivable at any time — so a tier can never disagree with the access data.
  Prove it with a test that walks a record through all three tiers by advancing time.
R3. **Ranking is deterministic and explainable**: a query may rank by tier, then recency, then count, then id —
  with the tie-breaks stated and total (no unstable ordering). The same query on the same state returns the same
  order (paste a `cmp`); and `pii memory show <id>` states **why** a record is in its tier (the two numbers).
R4. **Demotion, never silent deletion**: a record that has not been accessed inside the cold window is
  **demoted** (never removed) and counted; demotion is reversible by a single access. Deleting a fact requires an
  explicit, audited command (`pii memory archive --before <time>` writing a JSONL dump **first**) and must be
  refused if the dump cannot be written and verified (size + line count reported). Automatic deletion is a fail.
R5. **A total budget with an honest behaviour**: a configured cap on total bytes (and records). When the cap is
  reached, the policy must be one of {refuse the write with a counted reason, demote the coldest first} — pick one,
  state it, and prove the other is not happening accidentally. Silent loss is a fail; so is an unbounded store.
R6. **The archive is a first-class artefact**: `pii memory archive` writes `<area>/archive-<ts>.jsonl` (one record
  per line, provenance intact), reports `records=N bytes=M`, and only then removes those records; `pii memory
  restore <file>` reads it back and re-establishes the entities/relationships with their provenance, reporting what
  it restored and what it refused. Round-trip is proven: archive → restore → the query surface returns the same
  records (paste).
R7. **Observability**: events for tier transitions and archive operations (`memory.tier_changed`,
  `memory.archived`, `memory.archive_refused`, `memory.restored`), registered in `ALL`; counters by tier and by
  operation in `pii metrics`, **rendered through a live daemon** (the 123 lesson); `pii health` reporting the tier
  counts. A tier counter that only ever reads one value is a finding — prove at least one transition in the E2E.
R8. **Determinism and caps**: no unbounded scan anywhere; every query path keeps 126's caps; the archive is
  sorted (by id) so two archives of the same state are byte-identical (paste `cmp`).
R9. **Docs**: extend the memory guide with the tiers, the thresholds and why, the ranking and its tie-breaks, the
  demote-vs-archive distinction, the budget policy, and the explicit statement that **nothing deletes a fact
  without an audited command**. `001`'s pillar-3 line becomes complete: entities/relationships + tiers, with what is
  still absent stated (no model-driven inference, no cross-instance sync).
R10. **Injection stance inherited**: entity text stays data — a hostile string in an attribute must not affect
  tiering, ranking or the archive.

## Exit Criteria
- [ ] the tests exit 0 with: the tier walk by advancing time; the access record written by both the CLI query and
      the `recall` node; deterministic ranking (`cmp`); the demotion (and its reversal by one access); the archive
      round-trip; the budget policy with its refusal or demotion proven; the hostile-attribute control
- [ ] a mutation per rule — access not recorded; thresholds hard-coded as a field; ranking made unstable; archive
      skip-if-dump-fails inverted (delete first!); budget bypassed; a tier made underivable from the record
- [ ] the E2E paste: a record walks hot → warm → cold (or the equivalent) with the counters moving through a **live
      daemon**, then an archive + restore with the counts
- [ ] `deps-check`, `net-check`, `doc-drift-check`, `lint-observability` exit 0
- [ ] `bash quality-gate.sh` exits 0; plain `fresh-gate` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the guide is updated; `001`'s pillar-3 line updated and **marked complete with its absences listed** (paste)
- [ ] the report lists files, names, thresholds, config keys, and what was NOT verified

## Guardrails
- **Nothing in this spec may delete a fact automatically.** The only removal path is the audited archive, and it
  writes and verifies the dump first. If you find yourself adding a `remove` on a timer, stop and re-read R4.
- Reuse 126's caps and 123's determinism/provenance discipline; do not build a parallel mechanism.
- No model in the loop: tiering is arithmetic over recorded access, not inference.
- Do not change the archive's on-disk shape without saying so; the dump is an artefact a human may need to read.
- Scratch under `<scratch>/`, delete only your own, no daemon left running. Do NOT commit.

## Orchestrator verification (2026-10-10)
**My controls:**
- `cargo test -p pii-memory --test tiering` → **13 passed**; `cargo test -p pii-cli --test memory_tiering` →
  **2 passed**; `cargo test -p pii-server --test tiering_e2e` → **1 passed** (that one drives a **live daemon**
  with a real clock: hot → warm → hot via a `recall` run → cold, archive 3 records/729 bytes → 0 entities →
  restore 3 → hot, with the tier counters moving at every step).
- `deps-check`, `net-check`, `doc-drift-check`, `lint-observability` → **rc 0 each**.
- **Mutation (mine)**: making `record_access` a no-op at its three sites turns **seven** tests red —
  `access_is_recorded_by_get_by_kind_by_name_and_neighbours_in_one_write_each`,
  `a_record_walks_hot_warm_cold_by_advancing_time_and_one_read_restores_it`,
  `frequently_read_records_stay_hot_through_the_warm_window_but_not_past_it`,
  `explain_states_the_two_numbers_and_the_rule`, `ranking_is_total_and_stable_tier_then_recency_then_count_then_id`,
  `archive_writes_a_verified_sorted_dump_then_removes_and_restore_round_trips`, and — the one that matters most
  for R4 — **`nothing_is_ever_removed_by_time_alone`**; restored byte-identical → 13 pass.

**Design read at the source:** tiers are **derived** (`hot_secs` 86400 / `warm_secs` 604800 / `hot_count` 10, keys
`[memory_graph] …`), never stored; the budget policy is **refuse** (`max_bytes` 16 MiB, failure `bytes_cap`, and a
test asserts the tier counters did not move, so nothing is quietly demoted or dropped to make room); the archive
writes `archive-<ts>.jsonl` **sorted by id with no timestamp inside**, re-reads it and checks **bytes and line
count** before removing anything, and refuses with `archive_failed` otherwise; `restore` replays the dump through
the normal validation, caps and budget.

**Kept honest (the worker's own list):** the code was written before its tests (**no RED observed first**; the six
mutations are the failure proof); the **id tie-break cannot be shown to matter** (input comes from an id-sorted
map, so dropping it from the sort key would not fail a test — an untested tie-break is an untested claim); no soak
at the byte budget; no clock-goes-backwards test; no concurrent archive-while-writing stress; archiving near the
64 MiB read bound unexercised; and the tier-transition counters are **sweep-based**, so a change is counted at the
next read or status poll rather than at the instant the threshold passes.
### CLOSED (2026-10-10, plain fresh-gate on the committed HEAD)
`fresh-gate` (plain, on 38c5921, 55G free): `quality-gate: ALL PASS` · `fresh-gate: PASS` · 14 steps — the
criterion the worker could not run (it needs a commit, and workers do not commit).

With 127 closed, **specs 111-127 of `057` are complete**: every pillar of this vision is implemented in the
order the user set, each with its refusals proven by mutation, its limits written down, and the things this
container cannot test named rather than glossed. `001` carries the status line per pillar.

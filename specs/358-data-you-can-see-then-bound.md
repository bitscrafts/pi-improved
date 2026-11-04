# Spec: Data you can see, then bound

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening · **Source**: Opus optimisation review 2026-11-03,
workstream **W6** — closes findings 7, 12 and 19 (the retention half waits for four weekly samples).

## Verified premises

- `pii-store/src/mem/write.rs:71-82` bumps df even when the document already existed, so BM25 IDF is skewed for
  re-indexed documents; `:128-141` iterates the whole `MEM_TERM` table inside a write transaction with a `seen: Vec`,
  which is O(n²) per document.
- `RUN_SNAPSHOT` is written by `pii-graph/src/journal.rs:341` every 32 events and read only by
  `pii-store/tests/store_contract.rs:132`; `RESUME_INDEX` is written by `sched/dispatch.rs:423` and read by nothing.
- `pii-store/src/compact.rs:10-12` states there is no retention policy; compaction is offline only
  (`pii store compact`).
- Measured on lnx: `store.redb` and `memory.redb` are byte-identical to the development machine's, so production
  growth is ~0 today — the policy needs data, not speculation.

## Requirements

**R1.** `pii store stats` prints, per store and per table, the row count and the byte size, in one line each, and
exits 0. It reads through the daemon (single writer) or opens read-only.

**R2.** `bump_df` runs only when the document is genuinely new; a re-put of the same document leaves df unchanged.

**R3.** `RUN_SNAPSHOT` and `RESUME_INDEX` stop being written. The safety proof is the resume/suspend test set, which
must stay green with unchanged counts.

**R4.** No retention deletion is implemented in this spec: the numbers from R1 are the input to a later policy. Do not
write `[retention]` yet.

## The requirement a plausible implementation satisfies in appearance only

A worker can add `store stats` and print the file size of each `.redb` — that is not per-table and cannot support a
policy. The test asserts a per-table row count that changes when a row is written. R3 is the half most likely to be
"implemented" by commenting out the writer: the resume tests must still pass, which is what proves nothing needed it.

## TDD contract

| file | test | assertion |
|---|---|---|
| `crates/pii-store/tests/df_counts_documents.rs` | `a_document_put_twice_is_counted_once` | `put(d)` twice → `df(term) == 1` |
| `crates/pii-store/tests/stats_are_per_table.rs` | `a_written_row_changes_its_tables_count` | write one run → the run table's count rises by 1 and another table's does not |
| `crates/pii-graph/tests/resume.rs` (existing) | all tests | unchanged count, still green — this is the proof for R3 |

## What is already true (measured 2026-11-04, do not re-derive)

- **f7 is done.** `pii-store/src/mem/write.rs` bumps `df` only when the document is genuinely new
  (`if existed { return Ok(()) }` before `bump_df`), the same guard covers `n_docs`/`sum_dl`, and
  `df_counts_documents` passes. It landed swept into row 353's commit.
- `RUN_SNAPSHOT` is gone (`grep -c` is 0).
- **f12 is open**: `RESUME_INDEX` is declared, opened three times, written by
  `pii-graph/src/sched/dispatch.rs` — where the code's own comment says *"nothing reads
  RESUME_INDEX"* — and counted into identity stats. Write-only state that is also reported as if it
  mattered. Remove it, table definition and count included.
- **f19 is open**: there is no retention code in `pii-store` at all (`grep -c retention|prune` is 0).

## Files

- `crates/pii-store/src/tables.rs`
- `crates/pii-store/src/identity.rs`
- `crates/pii-graph/src/sched/dispatch.rs`

## Exit Criteria

- [ ] `cargo test -q -p pii-store --test df_counts_documents 2>&1 | grep -qE 'test result: ok. 1 passed'`
- [ ] `cargo test -q -p pii-store --test stats_are_per_table 2>&1 | grep -qE 'test result: ok. 1 passed'`
- [ ] `! grep -rq 'RESUME_INDEX' crates/ --include='*.rs'`
- [ ] `cargo test -q -p pii-store --test retention_drops_what_the_policy_expires 2>&1 | grep -qE 'test result: ok'`

## Guardrails

- Do NOT delete a store or a table; stop writing and let compaction reclaim.
- Do NOT implement retention in this spec (R4).
- Do NOT change BM25 scoring constants; fix only the double count.
- The worker does not commit; the orchestrator reviews and commits.

## Files

- `crates/pii-store/src/mem/write.rs`
- `crates/pii-store/src/lib.rs`
- `crates/pii-graph/src/journal.rs`
- `crates/pii-graph/src/sched/dispatch.rs`
- `crates/pii-cli/src/cmd_store.rs`
- `crates/pii-store/tests/df_counts_documents.rs` (new)
- `crates/pii-store/tests/stats_are_per_table.rs` (new)

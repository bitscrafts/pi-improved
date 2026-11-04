# Spec: The scheduler writes no index that nothing reads

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening · **Source**: Opus finding 12, split out of row 358
(the retention half of that row is now row 376).

## Why this exists

`RESUME_INDEX` was declared in `pii-store/src/tables.rs`, opened three times in `pii-store/src/lib.rs`, written by
`pii-graph/src/sched/dispatch.rs` — where the code's own comment said *"nothing reads RESUME_INDEX"* — and counted
into the identity stats.

So pii paid to write an index on every resume and then reported it as a store statistic, which is the worst of both:
the cost is real, and the number invites an operator to trust a structure with no reader. A store is not a place to
keep a guess about the future.

Removing it touches the table definition, every `open_table`, the write site, the journal helper that mentions it, and
the identity count.

## Files

- `crates/pii-store/src/tables.rs`
- `crates/pii-store/src/lib.rs`
- `crates/pii-store/src/identity.rs`
- `crates/pii-graph/src/journal.rs`
- `crates/pii-graph/src/sched/dispatch.rs`

## Exit Criteria

- [ ] `! grep -rq 'RESUME_INDEX' crates/ --include='*.rs'`
- [ ] `cargo test -q -p pii-graph --test resume 2>&1 | grep -qE 'test result: ok'`
- [ ] `cargo test -q -p pii-store --test stats_are_per_table 2>&1 | grep -qE 'test result: ok. 1 passed'`
- [ ] `cargo test -q -p pii-store --test df_counts_documents 2>&1 | grep -qE 'test result: ok. 1 passed'`

## Guardrails

- Do NOT drop `RUN_JOURNAL` or `RUN_BY_STATUS` — they are read.
- Do NOT change the identity stat names that remain; a rename would silently break the host conformance probe.
- Do NOT remove the scheduler's resume behaviour, only the index it wrote for it.

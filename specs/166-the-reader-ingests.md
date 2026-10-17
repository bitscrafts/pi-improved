# Spec: the incident lives in the store, with an id

## Overview
Tonight the reader found 1,425 identical refusals on the daemon's real log and opened an incident — as a printed line.
Ingest it, so the incident has an id and a fingerprint in the store and the card's incident field is a query rather than
a quote.

Today `pii incidents scan --source runtime --log <file>` (`crates/pii-cli/src/cmd_incidents.rs::scan_runtime`) prints
`incident <fingerprint> count=… source=…` and keeps nothing; `RuntimeIncident`
(`crates/pii-learn/src/incident_runtime.rs:30`) has no id, and nothing under `crates/pii-learn/src/` writes an incident.
The deploy card (`crates/pii-cli/src/cmd_deploy_card.rs:19`) requires `incident.id`, so an operator has to copy one in
by hand.

**Project**: /projects/pi-improved

## Requirements

### Functional
- R1. A file-backed incident store in `pii-learn`, shaped like the candidate store (`crates/pii-learn/src/store.rs`):
  one JSON file per incident at `<dir>/incidents/<id>.json`, written to a temp file and renamed into place.
- R2. The id is derived from the fingerprint and the source alone (e.g. a short hex digest), so ingesting the same
  log twice yields the same id and ONE file — re-ingest updates `count`/`last_seen`, it never creates a second row.
- R3. A stored incident carries at least: `id`, `fingerprint`, `source`, `count`, `first_seen`, `last_seen`,
  `opened_at`.
- R4. `pii incidents scan --source runtime --log <file> --ingest --dir <dir>` writes every incident the scan opened and
  prints `incident id=<id> fingerprint=<fp> count=<n>` per incident; without `--ingest` the command's output and
  behaviour are byte-identical to today's (spec 160's criteria still hold).
- R5. `pii incidents show --dir <dir> --id <id>` prints the stored incident; an unknown id exits non-zero.
- R6. The source log is read, never written (the existing `the_source_log_is_read_never_written` stays green).

### Non-Functional
- No `unwrap`/`expect`/`panic!` in production code; errors are typed and reach the CLI as a non-zero exit.
- Nothing outside the test's own `tempfile::TempDir` is written by any test (spec 136).

## TDD Contract
New file `crates/pii-learn/tests/incidents_ingest.rs`, using `tests/fixtures/runtime-stalled.jsonl` and
`tests/fixtures/runtime-clean.jsonl` (the fixtures `incidents_runtime.rs` already reads). Each test must fail on
today's tree:
- `an_opened_incident_is_stored_with_an_id` — scan the stalled fixture, ingest, then read back by id: same fingerprint
  and count.
- `ingesting_twice_is_one_row_with_the_same_id` — ingest the same scan twice: exactly one file under `incidents/`,
  same id. (N=2, so an overwrite or a duplicate is visible — DO-NOT-REPEAT 1.49.)
- `two_fingerprints_are_two_ids` — a log with two distinct refusals ingests to two different ids.
- `the_clean_control_stores_nothing` — the clean fixture ingests to zero files (the control).
- `an_unknown_id_is_an_error` — reading back an id that was never written returns an error, not a default.

## Exit Criteria
- [ ] `cargo test -p pii-learn --test incidents_ingest 2>&1 | grep -q 'test result: ok. 5 passed'`
- [ ] `cargo test -p pii-learn --test incidents_runtime_production 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-learn --test incidents_runtime 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `D=$(mktemp -d) && cargo run -q -p pii-cli -- incidents scan --source runtime --log tests/fixtures/runtime-stalled.jsonl --ingest --dir "$D" | grep -qE '^incident id=[0-9a-f]+ ' && ls "$D"/incidents/*.json >/dev/null`

## Guardrails
- Do NOT weaken an existing budget or control to make this pass; change the design instead.
- Do NOT edit, delete or rename an existing test; the four criteria above count tests, so a removed test is visible.
- If the code's shape does not match this spec (e.g. the scan cannot be fed to an ingest without changing spec 160's
  output), STOP and say so instead of forcing it.
- Do NOT commit; the loop's orchestrator commits.

## Closure
- 2026-10-17 (supervisor): the first pass was closed as `done` with `last=satisfied by 0e4b672` and was NOT done.
  The worker (session a15785c7, 6 turns) changed nothing; the cycle found 0 dirty paths, ran the old sole criterion
  `cargo test -p pii-learn --test incidents_runtime_production` — a file added 2026-10-16 01:32 (a8403f0), before this
  spec existed (5bf0db7, 2026-10-17 04:01) — saw it green, and closed the item. `0e4b672` is a 22-line review note,
  unrelated. Checked on the tree: no source file in `pii-learn`, `pii-cli` or `pii-store` was touched after the spec was
  written, and `RuntimeIncident` has no id. The criterion could not fail, so it certified nothing. The spec now carries
  criteria that are red on today's tree, and the item is back to `pending`.

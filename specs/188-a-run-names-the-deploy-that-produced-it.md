# Spec: 188 — a run names the process and the deploy that produced it

**Project**: /projects/pi-improved
**Phase**: after 187 (both from 070). Filed 2026-10-18 by the orchestrator; 070 §5 names the instrument and this
spec is its first half.

## Overview

MEASURED 2026-10-18 (070 §5, C-run on production and on the deployed binary): **this project cannot attribute an
event to the deploy that caused it.**

- A run record carries `{dur_ms, graph, run_id, seq, status}` and nothing else (`RunHeader`,
  `crates/pii-abi/src/journal.rs:29`) — **no revision, no timestamp** (`pii run ls` keys, C-run).
- `pii health`'s counters are **per process** and reset: `channel_posts: 1` while the store held 10 posts from
  that morning's occurrence (`crates/pii-server/src/daemon/identity.rs` fills `Counts` from in-memory hubs).
- The logs rotate across revisions.

The cost of that gap is on record, not hypothetical: at 2026-10-18 09:00 UTC one daily digest produced
**10 `channel.posted` and 1,836 `channel.post_refused`** (1,850 runs in the store), and the only way to see that
they came from **ten different processes** was arithmetic on run-id allocation blocks (`r…0001`, `r…0033`, …,
`r…0321` — 32 ids per process). Three readers that day reported the wrong thing with confidence: the exercise
matrix marked a failing occurrence PASS, `pii health` showed no trace of it, and the ids were the only clue.
Four armed digest runs are on disk again for `cron_entries: 1`; the next occurrence is 2026-10-19 09:00 UTC.

**The instrument (070 §5, verbatim):** every run record carries `{revision, process_start_ms, created_ms}`, and
`pii health` adds a `since_deploy` block **read from the store**, keyed by the running revision.

## Requirements

### Functional

- **R1 — the run header carries its origin.** `pii_abi::RunHeader` gains `revision: String`,
  `process_start_ms: u64` and `created_ms: u64`. **Every one of them is `#[serde(default)]`**: run headers are
  JSON in redb (`crates/pii-store/src/codec.rs`), and the store already holds 1,850 rows written without these
  fields. An old row MUST decode (empty revision, 0), not error.
- **R2 — the creation path stamps them, once.** The run-creation path stamps the three fields from an **origin
  value built once per process**: `revision` is the existing build identity (`PII_GIT_REV` through
  `crates/pii-server/build.rs`; `pii_server::daemon::identity::REVISION`), `process_start_ms` is captured **once
  per process** (never per run), and `created_ms` is the wall clock when the run is accepted.
- **R3 — `since_deploy` is read from the store, not from counters.** `pii_proto::sys::StatusResult` gains
  `#[serde(default)] pub since_deploy: SinceDeploy` with
  `{ revision, runs, runs_by_status: {running, suspended, succeeded, failed}, armed, unattributed }`, computed by
  reading run **headers out of the store** and counting only those whose `revision` equals the running binary's
  revision. `armed` is the suspended count for this revision (`cron_entries` is the expected value; the gap
  between them is the 187 defect made visible). Additive and `#[serde(default)]`, so an older server's payload
  still parses and an older client still reads a newer one.
- **R4 — unattributed is counted, never absorbed.** A run with an empty revision (written before this change, or
  by an older binary) is counted in `unattributed` and **never** in `runs` for the running revision. A `runs`
  that silently absorbed the 1,850 old rows would report the current deploy as the author of this morning's
  flood — a false green of exactly the kind this spec exists to prevent.
- **R5 — it is legible.** `pii health` (text mode, `crates/pii-cli/src/cmd_health.rs`) prints the block when the
  key is present; `--json` carries it unchanged.
- **R6 — it survives the process.** A **fresh** store handle reads the same attribution back: the read is the
  criterion, an in-memory counter is not evidence. This is also the reason the arming defect (187) could not be
  seen before: the number that would have shown it died with each process.
- **R7 — mutation control.** Emptying the revision stamp (or making `since_deploy` read the hub counters instead
  of the store) reddens the attribution test. Paste the red output in the Closure.

### Non-Functional

- 450-line file rule (spec 182; `daemon/identity.rs` is near it — put the `since_deploy` builder in its own
  module, e.g. `crates/pii-server/src/daemon/since_deploy.rs`, rather than growing `identity.rs` past the gate).
- No new dependency. `cargo fmt` + `clippy -D warnings` clean. Tests create their scratch under `_tmp` with the
  sanctioned helper (`pii_store::testing::tmp_scratch` or an explicit `create_dir_all`).
- The store read is bounded: one pass over the run table, no per-run `get_run` fan-out. 1,850 headers per status
  call is measured-acceptable; if the store cannot iterate the run table without materialising every header, say
  so and STOP — do not add an index in this spec.

## TDD Contract

| Test Name | Input | Expected Output |
| --- | --- | --- |
| `a_new_run_names_its_process_and_deploy` | one run created through the real path with origin `{revision: "abc123", process_start_ms: 4242}` at `created_ms` 1000 | the header read back by a **second** store handle has `revision == "abc123"`, `process_start_ms == 4242`, `created_ms == 1000` |
| `an_old_header_still_decodes` | a header JSON in the shape written before this change (no `revision`, `process_start_ms`, `created_ms`) | it decodes, the three fields are empty/0, no error |
| `since_deploy_counts_only_this_revision` | a store holding 2 runs at `abc123`, 1 at `def456`, 1 with no revision | `since_deploy.runs == 2`, `runs_by_status` sums to 2, `unattributed == 1`; the `def456` run contributes nothing |
| `since_deploy_survives_a_restart` | the runs above, then the store/engine dropped and a fresh handle opened | the same counts, read from the store |
| `a_header_without_a_revision_is_never_this_deploy` (control) | the same store, status built for revision `def456` | `runs == 1` and still `unattributed == 1` — the near-miss proves R4 rather than the happy path alone |
| `two_runs_of_one_process_share_their_start` | two runs in one process | both carry the same `process_start_ms`; a run created after a restart carries a different one |

## Exit Criteria

- [ ] `cargo test -p pii-abi --lib`
- [ ] `cargo test -p pii-store --test store_contract`
- [ ] `cargo test -p pii-server --test run_attribution`
- [ ] `cargo test -p pii-server --lib`
- [ ] `grep -q 'since_deploy' crates/pii-proto/src/sys.rs && grep -q 'since_deploy' crates/pii-cli/src/cmd_health.rs`

## Guardrails

- **Additive only.** `#[serde(default)]` on every new field and on the new `StatusResult` field. Do not rewrite,
  migrate or delete the 1,850 existing run rows; do not bump a store version for this.
- **One source for the revision.** The build identity already exists (`PII_GIT_REV` → `build.rs` →
  `identity::REVISION`). Do not read git at runtime, do not invent a version string, do not add a second source.
- **`process_start_ms` is per process.** A per-run `now_ms` in that field is a defect; the test must show two
  runs of one process sharing it and a restart changing it.
- **`pii-graph` must not depend on `pii-server`.** Pass the origin as **data** (a value) down the creation path.
  If the header is created somewhere that cannot receive it without a new crate edge, say what the edge would be
  and STOP rather than adding it.
- **Never fabricate attribution at report time.** No revision on the header means `unattributed`; nothing else.
- **Do not weaken a gate, a test, an assertion or an existing criterion** to make this pass.
- **Commit paths:** `crates/pii-abi/src`, `crates/pii-proto/src/sys.rs`, `crates/pii-store/src`,
  `crates/pii-graph/src`, `crates/pii-server/src`, `crates/pii-server/tests`, `crates/pii-cli/src`. If the change
  needs a file outside those, STOP and name it — a file left out of the commit is a HEAD that does not build.
- **Scope boundary, stated so it is not quietly expanded:** `posted`/`post_refused` counted from the store, and
  the per-cron `runs_per_occurrence == 1` assertion, are **not** in this spec (070 §5's second and third bullets).
  They are the next spec's. Do not stub them here.

## Closure

(To be filled by the orchestrator after independent verification: the criteria's own output, the mutation's red
output, the byte-identical restore, and what this does and does not close in 070 §1.)

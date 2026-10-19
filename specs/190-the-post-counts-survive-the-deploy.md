# Spec: 190 — the post counts survive the deploy

**Project**: /projects/pi-improved
**Phase**: after 188 (both from 070 §5). Filed 2026-10-19 by the orchestrator. 188 built the first half of 070 §5's
instrument (a run names its revision and its process); this is the second half — the posts.

## Overview

MEASURED 2026-10-18 (070 §5 and §0.1, C-run): **`pii health` cannot say how many posts this deploy made.**

- The only post counter on that surface is `counts.channel_posts`, filled from an **in-memory hub** in
  `crates/pii-server/src/daemon/identity.rs`. It dies with the process. At the 09:00 occurrence it read
  **`channel_posts: 1`** while the store held **10 `channel.posted` and 1,836 `channel.post_refused`** — a
  false green on the one number the user's phone depends on.
- The post outcome is written **only to the rotating observer log**: `crates/pii-server/src/node_kinds/nodes/post.rs:140-150`
  emits `channel.posted` (skipped for a duplicate), `:170-180` emits `channel.post_refused`. Logs rotate across
  revisions, so after the next deploy those lines are not evidence about *this* deploy.
- `since_deploy` (spec 188, `crates/pii-server/src/daemon/since_deploy.rs`) already counts runs per revision **from
  run headers in the store** — but it counts nothing a run *did*.

The gap is the difference between "186 runs happened" and "one of them reached the phone". The exercise matrix
marked E10 PASS while the post was refused, because the count it read came from a process that had already
exited.

**The instrument (070 §5, verbatim):** `pii health` adds a `since_deploy` block read **from the store**, keyed by
the running revision: `{runs_by_status, posted, post_refused, starts, hard_deaths}`. This spec is `posted` and
`post_refused`; the per-cron occurrence assertion and the process-liveness pair stay out of it.

**The store already holds both facts**, which is why this needs no new table: the run's **graph body** is stored
(`GRAPH_BODY`, `crates/pii-store/src/tables.rs:19`) and names which nodes have `kind = "post"`, and the run's
**journal** (`RUN_JOURNAL`, `:25`) records each of those nodes' `NodeFinished`/`NodeFailed`, including the
`duplicate` flag the post node returns.

## Requirements

### Functional

- **R1 — `posted` and `post_refused` are on `since_deploy`.** `pii_proto::sys::SinceDeploy` gains
  `#[serde(default)] pub posted: u64` and `#[serde(default)] pub post_refused: u64`, next to `runs` and `armed`.
  Additive: an older payload still parses, an older client reads a newer one.
- **R2 — they are counted from the store, never from a counter.** For every run whose header `revision` equals the
  running binary's revision (the rule spec 188 established), read that run's stored graph body to find the nodes
  declared `kind = "post"`, then read that run's journal:
  - `NodeFinished` on a post node whose value has `duplicate` **false** (or absent) → `posted += 1`;
  - `NodeFinished` on a post node with `duplicate: true` → **neither** counter moves (the idempotence key did its
    job; the post was made by an earlier run and counting it twice is the defect this project already fixed once);
  - `NodeFailed` on a post node → `post_refused += 1`.
  The rotating log file is not an input and no in-memory counter is an input.
- **R3 — unattributed runs contribute nothing.** A run with an empty revision is not this deploy's, exactly as
  spec 188's R4 says for `runs`; its posts are not counted as `posted` and not as `post_refused`.
- **R4 — a run whose posts cannot be read is counted, not guessed.** If a run's graph body is absent, or its
  journal cannot be read, its posts are **not** silently counted as zero: the block gains
  `#[serde(default)] pub unreadable: u64` (runs of this revision whose posts could not be counted) and the reader
  reports it. A zero with an unreadable witness is a finding; a zero without one is a lie.
- **R5 — it survives the process.** A **fresh** store handle, after the daemon that wrote the posts is gone, reads
  the same `posted`/`post_refused`. The read is the criterion; an in-memory counter is not evidence.
- **R6 — it is legible.** `pii health` text mode prints the two numbers with the rest of the block
  (`crates/pii-cli/src/cmd_health.rs`), and `--json` carries them unchanged.
- **R7 — mutation control.** Making either counter read the in-memory hub, counting `duplicate: true` as posted, or
  counting every revision regardless of header, reddens its test. Paste the red output in the Closure.

### Non-Functional

- **Bounded read.** One pass over the run table (as 188 already does); journal reads only for the runs of this
  revision. Do **not** add an index or a new table in this spec. If the run table cannot be iterated without
  materialising every header, or a graph body cannot be fetched by the run's graph name, say so and STOP.
- **450-line file gate.** `crates/pii-server/src/daemon/since_deploy.rs` is 55 lines today; if the counting grows
  it past the limit, put the post counting in `crates/pii-server/src/daemon/since_deploy/posts.rs` (a submodule,
  same crate) rather than shrinking comments.
- No new dependency. `cargo fmt` and `clippy -D warnings` clean. Tests create their scratch under `_tmp`
  (`pii_store::testing::tmp_scratch` or an explicit `create_dir_all`), never `/tmp`.
- The post node's behaviour does not change: same events, same idempotence, same refusals. This spec adds a
  reader, not a writer.

## TDD Contract

| Test Name | Input | Expected Output |
| --- | --- | --- |
| `posted_counts_a_successful_post_of_this_revision` | a run at revision `abc123` whose graph has one `post` node and whose journal holds `NodeFinished{port:"ok", value:{duplicate:false}}` on it | `since_deploy.posted == 1`, `post_refused == 0` |
| `a_refused_post_is_counted_as_refused` | the same shape with `NodeFailed` on the post node | `post_refused == 1`, `posted == 0` |
| `a_duplicate_post_is_not_counted_twice` (control) | `NodeFinished` on the post node with `value.duplicate == true` | `posted == 0` — the near-miss, so the counter cannot be "any NodeFinished on a post node" |
| `posts_of_another_revision_are_not_this_deploy` (control) | one posting run at `abc123` and one at `def456` | at `abc123`: `posted == 1`; at `def456`: `posted == 1`; neither reads 2 |
| `posts_survive_the_process` | the `abc123` posting run, then the store dropped and reopened by a fresh handle | `posted == 1` read from the second handle |
| `an_unreadable_run_is_counted_not_silently_dropped` | a run at `abc123` whose graph body is absent from the store | `posted == 0`, `post_refused == 0`, `unreadable == 1` |

## Exit Criteria

- [ ] `cargo test -p pii-server --test since_deploy_posts`
- [ ] `cargo test -p pii-server --lib`
- [ ] `cargo test -p pii-proto --lib`
- [ ] `grep -q 'post_refused' crates/pii-proto/src/sys.rs`

## Guardrails

- **Additive only.** `#[serde(default)]` on every new field. Do not rewrite, migrate or re-stamp existing run
  rows, do not bump a store version, do not add a table.
- **The store is the only input.** Reading the daemon log file, the in-memory `Counts`, or the `pii-observe`
  file sink to answer these two numbers is exactly the defect (070 §5: the logs rotate across revisions).
- **Do not count a duplicate.** The post node skips its event on `PostOutcome::Duplicate` deliberately
  (`post.rs:139`); a counter that counts it turns idempotence into a phantom post on the operator's phone.
- **Do not change the post path** to make counting easier (no new event kind, no new field on `PostRequest`).
  If the journal genuinely does not carry the outcome the counter needs, STOP and name what is missing — the fix
  then belongs in its own spec with its own red criterion.
- **Never fabricate a number at report time.** No graph body → `unreadable`, never `posted: 0` presented as good.
- **Do not weaken a gate, a test, an assertion or an existing criterion** to make this pass.
- **Commit paths:** `crates/pii-proto/src/sys.rs`, `crates/pii-server/src/daemon`, `crates/pii-server/tests`,
  `crates/pii-cli/src/cmd_health.rs`. If the change needs a path outside those, STOP and name it — a file left out
  of the commit is a HEAD that does not build.
- **Scope boundary, stated so it is not quietly expanded:** `starts`/`hard_deaths` (the process-liveness pair) and
  the per-cron `runs_per_occurrence == 1` assertion are **not** in this spec. They are the next one. Do not stub
  them here.

## Closure

**Verified 2026-10-19 08:47Z by the orchestrator, independently of the worker's report, on the committed tree
(`121fb6d`).**

*The four criteria, run by hand:*

- `cargo test -p pii-server --test since_deploy_posts` → `test result: ok. 6 passed; 0 failed`, rc=0.
- `cargo test -p pii-server --lib` → `test result: ok. 129 passed; 0 failed`, rc=0.
- `cargo test -p pii-proto --lib` → `test result: ok. 7 passed; 0 failed`, rc=0.
- `grep -c 'post_refused' crates/pii-proto/src/sys.rs` → 1 (the field is on the wire type).

*The near-miss controls are real, not named.* The committed battery carries every control the TDD Contract
demanded, including the ones a lazy version would omit: `a_duplicate_post_is_not_counted_twice` (a post node
finishing with `duplicate: true` moves NEITHER counter — the counter cannot be "any NodeFinished on a post
node"), `posts_of_another_revision_are_not_this_deploy` (two revisions with one posting run each read 1 at
their own revision, never 2 — the counter cannot be "any posting run"), `posts_survive_the_process` (the
count is read back from a freshly-opened store handle after the writer is dropped — the in-memory hub is not
an input), and `an_unreadable_run_is_counted_not_silently_dropped` (a run with no graph body reads
`unreadable == 1`, never `posted == 0` presented as good).

*Live agreement measured 08:40Z.* The running daemon (revisão `121fb6de67e1`) reads against the production
store: `posted 0, post_refused 0, unreadable 0` — correct, because zero posts have been *attempted* by this
revision (the 1,836 refusals in the rotating log are yesterday's revision's work, correctly outside since-
deploy, visible in `unattributed: 1850`).

*What this closes, and what it does not.* `pii health` now reports what a deploy *did* from the store, so the
09:00-class false green (counter said 1, store held 1,836) cannot happen again. It does NOT build the per-cron
`runs_per_occurrence == 1` assertion (excluded by the scope boundary, next spec), and it does not touch the
post path's behaviour — events, idempotence and refusals are unchanged (this spec adds a reader, not a
writer). The E6 defect (nothing READS `retry_after` to pace a refused post) also stays open: counting a
refusal is not pacing one.

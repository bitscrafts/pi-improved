# Spec: the CLI's learn revert cannot race the daemon's sweep

## Overview

Filed with spec 125 and still open in `_docs/05-operations/open-items.md` §9: `pii learn revert` from a CLI process
and the daemon's own auto-revert sweep can act on the same promotion at the same time — two writers on one
record. This is the same class as the audit fork (§1), whose resolution the project already chose and shipped in
spec 138: **one writer**, the daemon applies and records.

**Project**: /projects/pi-improved
**Phase**: the gate's own robustness (pre-video backlog, specs 145–148)

## Requirements

### Functional

R1. When a daemon owns the store, a CLI revert is a **request**, applied by the daemon through the single-writer
    path spec 138 built. The CLI does not write the promotion record itself in that case.
R2. When no daemon owns the store, the CLI may apply the revert directly — the documented degraded path — and it
    must say which path it took in its output.
R3. Two reverts of one promotion (one from the CLI, one from the sweep) result in exactly one application and one
    "already reverted": idempotent, with no double bookkeeping in the ledger.
R4. The decision ledger records which principal acted (`cli` or `daemon`) for every revert.

### Non-Functional

R5. The request path must not lose a revert when the daemon is busy: the request is durable (the store it lands
    in survives a restart) and the daemon applies it on its next pass.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| a_cli_revert_is_a_request_when_the_daemon_owns_the_store | daemon holding the store, CLI revert | the daemon applies it once; the CLI writes no record |
| a_cli_revert_applies_directly_without_a_daemon | no daemon | applied, and the output says which path |
| two_reverts_are_one_revert | CLI + sweep on the same promotion | one application, one "already reverted" |
| the_ledger_names_the_principal | the same | the ledger row says cli or daemon |

## Exit Criteria

- [ ] `cargo test -p pii-learn -p pii-server --lib` prints `test result: ok`
- [ ] `grep -c 'append(true)' crates/pii-learn/src/**/*.rs` is 0, or every site is reachable only from the daemon
      (the same criterion spec 138 set for promotions)
- [ ] `cargo xtask fresh-gate` on this spec's commit is `ALL PASS`

## Guardrails

- Do NOT add a second writer. Reuse spec 138's path.
- Do NOT silently ignore a revert request when the daemon is not running; print the path taken.
- Do NOT make the CLI wait indefinitely for the daemon.

## Closure (2026-10-13, orchestrator, independent of the worker's report)

**The criteria pass on the tree the worker left (HEAD `bd24f26`).** `cargo test -p pii-learn -p pii-server
--lib` → `test result: ok` (pii-server 125 passed · 1 ignored; `pii-learn` 0 tests under `--lib` — its tests are
integration tests, so that half of criterion 1 is satisfied by an empty set). Criterion 3 `fresh-gate` PASS: my
own run rc 0, `quality-gate: ALL PASS`, on HEAD `e38a143` (this commit plus two specs and two plan rows, no
code); the cycle's own run on `bd24f26` at 18:16 also `PASS`.

**My mutation — the load-bearing guard is real.** `Promoter::writer()` altered to create `learn/.writer.lock`
and never lock it (`crates/pii-learn/src/promote/mod.rs:57`; types spelled out so the crate still compiles)
reddens `the_writer_lock_is_held_across_processes` at `crates/pii-learn/tests/promote.rs:546` —
`the revert wrote while another process held the writer lock`, rc 101 — while unmutated the same test is `ok`
(rc 0). Source restored byte-identical:
`ff3774734be66ad0ea38ee64f944a7ef267ee2d23088a881916326b1ad07f100` before and after, `git status` clean.
`two_reverts_are_one_revert` (2 ledger rows, one "already reverted") and `the_ledger_names_the_principal`
(`cli…` / `daemon`) pass.

**Three defects this green hides — recorded here, not fixed here (code changes are a new item; specs are the
planner's).**

1. **276 lines, 6 tests, deleted and replaced by nothing.** `bd24f26` emptied
   `crates/pii-server/src/daemon/learn_rpc_tests.rs` (277 lines → 1 byte):
   `a_promotion_reads_back_after_a_restart`, `a_request_for_another_store_is_refused_not_written_here`,
   `the_daemon_reads_an_untouched_graph_as_unsatisfied`, `the_daemon_reads_an_edited_graph_as_satisfied`,
   `the_unattended_pass_runs_without_an_operator_and_never_promotes`,
   `the_unattended_pass_without_a_trajectory_root_says_so`. The code they cover
   (`crates/pii-server/src/engine/learn_methods.rs`, unchanged since `2e367b9`) and the declaration
   (`crates/pii-server/src/daemon/mod.rs:13`) both stay. They were live, not stale: restored from `bd24f26^`
   onto the committed tree, `cargo test -p pii-server --lib learn_rpc_tests` → `6 passed; 0 failed`. After the
   deletion **no test in the tree drives the daemon's learn RPC handlers**
   (`Method::LearnPromote/LearnRevert/LearnSweep` → `crates/pii-server/src/engine/dispatch.rs:142-144`); the
   callers left are production code and the CLI. Criterion 1 is therefore a criterion a deletion satisfies:
   `cargo test -p pii-server --lib` runs **126** tests on this commit and **132** with those six back.
2. **R1 has no test.** The TDD Contract's first row, `a_cli_revert_is_a_request_when_the_daemon_owns_the_store`
   (the daemon applies it once; the CLI writes no record), does not exist (`grep -rn` over `crates/` = 0): 3 of
   4 contract rows are implemented, and the missing one is the requirement the item is named after.
3. **Criterion 2 is false as written on the committed tree.** The only `append(true)` in
   `crates/pii-learn/src/` is `promote/storage.rs:84` (`append_ledger`), and this item adds a second reachable
   caller: `crates/pii-cli/src/cmd_learn.rs:437` → `Promoter::revert` in the CLI process (R2's documented
   direct path). Neither branch holds — the count is 1, and that site is no longer "reachable only from the
   daemon". The property the item actually built is different and stronger: every writer takes the
   cross-process `learn/.writer.lock`. Amending the criterion to that property (or dropping R2's direct write)
   is a planner decision; it was **not** weakened here.
4. Read from the code, not exercised by any test: `writer()`'s `.ok()`
   (`crates/pii-learn/src/promote/mod.rs:54-58`) turns a failure to create or lock the file into `_file: None`,
   so the write proceeds under the process mutex alone — fail-open on the very path whose claim is "cannot
   race". One test decides it (an unwritable `learn/`, or a lock the process cannot take).

**Process note.** The plan row A16's `verify` column carried one test (`two_reverts_are_one_revert`), not the
spec's three criteria — the paraphrase pitfall; and criterion 1 with `--lib` runs zero tests in `pii-learn`,
whose tests live in `crates/pii-learn/tests/`.

**Evidence** (before/after sha256 of both restored files, `mutate_lock.py`, the green and red logs,
`fresh-gate.log`): `_tmp/verify-149-20261013T182043Z/`. The item stays `done`; the repair — restore the six
tests, add R1's, decide criterion 2 — belongs to a new spec.

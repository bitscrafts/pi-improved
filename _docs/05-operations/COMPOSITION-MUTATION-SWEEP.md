# COMPOSITION MUTATION SWEEP — first run (spec 286)

2026-10-26, on commit `3e91402` plus the uncommitted spec-286 files (the sweep ran on the working tree, which holds no
change under `crates/*/src` or `graphs`). Run with `bash scripts/mutate/composition.sh`; it takes about a minute on a
warm target dir. Its self-test is `bash scripts/mutate/tests/composition-sweep-test.sh` (PASS: 8 FAIL: 0).

Result: `MUTATION-SWEEP: required 15/18 probes 2`, exit 1. Three required rows are not RED: two SURVIVED and one
BASELINE-RED. The sweep exits nonzero until they are resolved.

Each row is one mutation, applied alone to a scratch copy of the tree; only the named test file is run there. The
table is `scripts/mutate/composition_mutations.py`.

| id | kind | file | test | result |
|---|---|---|---|---|
| m01 | required | crates/pii-graph/src/fragment/call.rs | pii-graph/call_inlines_a_graph | RED |
| m02 | required | crates/pii-graph/src/fragment/call.rs | pii-graph/call_inlines_a_graph | RED |
| m03 | required | crates/pii-graph/src/fragment/call_needs.rs | pii-graph/call_needs | RED |
| m04 | required | crates/pii-graph/src/fragment/call_needs.rs | pii-graph/call_needs | RED |
| m05 | required | crates/pii-graph/src/fragment/call_needs.rs | pii-graph/call_needs | RED |
| m06 | required | crates/pii-graph/src/fragment/call.rs | pii-graph/call_inlines_a_graph | RED |
| m07 | required | crates/pii-graph/src/fragment/call.rs | pii-graph/call_inlines_a_graph | RED |
| m08 | required | crates/pii-graph/src/fragment/call.rs | pii-graph/call_inlines_a_graph | SURVIVED |
| m09 | required | crates/pii-graph/src/fragment/call.rs | pii-graph/call_records | SURVIVED |
| m10 | required | crates/pii-graph/src/run/call_events.rs | pii-graph/run_journals_calls | RED |
| m11 | required | crates/pii-graph/src/fragment/call.rs | pii-graph/call_inlines_a_graph | RED |
| m12 | required | crates/pii-graph/src/fragment/call.rs | pii-graph/call_inlines_a_graph | RED |
| m13 | required | crates/pii-graph/src/fragment/call.rs | pii-graph/call_inlines_a_graph | RED |
| m14 | required | crates/pii-server/src/node_kinds/nodes/coder/budget.rs | pii-server/coder_charges_run_budget | RED |
| m15 | required | crates/pii-server/src/node_kinds/nodes/coder/graph_node.rs | pii-server/coder_charges_run_budget | RED |
| m16 | required | crates/pii-server/src/node_kinds/nodes/coder/session_path.rs | pii-server/coder_session_per_node | RED |
| m17 | required | crates/pii-server/src/engine/runs.rs | pii-server/spec_graph_runs_through_the_daemon | BASELINE-RED |
| m18 | required | crates/pii-graph/src/fragment/check.rs | pii-graph/call_inlines_a_graph | RED |
| m19 | probe | crates/pii-graph/src/fragment/call_needs.rs | pii-graph/call_needs | RED |
| m20 | probe | crates/pii-graph/src/fragment/call_needs.rs | pii-graph/call_needs | RED |

MUTATION-SWEEP: required 15/18 probes 2

Every anchor occurred exactly once in its file, so no row is NO-APPLY. Every restored scratch file matched the digest
of its live file, so the sweep did not abort.

## Survivors

- **m08 — the call-site caps check** (`call.rs`, `if !widen.is_empty()`), named test `call_inlines_a_graph`. Removing
  the check still passes the test, because the node-level check in `load_sub` (`graph.call.widen` on each sub-graph
  node) refuses the same widening. The call-site check is redundant for that test, so the sweep cannot tell them apart.
  The next spec must add a case the node-level check does not catch: a call whose own `caps` widens, with no sub-graph
  node that needs the extra capability.
- **m09 — the call record is not attached to the call head** (`call.rs`, the `call_needs::mark` in `splice`), named
  test `call_records`. It passes without the record. It is caught by `run_journals_calls` (probed separately: RED), so
  the named file does not hold the record's attachment. The next spec should move the record assertion into
  `call_records` (or name `run_journals_calls` as the catcher in the table).
- **m17 — the daemon path builds the budget without `[limits]`** (`engine/runs.rs`), named test
  `spec_graph_runs_through_the_daemon`, which is **BASELINE-RED**. Its `CONTROL_a_missing_docs_brief_refuses_before_any_agent`
  case fails on the live tree too, before any mutation, so the row cannot be judged. This is not a scratch-tree artifact.
  A probe run with `pii-server/spec_graph_spend_limit_bites` instead gives SURVIVED, so no other named test catches
  the removal either. The next spec should repair the red CONTROL case, then re-judge m17.

## Method notes

- The scratch tree is a copy of the files `git ls-files -co --exclude-standard` lists (tracked and untracked, minus
  ignored), so the untracked daemon tests are included. A `git worktree` would have left them out.
- Each row restores the scratch file from the live file and compares its sha256 with the live file's; a mismatch
  aborts the sweep. The live tree is never written.
- A row whose test is red before any mutation is `BASELINE-RED` and does not count.
- A mutation that stops the test from compiling is `BROKEN` and does not count; no row was BROKEN in this run.
- The sweep needs `TMPDIR` writable for the linker; `/tmp` is read-only in this sandbox, so `composition.sh` defaults
  `TMPDIR` to `_tmp/tmp`. The uv cache and managed Python go under `_tmp/uv` for the same reason.

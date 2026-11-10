# COMPOSITION MUTATION SWEEP

Run with `bash scripts/mutate/composition.sh`; it takes about a minute on a warm target dir. Its self-test is
`bash scripts/mutate/tests/composition-sweep-test.sh`. Each row is one mutation, applied alone to a scratch copy of the
tree; only the row's named test file is run there. The table is `scripts/mutate/composition_mutations.py`.

## Latest run — spec 291 (2026-10-27)

Result: `MUTATION-SWEEP: required 18/18 probes 2`, exit 0. Every required row is RED. The three survivors of the first
run (m08, m09, m17) are killed by tests added or re-pointed in spec 291, not by renaming the mutations.

Spec 291 changed two things: m17's named test went from `spec_graph_runs_through_the_daemon` (BASELINE-RED on the live
tree) to `composition_failure_matrix`, whose M5 case already turns m17 red; and m08 and m09 gained their own cases.

## Survivors: none

One line per required row, with what turns it RED. Rows 01–07, 10–16 and 18 were RED in the first run and are unchanged
in this one; their line names the test file, not a re-measured assertion.

- **m01** — `call_inlines_a_graph`: the sha256 pin check (RED in both runs).
- **m02** — `call_inlines_a_graph`: the required `sha256` key and the pin check (RED in both runs).
- **m03** — `call_needs`: the host-id `needs` scan (RED in both runs).
- **m04** — `call_needs`: the declared-key check (RED in both runs).
- **m05** — `call_needs`: the empty-segment filter (RED in both runs).
- **m06** — `call_inlines_a_graph`: the depth limit, `depth_five_is_refused_and_depth_four_loads` (RED in both runs).
- **m07** — `call_inlines_a_graph`: the self-cycle check (RED in both runs).
- **m08** — `call_inlines_a_graph`, new `a_call_widening_the_host_is_refused_at_the_call_site`: a pure `transform`
  sub-graph and a call asking `proc, spawn` on a `proc` host is refused with `call 'docs' grants spawn`. The node-level
  check cannot see this case, so only the call-site check refuses it. The near-miss (`proc` alone) and the empty ask
  both load.
- **m09** — `call_records`, new `the_head_carries_the_record_fields`: after a load, `docs__in` carries the `call-from`
  mark with the record's graph, sha256, caps, exit and input keys, and no `needs` or `parent` on a top-level call.
  `the_head_carries_needs_when_the_subgraph_declares_them` and `a_nested_head_names_its_parent` pin the other two fields.
- **m10** — `run_journals_calls`: the journalled `graph.call` observation (RED in both runs).
- **m11** — `call_inlines_a_graph`: the namespaced node ids (RED in both runs).
- **m12** — `call_inlines_a_graph`: the call's `in` mapping (RED in both runs).
- **m13** — `call_inlines_a_graph`: the edge rewiring at the call site (RED in both runs).
- **m14** — `coder_charges_run_budget`: the coder's budget charge (RED in both runs).
- **m15** — `coder_charges_run_budget`: the start gate (RED in both runs).
- **m16** — `coder_session_per_node`: the per-node session path (RED in both runs).
- **m17** — `composition_failure_matrix`, case `m5_budget_exhausted_inside_a_called_graph`: with `[limits]
  max_micro_usd = 100000` the first coder's $0.40 exhausts the budget and the run ends `Failed`; with the budget built
  without `[limits]` the run succeeds and the status assertion fails. The first run's named test
  (`spec_graph_runs_through_the_daemon`) was BASELINE-RED on the live tree, so it could not judge the row.
- **m18** — `call_inlines_a_graph`: the has-call check in `check.rs` (RED in both runs).
- **m19** (probe) — `call_needs`: RED.
- **m20** (probe) — `call_needs`: RED.

## First run (spec 286, 2026-10-26)

On commit `3e91402` plus the uncommitted spec-286 files. Result: `MUTATION-SWEEP: required 15/18 probes 2`, exit 1. Three
required rows were not RED: m08 and m09 SURVIVED, and m17 was BASELINE-RED because `spec_graph_runs_through_the_daemon`
had a red `CONTROL_a_missing_docs_brief_refuses_before_any_agent` case on the live tree. A probe of m17 under
`spec_graph_spend_limit_bites` also survived. The survivors were recorded as: m08 is redundant with the node-level
check for `call_inlines_a_graph`; m09 is caught only by `run_journals_calls`; m17 needs a repaired control or a different
test. The first run's table is superseded by the table above.

## Method notes

- The scratch tree is a copy of the files `git ls-files -co --exclude-standard` lists (tracked and untracked, minus
  ignored), so the untracked daemon tests are included. A `git worktree` would have left them out.
- Each row restores the scratch file from the live file and compares its sha256 with the live file's; a mismatch
  aborts the sweep. The live tree is never written.
- A row whose test is red before any mutation is `BASELINE-RED` and does not count.
- A mutation that stops the test from compiling is `BROKEN` and does not count.
- The sweep needs `TMPDIR` writable for the linker; `/tmp` is read-only in this sandbox, so `composition.sh` defaults
  `TMPDIR` to `_tmp/tmp`. The uv cache and managed Python go under `_tmp/uv` for the same reason.
- `cargo` is not on the default `PATH` in this sandbox; it lives under `~/.cargo/bin`.

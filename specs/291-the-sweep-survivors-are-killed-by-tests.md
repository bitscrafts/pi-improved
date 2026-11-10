# Spec 291: the three composition mutants the sweep let live are killed by tests, not by renaming them

## Overview
**Project**: <repo>
**Phase**: production — composability, closing the sweep. Spec 286's first run killed 15 of 18 required mutations and recorded three survivors in `_docs/05-operations/COMPOSITION-MUTATION-SWEEP.md`. A survivor is a behaviour no test pins; this spec adds the tests, and the sweep — re-run — is the proof. The sweep exits nonzero until every required row is RED, so this spec is done when `bash scripts/mutate/composition.sh` exits 0.

The three, from the document and the code read 2026-10-26:
- **m08 — the call-site caps check** (`crates/pii-graph/src/fragment/call.rs:141`, `let widen = call_caps & !grant; if !widen.is_empty()`): a call whose own `caps` ask for more than the graph it sits in holds. The node-level check in `load_sub` (`call.rs:239`, `needs & !call_caps`) refuses the same widening **only when a sub-graph node needs the extra capability**, so the named test passes without the call-site check. The missing case: a call whose `caps` widen the host's grant and whose sub-graph has **no node that needs the extra capability** (a pure `transform` sub-graph).
- **m09 — the call record is not attached to the call head** (`call.rs:274`, `call_needs::mark(&mut added, std::slice::from_ref(own))`): `call_records.rs` asserts on the loader's record list, which is built separately from the head node's `call-from` mark; `run_journals_calls` catches the missing mark, so the sweep's named file does not hold it. The missing assertion: **after a load, the call's head node carries the `call-from` mark with the record's fields** (graph, sha256, caps, exit, `input_keys`, and `needs`/`parent` only when they apply, never null — `call_record::call_from_fields`).
- **m17 — the daemon path builds the budget without `[limits]`** (`crates/pii-server/src/engine/runs.rs:59-64`, `Budget::new(spec.limits.max_tokens, …, spec.limits.max_micro_usd, spec.limits.max_wall_ms)`): spec 284's tests drive the engine with a budget that is never exhausted, so replacing the limits with `u64::MAX` changes no assertion. The missing case: through `Engine::start_run`, a graph whose `[limits] max_micro_usd` is smaller than its first coder's reported cost **stops before the second coder** — a case spec 285's M5 already asserts for a scratch `[limits]`; the sweep's named file for m17 must be the file that holds it.

## Verified Premises
- **CONFIRMED** (read 2026-10-26) the three sites above, and that `crates/pii-server/tests/spec_graph_spend_limit_bites.rs` exists (the sweep's probe for m17 named it and the mutant survived it too).
- **CONFIRMED** the sweep's table is `scripts/mutate/composition_mutations.py` (rows `m08`, `m09`, `m17`: id, file, old, new, test file, kind); a row's **test file** is what the sweep runs for it.
- **MEASURE FIRST (1)** whether `composition_failure_matrix.rs` M5 (`m5_budget_exhausted_inside_a_called_graph`) already turns m17 red: run the sweep's row m17 with the **named test changed** to `composition_failure_matrix` before writing anything. If it does, the fix for m17 is the table row, not a new test — say so and change only the row.
- **MEASURE FIRST (2)** the exact refusal text of the call-site check (`graph.call.widen: call '<id>' grants <caps> the graph it sits in does not hold`) so the new case asserts the **call-site** message, not the node-level one (`graph.call.widen: node '<id>' of graph …`) — the two share a prefix, and a test that matches the prefix alone is the vacuous assertion this spec exists to remove.
- **NOT CLAIMED** that the other 15 rows are strong: a row that is RED under one mutation is not proof the behaviour is fully pinned. Spec 292 widens the table.

**Recorded 2026-10-26, not part of this spec.** The review of specs 288/289/290 found two more weak tests (a reversed `needs` scan and a reversed observation batch) and closed them with tests of their own, committed with those specs. A third, removing the post-run drain in `dispatch.rs`, is an **equivalent mutant**: every observation sends its wake before the node returns, so the watcher has already journalled it; it is defence in depth and the document lists it under that name rather than as a survivor.

## Requirements
### Functional
- **R1. m08.** New cases in `crates/pii-graph/tests/call_inlines_a_graph.rs` (or, if that file would pass 450 effective lines, a new `crates/pii-graph/tests/call_caps_at_the_call_site.rs` — measure first): (a) a host granting `["proc"]`, a call asking `["proc", "spawn"]`, a sub-graph of one `transform` node (no capability needed) → refused with the **call-site** message naming `spawn`; (b) the same call asking exactly `["proc"]` → loads (the near-miss); (c) a call asking `[]` → loads. The assertion matches `"call 'docs' grants"` (the call-site wording), never only `graph.call.widen`.
- **R2. m09.** New cases in `crates/pii-graph/tests/call_records.rs`: after `load_host_with_graphs`, the call head node (`<call>__in`) in the **loaded graph** carries `call-from` whose `graph`, `sha256`, `caps`, `exit` and `input_keys` equal the record's; `needs` is **absent** (not null) when the sub-graph declares none and present when it does; `parent` is absent for a top-level call and names the outer call for a nested one.
- **R3. m17.** Per MEASURE FIRST (1): if M5 already turns it red, change the sweep row's named test; otherwise add the case to `composition_failure_matrix.rs`'s sibling file `crates/pii-server/tests/engine_budget_from_limits.rs`: a one-coder host with `[limits] max_micro_usd = 100000`, a fake claude reporting `FAKE_COST = 0.40`, a second node after it → the run ends `Failed` and the second node never ran; CONTROL: the same graph with no `[limits]` completes.
- **R4. The proof.** Re-run the sweep: `bash scripts/mutate/composition.sh`. All 18 required rows RED, exit 0. Rewrite `_docs/05-operations/COMPOSITION-MUTATION-SWEEP.md` with the new result table (replace the "Survivors" section with "Survivors: none" and one line per row of what killed it), keeping the first run's result as a dated paragraph so the history stays.

### Non-Functional
- Tests and the sweep table only; no change under `crates/*/src`, `graphs/` or `scripts/autonomy/`. Every file under the 450-line gate.

## Exit Criteria
- [ ] `bash scripts/mutate/composition.sh 2>&1 | tail -1 | grep -qE "MUTATION-SWEEP: required 18/18"`
- [ ] `cargo test -q -p pii-graph --test call_records 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-graph --test call_inlines_a_graph 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `grep -rqE "call 'docs' grants" crates/pii-graph/tests`
- [ ] `grep -q 'Survivors: none' _docs/05-operations/COMPOSITION-MUTATION-SWEEP.md`
- [ ] `git diff --quiet -- crates/pii-graph/src crates/pii-server/src graphs`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `crates/pii-graph/tests/call_records.rs`
- `crates/pii-graph/tests/call_inlines_a_graph.rs`
- `crates/pii-graph/tests/call_caps_at_the_call_site.rs`
- `crates/pii-server/tests/engine_budget_from_limits.rs`
- `scripts/mutate/composition_mutations.py`
- `_docs/05-operations/COMPOSITION-MUTATION-SWEEP.md`

## Guardrails
- Do NOT edit source. Do NOT weaken, delete or `#[ignore]` an existing assertion. Do NOT remove a row from the sweep table or change a mutation to make it turn red (a row's **test file** may change only under R3's MEASURE FIRST (1), and the worker says so). Do NOT match only the prefix `graph.call.widen`. Do NOT commit.

## Follow-ups
Spec 292: widen the table (the 15 killed rows each under two mutations, one per behaviour a test names), and run the sweep from the queue behind `PII_SWEEP=1` so a regression in the composition code is a red row before it is a commit.

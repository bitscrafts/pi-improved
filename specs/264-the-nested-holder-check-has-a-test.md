# Spec 264: the nested-holder check of `needs` has a test that can fail

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composition (spec 261 follow-up, found 2026-10-24 by mutating spec 261's implementation: mutation m8 survived all 20 tests)

Spec 261 R8 makes nesting transitive: when a sub-graph `leaf` declares `needs = ["x"]` and the call to it maps no `x`, `leaf` reads its HOLDER's input, so the holder must declare `x` or map it.
`crates/pii-graph/src/fragment/call_needs.rs` enforces that in `check` (`if chain.len() > 1 && !passed.is_empty() {` ... `graph.call.undeclared ... which the holder does not declare`).

Measured by `/projects/_tmp/probe-holder-check.sh` on the built binary, host `-> mid -> leaf`, `mid` has NO `needs`, `leaf` has `needs = ["x"]` and reads `$.input.x`, no `in` anywhere:
- With the check: `graph.call.undeclared: graph 'mid' node 'r' reads input 'x' for call 'leaf', which the holder does not declare; holder declares only []`.
- With the check skipped (mutation m8): the graph LOADS and `leaf` reads `$.nodes.m__in.x`, which is `Null`. That is the silent-Null failure Opus ranked second among the risks.

The existing test `a_nested_need_must_be_declared_by_the_outer_graph` does not exercise it: it gives `mid` `needs = []`, so the OUTER call's own check (`undeclared` on `mid`'s expanded nodes) refuses first, and the holder check is never reached. The check is correct and unprotected.

## Verified Premises
- **CONFIRMED** (run) mutation m8 (`if false && chain.len() > 1 && !passed.is_empty() {`) leaves `call_needs` at `20 passed; 0 failed`; restored, 20 pass.
- **CONFIRMED** (run, probe) the refusal text above on the real tree, and the load success with Null on the mutated tree.
- **CONFIRMED** (read) the existing test builds `mid` with header `needs = []` and a `leaf` call without `in` (`crates/pii-graph/tests/call_needs.rs:336-355`).
- **CONFIRMED** (read) `call_needs::parse(host, holder)` returns `None` for a holder without `needs`, and `unwrap_or_default()` makes that an empty list, so a holder with no declaration refuses a nested declared-but-unmapped key.

## Requirements
- **R1.** New file `crates/pii-graph/tests/call_needs_holder.rs` (do NOT edit `call_needs.rs`; copy the helpers it needs). It adds the tests below. No source file changes.
- **R2.** Each test builds the temp-dir graph set, loads it through the same loader the existing tests use, and (for the load-success cases) RUNS the graph.

## TDD Contract
| Test name | Setup | Expected |
|---|---|---|
| `a_holder_without_needs_refuses_a_nested_declared_key` | host -> `mid` (NO `needs` key at all) -> `leaf` (`needs=["x"]`, node reads `$.input.x`, call maps nothing) | load error contains `graph.call.undeclared`, `mid`, `leaf` and `x` |
| `a_holder_that_maps_the_key_loads_and_reads_it` | same, but `mid`'s call to `leaf` carries `in.x = "mapped"` | loads; run output `"mapped"` (CONTROL: the refusal is about the missing declaration, not about nesting) |
| `a_holder_that_declares_the_key_loads_and_reads_the_host_input` | same, `mid` has `needs=["x"]`, host input `{x: 7}` | run output `7` (CONTROL) |
| `the_error_is_not_the_outer_checks` | `mid` has NO `needs` | the message contains `which the holder does not declare` (the holder check's own wording, so the outer check cannot satisfy it) |
| `three_levels_need_every_holder_to_declare` | host -> `a` (`needs=["x"]`) -> `b` (no `needs`) -> `leaf` (`needs=["x"]`) | refused naming `b` (the middle holder that does not declare) |

**The requirement a plausible implementation satisfies only in appearance:** checking only the outer call's `needs` (what `needs = []` exercises). `a_holder_without_needs_refuses_a_nested_declared_key` and `the_error_is_not_the_outer_checks` are the pair that catch it; the holder must have no `needs` key at all.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test call_needs_holder 2>&1 | grep -qE "^test result: ok\. [5-9] passed"`
- [ ] `cargo test -q -p pii-graph --test call_needs 2>&1 | grep -qE "^test result: ok\. 20 passed"`
- [ ] `git diff --quiet -- crates/pii-graph/src crates/pii-graph/tests/call_needs.rs crates/pii-graph/tests/call_inlines_a_graph.rs`
- [ ] `cargo clippy -q -p pii-graph --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit any file under `crates/pii-graph/src` or any existing test: this spec adds a test file only.
- Do NOT commit.

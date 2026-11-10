# Spec 256: the call's caps ceiling against its host has a test that can fail

## Overview
**Project**: <repo>
**Phase**: production — composition (spec 255 follow-up found by its independent verification, 2026-10-24)

Spec 255 R6 has two caps rules: every inlined node's caps fit inside the call node's `caps`, and the call node's `caps` fit inside the host graph's. The code has both
(`crates/pii-graph/src/fragment/call.rs`: the `widen = call_caps & !grant` check, and the per-node check in `load_sub`). The test file has one caps test,
`a_subgraph_cannot_widen_caps`, and it trips the PER-NODE rule. Measured: with the call-versus-host check disabled (`if false && !widen.is_empty()`), all 19 tests of
`call_inlines_a_graph` still pass. The rule that stops a call from granting a sub-graph more than the host holds is untested.

## Requirements
- **R1.** New test file `crates/pii-graph/tests/call_caps_ceiling_against_the_host.rs` (do NOT edit `call_inlines_a_graph.rs`). Use the same temp-dir approach as that file; copy the few helpers you need.
- **R2.** `a_call_cannot_grant_more_than_its_host_holds`: host graph `caps = ["proc"]`, a `call` node with `caps = ["proc", "spawn"]`, a sub-graph whose nodes need only `proc`. The load is refused and the error names `spawn` and the call node's id. (The sub-graph needs only `proc` on purpose: the per-node rule must NOT be what refuses it.)
- **R3.** `a_call_within_its_host_grant_loads`: the same shape with the host holding `["proc", "spawn"]`. It loads. (CONTROL for R2.)
- **R4.** `the_call_ceiling_does_not_depend_on_the_subgraph`: the same refusal when the sub-graph declares no caps at all. (Proves the check reads the call node, not the sub-graph.)

## TDD Contract
| Test name | Expected |
|---|---|
| `a_call_cannot_grant_more_than_its_host_holds` | error contains `spawn` and the call id |
| `a_call_within_its_host_grant_loads` | loads |
| `the_call_ceiling_does_not_depend_on_the_subgraph` | error contains `spawn` |

**The requirement a plausible implementation satisfies only in appearance:** a test that refuses for the wrong reason. If the sub-graph's node needs `spawn`, the per-node rule refuses and the test passes with the call-level check removed. R2's sub-graph must need only `proc`.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test call_caps_ceiling_against_the_host 2>&1 | grep -qE "^test result: ok\. [3-9] passed"`
- [ ] `cargo test -q -p pii-graph --test call_inlines_a_graph 2>&1 | grep -qE "^test result: ok\. 19 passed"`
- [ ] `git diff --quiet -- crates/pii-graph/tests/call_inlines_a_graph.rs crates/pii-graph/src`
- [ ] `cargo clippy -q -p pii-graph --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit any file under `crates/pii-graph/src`, and do NOT edit `call_inlines_a_graph.rs`. This spec adds a test file only.
- Do NOT commit.

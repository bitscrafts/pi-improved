# Spec: the store's failure path is reachable, with an induced failure

## Overview

`crates/pii-store/src/lib.rs` swaps an operation/resource at three places (`:96`, `:103`, `:357` around the
store-op helper) whose failure path no test can currently reach: the store either opens or the test never
starts. The consequence is a branch that is written, compiled, and never exercised — the reader cannot tell
whether it behaves as promised, and a defect there would be invisible. This is the item the open-items
register carries as `g4`, and the reason it was parked is honest: there is no test-only way to induce the
failure.

**Project**: /projects/pi-improved
**Phase**: post-vision hardening (open-items g4)
**Source**: `/projects/pi-improved/_docs/05-operations/open-items.md` (g4); the reviewer's coverage findings
(`_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §2.2), which measured line coverage 81.30%

## Requirements

### Functional
- **R1** Read the three sites first and report what each swap is (op → resource, resource → op) and what
  failure it guards. If the shape differs from this premise, **STOP and report** — do not force the change.
- **R2** Introduce the smallest **test-only** seam that makes the failure reachable (a trait with an
  implementation that fails on demand, a `#[cfg(test)]` failpoint, or an injectable closure). The seam must
  be **production-inert**: no behaviour change, no runtime cost in a release build, and no new public API
  that a caller could misuse. State which mechanism was chosen and why in the report.
- **R3** A test induces the failure and asserts the **caller-visible** outcome: the error variant/message a
  caller receives, and that the store is left in the state the code claims (e.g. the swap not applied, or
  applied atomically — whichever the code promises). Assert what the live path reads, not a helper's
  intermediate value (a value asserted where nobody reads it is a false green).
- **R4** The induced-failure test must be **RED without the seam** (i.e. the seam is what makes it possible)
  and green with it. If the failure path turns out to be unreachable by design (the branch cannot happen),
  say so with the reasoning and propose removing the branch instead — a branch that cannot happen is dead
  code, and this project audits for dead code.
- **R5** No existing test is weakened and the store's real behaviour under normal operation is unchanged:
  the existing `pii-store` suite stays green without modification.

### Non-Functional
- No new dependency. clippy `-D warnings` clean. `#![forbid(unsafe_code)]` untouched.
- The seam must not require running a test binary twice, and must not slow the normal path measurably.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `a_failed_op_swap_reports_what_the_caller_reads` (new) | the seam induced to fail | the documented error variant/message, and the promised store state |
| the same test with the seam absent (scratch copy) | — | cannot be written / RED — evidence that the seam is what makes the path reachable |
| the existing `pii-store` suite | unchanged | still green |

## Exit Criteria

- [ ] `cargo test -p pii-store 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -1 | grep -qv 'error'`
- [ ] the seam is production-inert: `grep -rn 'cfg(test)\|debug_assert' crates/pii-store/src/lib.rs | head -3` is non-empty
- [ ] `cargo run -q -p xtask -- deps-check 2>&1 | tail -1 | grep -qv 'FAIL'`

## Guardrails

- Do NOT make a production code path depend on a test-only switch (a runtime `if cfg!(test)` inside the hot
  path is a defect): the seam is compile-time or an injected trait object chosen by the caller.
- Do NOT add an allowlist entry or an exemption to silence a check that this change reddens.
- If the failure path proves unreachable, do NOT invent a failure to justify the branch: report it as dead
  code with the evidence and let the orchestrator decide.
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit; scratch under `/projects/_tmp/`.

## Closure

**Closed as already covered — the premise was false, and the worker stopped instead of forcing it.**

The item came from open-items `g4`, which cited `crates/pii-store/src/lib.rs:96,103,357` as op/resource swaps
whose failure path no test could reach. Measured on 2026-10-13 (the worker's own report, `SPEC-DONE 140
criteria=0/4`, no commands run, no code touched):

- **Those line numbers are doc and comment lines** — stale references, not code.
- **There are no op/resource swaps with their own failure branch.** The real sites are `PiiError::store_op(op,
  resource, e)` calls, and the failure is already induced and asserted by two tests from an earlier spec:
  `open_failure_renders_structured_store_op` (opens a directory as the database; asserts `store op open on
  <path>: … Is a directory (os error 21)`) and `open_table_failures_render_op_resource_and_detail` (a table with
  the wrong type; asserts `store op open on <name>: <name> is of type Table<u8, u8>`). Both assert the full
  caller-visible message, which is exactly what this spec was written to create.
- **The residual, recorded rather than specced**: the `begin_write`/`"db"` sites (`lib.rs:115, 205, 484, 531`,
  plus sites in `heal.rs`, `identity.rs`, `session.rs`) remain unreachable, because redb's `begin_write` only
  errors when the database is poisoned. Reaching them would need another injection flag, and there is no
  user-visible behaviour to assert when it fires — a test whose purpose cannot be stated is not worth a seam.
  If one of those sites ever gains a caller-visible failure, it becomes its own item, named by line.
- **Lesson recorded in the plan**: this row's verify command (`cargo test -p pii-store … ok`) passed on an
  unchanged tree, so a no-op satisfied it — a criterion no change can fail is not a criterion. The item's real
  acceptance is the two named tests above, which existed before this spec.

**Outcome**: nothing to build. `g4` is closed as covered; the unreachable `begin_write` sites are a documented,
deliberate residual.

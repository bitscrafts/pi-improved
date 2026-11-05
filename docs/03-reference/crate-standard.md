# Crate standard

Every crate under `crates/` is held to the same seven points. This page states what each point measures and the
check behind it. A point with no check says so: a standard with unmeasured clauses is a wish list, and this one
names the wish list.

## The points

| Point | What is measured | Check | Status |
|---|---|---|---|
| File size | Files over 450 physical lines per crate (`src/` and `tests/`, `target/` skipped) | `cargo xtask crate-audit`; ratchet via `crate-audit --ratchet` in `gate-steps.txt` | Measured and gated. The ratchet refuses growth against `_docs/05-operations/crate-audit-baseline.tsv`. |
| File size, touched files | Effective lines (non-blank, non-comment, outside `#[cfg(test)]`) of `src/` files | `cargo xtask lint-files` in `gate-steps.txt` | Measured and gated, for the files a change touches only. |
| Tests exist | `#[test]` count per crate, and test files per crate | `cargo xtask crate-audit` (the `tests` and `#[test]` columns) | Counted and printed. Not gated: a crate with no tests is shown, not refused. |
| Tests can fail | Whether a test would catch a broken behaviour | not measured yet | Not measured yet. |
| No duplicated module logic | Two modules holding the same logic | not measured yet | Not measured yet. |
| Minimal public surface | How much of the crate is `pub` beyond its own crate root | partial: workspace lint `unreachable_pub = "deny"` (compile time, `clippy` step) rejects `pub` items no caller can reach | Partial. The size of the surface is not measured. |
| Observability through the seam | Every crate uses the `pii-observe` contract, and raw prints stay under budget | `cargo xtask lint-observability` in `gate-steps.txt` | Measured and gated. |
| No dead module | A module nothing declares or calls | partial: `cargo xtask reachability-check` (declared capabilities), `silent-noop-check` (config keys, events, RPCs with no reader or producer) | Partial. A module with no caller is not measured as such. |
| No `unwrap` on a runtime path | `unwrap()` and `expect()` outside tests | workspace lints `unwrap_used` and `expect_used` set to deny, enforced by the `clippy` step | Measured and gated. Test code opts out with a reason on the `allow`. |

## Reading the file-size numbers

- The audit counts **physical** lines, not effective lines. `lint-files` counts effective lines over `src/` only. The
  two disagree on a comment-heavy file: a file can pass `lint-files` and still be counted by the audit. That gap is
  deliberate; the audit is the wider measurement.
- Over-budget files are **debt**. The baseline records each crate's current count, and a crate's count may only fall.
  It reaches zero, then stays there.
- The baseline is written by `cargo xtask crate-audit --update-baseline`. Once it exists, that flag refuses any raise.
  The gate never writes it.
- The budget is the same for every crate. Raising it for one file is not an option: a file over the budget needs its
  module boundary found, not more room.

## Where the debt is paid

The audit's output is the row generator. Each over-budget file is one row, worst crate first, and a row names the file
it clears. A refactor row states what it does not change, and its proof is the crate's existing tests, unchanged in
count or increased. A split that deletes tests is a regression.

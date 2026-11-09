# Crate standard

Every crate under `crates/` is held to the same seven points. This page states what each point measures and the
check behind it. A point with no check says so: a standard with unmeasured clauses is a wish list, and this one
names the wish list.

## The points

| Point | What is measured | Check | Status |
|---|---|---|---|
| File size | Files over 450 physical lines (`wc -l`) under `crates/` (`src/` and `tests/`, `target/` skipped) | `cargo xtask lint-files` and `crate-audit --ratchet`, both first in `gate-steps.txt`; one counter (`file_size::physical_lines`) | Measured and gated per file. A file not listed may not exceed 450; a listed file may not grow; a listed file that is at or under 450, or gone, must leave the baseline. |
| File size headroom | Files within 30 lines of the limit | `cargo xtask lint-files --report` (headroom section) | Reported. New code must not go into these files. |
| Tests exist | `#[test]` count per crate, and test files per crate | `cargo xtask crate-audit` (the `tests` and `#[test]` columns) | Counted and printed. Not gated: a crate with no tests is shown, not refused. |
| Tests can fail | Whether a test would catch a broken behaviour | not measured yet | Not measured yet. |
| No duplicated module logic | Two modules holding the same logic | not measured yet | Not measured yet. |
| Minimal public surface | How much of the crate is `pub` beyond its own crate root | partial: workspace lint `unreachable_pub = "deny"` (compile time, `clippy` step) rejects `pub` items no caller can reach | Partial. The size of the surface is not measured. |
| Observability through the seam | Every crate uses the `pii-observe` contract, and raw prints stay under budget | `cargo xtask lint-observability` in `gate-steps.txt` | Measured and gated. |
| No dead module | A module nothing declares or calls | partial: `cargo xtask reachability-check` (declared capabilities), `silent-noop-check` (config keys, events, RPCs with no reader or producer) | Partial. A module with no caller is not measured as such. |
| No `unwrap` on a runtime path | `unwrap()` and `expect()` outside tests | workspace lints `unwrap_used` and `expect_used` set to deny, enforced by the `clippy` step | Measured and gated. Test code opts out with a reason on the `allow`. |

## Reading the file-size numbers

- One counter, physical lines (`wc -l`). The effective count (non-blank, non-comment) is a `--report` column only: a
  comment-heavy file is judged by its physical length like any other.
- Over-budget files are **debt**. The baseline (`path<TAB>lines`) records each over-budget file by path and its count.
  A file's count may only fall, and a second file may not take the place of a listed one.
- The baseline is written once by `crate-audit --migrate-from-crate-counts` (spec 402), and afterwards by
  `cargo xtask crate-audit --update-baseline`, which refuses any new path or raised count. The gate never writes it.
- The budget is the same for every file. Raising it for one file is not an option: a file over the budget needs its
  module boundary found, not more room.

## Where the debt is paid

The audit's output is the row generator. Each over-budget file is one row, worst crate first, and a row names the file
it clears. A refactor row states what it does not change, and its proof is the crate's existing tests, unchanged in
count or increased. A split that deletes tests is a regression.

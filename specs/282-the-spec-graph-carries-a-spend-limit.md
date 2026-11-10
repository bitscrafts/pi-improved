# Spec 282: the shipped spec graph carries a spend limit

## Overview
**Project**: <repo>
**Phase**: production — first real composed run. Spec 276 made a graph's `[limits] max_micro_usd` real for the node that spends the money. Neither tracked graph declares one (`graphs/spec.toml`, `graphs/docs.toml`), so the run's budget is `u64::MAX` and nothing bounds a real run of the composed graph — the last thing standing between this queue and an unattended run of `pii run start spec`.

The figure is a policy number, and this spec fixes it with its derivation rather than leaving it open: **1 000 000 micro-USD (1.00 USD) per run**. Measured basis: one full spec implementation through the queue's own daemon — claude/haiku, 55 turns, 647 s — journalled `claude_cost_usd = 0.070610` (2026-10-26, run `r000000000130`). One US dollar is about fourteen such passes in ONE run, which covers `code` + `docs` + `reverify` and several fix iterations, while making a runaway (a loop that keeps re-running the coder) stop after roughly fourteen instead of never. It is one integer in one file; changing it is a one-line commit, and this spec says so.

## Verified Premises
- **CONFIRMED** (read 2026-10-26) `graphs/spec.toml` and `graphs/docs.toml` carry no `[limits]` table; `pii-graph/src/spec.rs:122` defaults `max_micro_usd` to `u64::MAX`; `runs.rs:59` builds the run's `Ctx` budget from those limits, and the coder charges it (spec 276, `coder/budget.rs`).
- **CONFIRMED** a `call` node is expanded INLINE, so a called graph shares the host run's `Ctx` and budget: `graphs/docs.toml` cannot have a budget of its own, and a `[limits]` written there would be ignored.
- **CONFIRMED** the measured cost above comes from the refreshed daemon's journal (`claude_cost_usd` is journalled since spec 263's head+tail capture), not from an estimate.
- **NOT CLAIMED**: that 1.00 USD is the right number for every operator. It is the default this repo ships, chosen so a runaway stops and a real run completes; the spec's test pins the VALUE so that changing it is deliberate and visible.

## Requirements
### Functional
- **R1. The limit, where the run spends.** `graphs/spec.toml` gains a `[limits]` table with `max_micro_usd = 1000000`, above every node table, with a comment giving the measured basis and saying that a `call` shares this budget. Nothing else in the file changes: no node, edge, `entry`, `caps`, value or comment changes, and `docs.toml` gains nothing.
- **R2. Tests, with controls.** New `crates/pii-graph/tests/spec_graph_has_a_spend_limit.rs`: (a) the tracked `graphs/spec.toml`, loaded through `load_host_with_graphs` with `graphs/` as the graph dir (the shape `spec_graph_is_tracked_and_valid.rs` uses), has `spec.limits.max_micro_usd == 1_000_000`; (b) the same load shows the value is NOT `u64::MAX` (the default a graph without `[limits]` gets) — this is the assertion that fails today; (c) CONTROL: the CALLED graph, loaded on its own, still has `u64::MAX`, and a comment in the test records why that is correct (an inline call shares the host's context, so a limit there would be ignored); (d) CONTROL: `docs.toml` is still byte-identical to the digest pinned in `spec.toml` (recompute `sha256_hex`), so this change did not silently edit the called graph; (e) a run-level proof that the limit bites, using the fake-agent helpers of `crates/pii-server/tests/coder_charges_run_budget.rs`: the tracked graph's limits are handed to `Budget::new` and the coder's `cap_usd` yields `1.0` USD for the FIRST coder — read from the argv the fake agent recorded (`--max-budget-usd 1`), which is the live path and not the config text.
  The exit criteria below include a `grep` for the value, but (a)/(b)/(e) are the real checks.

### Non-Functional
- One line of TOML plus a test. No Rust source change. Every file under the 450-line gate.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test spec_graph_has_a_spend_limit 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `grep -q 'max_micro_usd = 1000000' graphs/spec.toml`
- [ ] `cargo test -q -p pii-graph --test spec_graph_is_tracked_and_valid 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-graph --test spec_graph_has_a_verdict 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test spec_graph_documents_after_verify 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `git diff --quiet -- crates/pii-graph/src crates/pii-server/src`
- [ ] `test "$(git diff --numstat -- graphs/spec.toml | cut -f1 | grep -E '^[0-9]+$' || echo 0)" -le 6`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit `graphs/docs.toml` (its digest is pinned in `spec.toml`). Do NOT change any node, edge or value in `graphs/spec.toml` beyond adding `[limits]`. Do NOT touch Rust. Do NOT commit.
- If `spec_graph_documents_after_verify` stops passing, the limit broke the composed run: stop and report rather than loosening either test.

## Follow-ups (not this spec)
1. `pii run start spec` driven by the queue's worker instead of `run-batchC-pii.sh` (the lane's shell graph would go).
2. A per-node budget share, so `docs` cannot spend the whole run's allowance before `reverify`.

## Closure

Closed by the orchestrator 2026-10-26, on the tree the worker left (the item had been parked `gate-failed` for 5 h 15 min).

**The eight Exit Criteria, run again by the orchestrator on that tree: 8 of 8 green.** `spec_graph_has_a_spend_limit` `ok. 4 passed`; the value grep; `spec_graph_is_tracked_and_valid`; `spec_graph_has_a_verdict`; `spec_graph_documents_after_verify`; no Rust diff; `graphs/spec.toml` `3 0` (≤ 6 added lines); `lint-files` PASS. The run-level proof R2(e) is `crates/pii-server/tests/spec_graph_spend_limit_bites.rs`, `ok. 1 passed` — it reads `--max-budget-usd 1` from the argv the fake agent recorded.

**The test can fail (mutation, then restored byte-identical).** Deleting the `[limits]` block from `graphs/spec.toml` turned `spec_graph_has_a_spend_limit` red — `2 passed; 2 failed`, and the two that failed are exactly assertions (a) and (b), while both controls stayed green (the called `docs.toml` is still `u64::MAX`; its pinned digest still matches). Restored with `cp` + `touch` (so the mtime is newer than the mutated build — a `cp -p` restore would have re-used the stale binary and produced a false red), `sha256 495ed509e15f59f2cefc10b8c77d4ea184b99414c27328a9705f24cc3529ecc3` before and after, then `ok. 4 passed`.

**The gate, on the door that measures coverage.** `cargo xtask fresh-gate --worktree` → `quality-gate: ALL PASS` (13 steps; it SKIPS `coverage-floor` by design, an export cannot fit ~18 G). The repo gate, run exactly as `cycle.sh` runs it (`QG_SKIP_COVERAGE=0`, `cargo llvm-cov --workspace --lib --bins --tests`) → `coverage-floor: total 82.92% >= floor 74.00%`, `quality-gate: ALL PASS`, rc 0.

**The first gate red was not this spec, and is not a regression here.** Cycle `20261026T005407Z` failed `coverage-floor` on `pii-server --test latency`, an unrelated 50 ms wall-clock assertion (`session.prompt` must return before the run finishes), under `cargo llvm-cov --workspace` with 177 test binaries in parallel — the same test binary then passed in the worktree gate, in the repo gate's test step and in the repo gate's coverage run. Not retuned and not weakened. It has never reproduced outside a full coverage run; if it does, it earns its own row rather than a retry.

**The row was wrong, and that is why this sat.** A146's commit-path column named two of the three files R2 requires — `crates/pii-server/tests/spec_graph_spend_limit_bites.rs` was missing, so the cycle would have staged an incomplete path list even with a green gate. The row now names all three files, and its `verify` column was diffed against this spec's `## Exit Criteria` **programmatically**: 8 criteria, 8 commands, identical.


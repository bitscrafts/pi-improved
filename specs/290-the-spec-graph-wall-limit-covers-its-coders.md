# Spec 290: the shipped spec graph's wall limit covers the coders it runs

## Overview
**Project**: <repo>
**Phase**: production — composability, found while reviewing the first real runs through the daemon. `graphs/spec.toml` declares `[limits] max_micro_usd = 1000000` (spec 282) and **nothing else**. The loader fills every absent limit with its default, and `Limits::default().max_wall_ms` is **600 000 ms (10 minutes)** (`crates/pii-graph/src/spec.rs:120`). The engine checks it **before dispatching each next node** (`crates/pii-graph/src/run/engine.rs:72`): a run whose first node took more than ten minutes ends `Failed` with `max_wall_ms exceeded`, whatever that node returned.

The `spec` graph's `code` node allows `timeout_ms = 3 000 000` (50 minutes) and the called `docs` graph's coder another `1 200 000`. Measured on the daemon 2026-10-26, a real pass takes 382 s (`r000000000259`, succeeded), 601 s (`r000000000261`) and 717 s (`r000000000260`) — the last two **failed with the agent's work complete and its exit 0**. The queue lane's generated graph had the same gap and was fixed (its wall limit is now the node timeout plus five minutes). The shipped graph was not: **the first real run of the composed graph would fail at the `verify` step whenever the coder takes longer than ten minutes**, which is the usual case. Spec 284's tests could not see it: the fake agents finish in milliseconds.

## Verified Premises
- **CONFIRMED** (read 2026-10-26) `graphs/spec.toml:11-12` has `[limits]` with `max_micro_usd` only; `graphs/docs.toml` has no `[limits]`; `toml_load.rs:85-110` sets only the fields present, so `max_wall_ms` stays 600 000; `engine.rs:72` compares `elapsed_ms() > max_wall_ms` once per tick.
- **CONFIRMED** a `call` is inlined, so the host's `[limits]` govern the called graph's nodes (spec 282): a limit written in `docs.toml` would be ignored in a composed run, so the figure belongs in `spec.toml`.
- **CONFIRMED** node timeouts: `code` 3 000 000 ms (`spec.toml:25`), `docs` coder 1 200 000 ms (`docs.toml:20`); `verify` and `reverify` are judges whose criteria are shell commands (cargo tests, measured at minutes each — the A148 criteria ran 5–10 minutes on a cold build).
- **MEASURE FIRST** how long the judge nodes may run: read `crates/pii-server/src/node_kinds/nodes/queue/judge*.rs` for any per-criterion timeout; if there is none, the figure for judges is a stated allowance, not a measurement, and the spec says so.
- **NOT CLAIMED** that the sum of node timeouts is the right wall limit for every operator: it is the **smallest** value at which the graph's own per-node limits, not the run's default, decide when a node is too slow.

## Requirements
### Functional
- **R1. The limit.** `graphs/spec.toml`'s `[limits]` gains `max_wall_ms` equal to: the `code` node's `timeout_ms`, plus the called graph's coder `timeout_ms`, plus **1 800 000** (30 minutes for the two judges and the call's own steps) — **6 000 000** with today's numbers — with a comment stating the derivation and the measured failure. Nothing else in the file changes: no node, edge, value, `max_micro_usd`, or comment other than the one added. `graphs/docs.toml` is not edited (its digest is pinned in `spec.toml`).
- **R2. The test derives, it does not copy.** New `crates/pii-graph/tests/spec_graph_wall_covers_its_nodes.rs`: (a) load the tracked `spec` graph through `load_host_with_graphs` (the shape `spec_graph_is_tracked_and_valid.rs` uses) and assert `limits.max_wall_ms >=` the **sum of every `coder` node's `timeout_ms` in the loaded, expanded graph** (read from the loaded nodes — including the inlined `docs__…` coder — not hard-coded); (b) assert it is **not** 600 000 (the default); (c) CONTROL: a scratch copy of `spec.toml` with the `max_wall_ms` line removed fails (a) and (b) — proven by running the same assertion function on the modified text; (d) CONTROL: a scratch copy whose `max_wall_ms` is `sum - 1` fails (a), and one equal to `sum` passes — the boundary; (e) CONTROL: the digest of `docs.toml` still equals the pin (the file was not edited).
- **R3. The same property for the other tracked graphs that run a coder alone.** **AMENDED 2026-10-26** — the first version told the worker not to edit them, and its own test (`every_tracked_coder_graph_covers_its_coders`) then found four graphs with the same defect: `coder-demo.toml`, `coder-podman.toml`, `coder-selffix.toml` and `queue.toml` each run a coder with `timeout_ms = 1800000` and declare no `max_wall_ms`, so the 600 000 ms default fails any run whose coder passes ten minutes (these are the graphs `scripts/archive/deploy-the host-test.sh`, `scripts/autonomy/deploy-row.sh` and `modules/devbox/verify-spec.sh` run). Each gets a `[limits]` table with `max_wall_ms` = the sum of its coder timeouts + 300 000 (**2 100 000** for the three single-coder graphs) and, for `queue.toml`, which also has a judge, + 1 800 000 (**3 600 000**), with a comment naming the measurement. Nothing else in those files changes. The test stays as the worker wrote it and turns green. `docs.toml` stays the one allowed exception (it is only called, and a called graph runs under its host's limits).

### Non-Functional
- One new TOML key and one new test file. No Rust under `crates/*/src`. The test file stays under the 450-line gate.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test spec_graph_wall_covers_its_nodes 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test "$(cargo test -q -p pii-graph --test spec_graph_wall_covers_its_nodes -- --list 2>/dev/null | grep -c ': test$')" -ge 5`
- [ ] `grep -qE '^max_wall_ms = [0-9]+' graphs/spec.toml`
- [ ] `cargo test -q -p pii-graph --test spec_graph_is_tracked_and_valid 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-graph --test spec_graph_has_a_verdict 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test spec_graph_runs_through_the_daemon 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `git diff --quiet -- graphs/docs.toml crates/pii-graph/src crates/pii-server/src`
- [ ] `test "$(git diff --numstat -- graphs/spec.toml | cut -f1 | grep -E '^[0-9]+$' || echo 0)" -le 8`
- [ ] `test "$(grep -lE '^max_wall_ms = [0-9]+' graphs/coder-demo.toml graphs/coder-podman.toml graphs/coder-selffix.toml graphs/queue.toml | wc -l)" -eq 4`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `graphs/spec.toml`
- `graphs/coder-demo.toml`
- `graphs/coder-podman.toml`
- `graphs/coder-selffix.toml`
- `graphs/queue.toml`
- `crates/pii-graph/tests/spec_graph_wall_covers_its_nodes.rs`

## Guardrails
- Do NOT edit `graphs/docs.toml` (its digest is pinned), any graph other than `spec.toml` and the four named in R3, or any source. Do NOT lower or remove an assertion. Do NOT pick the number by trying values until a test passes: it is the derivation above. Do NOT commit.

## Follow-ups
A graph-level `pii graph check` warning when a graph's wall limit is below the sum of its coder timeouts, so the next graph that spends money cannot ship with this gap.

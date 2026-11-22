# DEFECT 2026-11-22 — a refusal that named the wrong cause, and the six reds behind it

One line of graph text cost most of a day. This document records what it hid, how each layer was
found, and the rules that came out of it, so the next person does not pay again.

## The sentence

`graphs/sdd.toml` had one `refuse` node fed by two different `contract` ports — `fail` (a criterion ran
and did not pass) and `unrunnable` (a criterion could not run at all) — with the reason
`"the agent failed or the criteria could not run"`. The graph knew which had happened; the node threw
it away. Every refusal read like a broken agent, and five hypotheses in one night died on that
sentence: CLI auth, PATH, daemon, container, disk. All five were measured and all five were innocent.

The split (`graphs/sdd.toml`, plus `graphs/spec.toml` routing both arms) gives each cause its own
sentence. The pins are a chain — `spec` → `queue` → `self-improve`/`night` — so they are re-fixed to a
fixed point, never level by level.

## What the first honest refusal then said

```
reason=the agent failed: refused_unrunnable: the criteria could not run at all, so nothing was verified
```

Nine criteria across specs 431, 435 and 436 invoked files the rows themselves create
(`graphs/row-verdict.toml`, `scripts/checks/row-verdict-cites.sh`, `tests/row_verdict.rs`). A command
that names a missing file exits 127 — unrunnable, not RED — and the contract judge is right to refuse
to authorise work it cannot measure. The rows could never start.

**Rule:** a criterion may only name a path that exists today, or it must fail first:
`test -f X && <command>` exits 1 now (RED, measurable) and 0 when the work is done. Not running is not
the same as measuring and finding nothing.

## The six reds that were not the defect

| # | What the log accused | What was true |
|---|---|---|
| 1 | "no runtime" for 800 aborts | `pii-devbox` was stopped on the host (`mvcorrea@lnx`, not root) |
| 2 | the lane could not reach its socket | `XDG_RUNTIME_DIR` was unset in cron; the socket path was invented |
| 3 | `memory_learn` failed under the gate | a daemon the operator had started held `channel.redb` |
| 4 | tests failed while a graph was being edited | `run-batchC-pii.sh` refuses a graph that differs from the repo's HEAD — correctly |
| 5 | `pii graph lint` rejected a valid graph | `/root/.local/bin/pii` was from 14 Nov, before the new node kinds existed |
| 6 | `test (touched crates)` found no tests | the step demanded `--lib` of `pii-cli`, which is a binary crate |

Each one is the same shape: **the measurement was of something other than the thing being certified.**

## Rules earned, and where they are enforced

- **A test that drives the real shell owns the HEAD it runs against.** Spec 441 gave the shell tests a
  scratch git repo; spec 442 extended it to `crates/*/tests` and to `lane-pin-head`'s clean case.
  `scripts/checks/graph-edits-are-testable.sh` fails and names any test that starts the lane without
  one, and fails when it inspects nothing.
- **A called graph's interface is its terminal node names.** Splitting `refused` removed a port from
  every caller; `spec.toml` maps both new arms.
- **A vocabulary kind must serve more than one graph.** `xtask lint-graph-kinds` refuses a kind used by
  a single graph ("specialised"). Row 431's two new kinds fail this fence, and its own criteria could
  not see it: `pii graph lint` validates one graph, `lint-graph-kinds` validates the set.
- **The gate for an item is the quick gate.** `scripts/autonomy/env.sh` has said so since October
  (`GATE_STEPS=gate-steps-quick.txt`, `QG_SKIP_COVERAGE=1`): coverage belongs to a release. The full
  gate costs ~50 GB and ~45 min; the quick one costs ~10 GB and ~2 min.
- **The binary that runs the criteria must be the build.** `cargo install` puts it in
  `/usr/local/cargo/bin` while `PATH` resolves `/root/.local/bin` first; and a running daemon keeps the
  old file (`Text file busy`). Stop the daemon, install with `--force` or to `--root /root/.local`.
- **A cleanup must never remove what makes the next operation fit.** `_tmp/gate-target` (the warm gate
  cache) is why a gate costs 37 GB and not 96. `target/llvm-cov-target` is safe to remove.
- **`blocked` must be a lease, not a life sentence.** Each failed attempt re-blocks a row forever; the
  operator cleared it by hand six times in one day. Open.

## What is not done

- **431** — the work (node kinds, graph, test, check) is preserved in `_tmp/_trash/431-wip/`. It fails
  the vocabulary fence. Either the kinds become general (a second graph uses them), or the logic runs
  through existing kinds, or the row is re-scoped to the check script alone. Decide before coding.
- **440** — withdrawn with `_tmp/_trash/440-wip/REWORK.md`: it asks the local machine and dispatches to
  the remote, and compares an image name with an image id.
- **The `blocked` lease**, the `sdd` → `tdd` call (spec 373's criterion is still red), and the
  contradictory `"the agent failed"` prefix at `graphs/spec.toml` (now redundant with the arm reason).
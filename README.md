# pi-improved

A Rust workspace for an always-on agent substrate: a terminal coding agent, a personal-assistant daemon, a
reactive IoT/automation engine and a self-improving layer, all around **one event-sourced daemon**.

Four binaries ship today:

```
pii         terminal entry point; embeds the daemon in-process
pii-mcp     MCP server over stdio
pii-web     web UI
pii-evolve  the evolution loop
```

## Shape

```
crates/pii-abi         wire types shared by every crate
crates/pii-core        the domain: events, state, transitions
crates/pii-store       persistence
crates/pii-graph       graph engine for automation
crates/pii-memory      memory / knowledge store
crates/pii-model       model access
crates/pii-tools       tool surface
crates/pii-mcp         MCP server
crates/pii-worker      worker pool
crates/pii-server      daemon + web
crates/pii-cli         the `pii` binary
...
```

22 crates plus `xtask`. `#![forbid(unsafe_code)]` is workspace-wide, and **no per-crate override
exists** — the rule is not decorative.

## The bar

Two trees, eight steps:

```sh
bash quality-gate.sh                     # build + fmt · clippy · clippy(llamacpp) · test lib/bins/tests
                                         # · test doc · lint-files · lint-scratch · reachability-check
cargo run -q -p xtask -- fresh-gate      # exports HEAD with `git archive` and runs the same gate there
```

`fresh-gate` exists because **a gate can be green in a dirty tree and red in a clean clone** — leftover
files, untracked fixtures and local scratch all hide real breakage. When the two disagree, believe
`fresh-gate`.

Two constraints that shape the code:

- **450-line file cap** per file.
- **Scratch lives under `_tmp/`, never `/tmp`**, and a test must *create* the scratch it uses — enforced as
  a gate step (`lint-scratch`), because a test that silently reuses leftover state passes for the wrong
  reason.

## Spec-driven development

Features start as numbered specs in `specs/` carrying a TDD contract and **runnable** exit criteria:

```sh
pidag lint specs/01-xxx.md
pidag sdd  specs/01-xxx.md --run --concurrency 2
```

The planner writes specs; the workhorse implements them and never commits; the orchestrator reviews the
diff and commits. The spec set is read-only for the implementer by rule, not by convention.

## Status

Research-grade infrastructure under active development, not a packaged product. The interesting parts are
the substrate (event-sourced daemon, graph engine, worker pool) and the enforcement machinery above.

## Licence

MIT. See [`LICENSE`](LICENSE).

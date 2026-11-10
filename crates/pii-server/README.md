# pii-server

`pii-server`: the daemon side of the boundary.

Public modules: `daemon`, `engine`, `graph_paths`, `node_kinds`, `observed_provider`, `tiers`.

Depends on: `pii-abi`, `pii-channels`, `pii-core`, `pii-graph`, `pii-hal`, `pii-learn`, `pii-memory`, `pii-model`, `pii-net`, `pii-observe`, `pii-proto`, `pii-store`, `pii-tools`, `pii-wasm`, `pii-worker`.
Used by: `pii-cli`, `pii-mcp`.

105 source files; 154 integration test files under `tests/`. Test with `cargo test -p pii-server`.

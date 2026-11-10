# pii-net

`pii-net`: the one socket. The only crate in the workspace that opens a socket or resolves a name — enforced by `cargo xtask net-check`. Synchronous by design: `GraphNode::run`, `ModelProvider::chat` and `pii_core::HttpTransport` all execute on blocking threads.

Public modules: `authorize`, `clock`, `dial`, `grant`, `http_codec`, `limits`, `listen`, `resolve`, `service`, `testing`.

Depends on: `pii-abi`, `pii-core`, `pii-observe`.
Used by: `pii-server`.

17 source files; 6 integration test files under `tests/`. Test with `cargo test -p pii-net`.

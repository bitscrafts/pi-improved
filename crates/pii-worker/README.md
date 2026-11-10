# pii-worker

`pii-worker`: supervision and hot reload. The worker wire vocabulary lives here, decoupled from `pii-proto`.

Public modules: `control`, `frame`, `spec`, `supervisor`.

Depends on: `pii-abi`, `pii-core`, `pii-observe`.
Used by: `pii-cli`, `pii-server`.

8 source files; 2 integration test files under `tests/`. Test with `cargo test -p pii-worker`.

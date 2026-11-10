# pii-proto

`pii-proto`: the wire contract between clients and the daemon.

Public modules: `client`, `event`, `method`, `run`, `session`, `sys`.

Depends on: `pii-observe`.
Used by: `pii-cli`, `pii-evolve`, `pii-server`, `pii-tools`, `pii-tui`, `pii-web`, `xtask`.

7 source files; 2 integration test files under `tests/`. Test with `cargo test -p pii-proto`.

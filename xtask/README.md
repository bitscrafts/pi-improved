# xtask

Library surface for `xtask`'s dev-tooling modules that need to be reachable from another crate's tests without spawning a subprocess.

Binaries: `xtask`.

Public modules: `cli_parse`, `file_size`, `heal_collect`, `heal_seed`, `lint_files`, `lint_scratch`, `promote_build`, `reachability`.

Depends on: `pii-core`, `pii-memory`, `pii-proto`, `pii-store`.
Used by: no other crate of this workspace.

29 source files; 7 integration test files under `tests/`. Test with `cargo test -p xtask`.

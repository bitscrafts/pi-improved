# pii-learn

`pii-learn`: the learning loop's first stage — a deterministic, offline reader that turns the trajectory substrate into **cited candidate lessons**. It proposes; it never acts. Stage 124 verifies, 125 promotes.

Public modules: `distance`, `incident`, `incident_runtime`, `incident_store`, `loop_events`, `store`.

Depends on: `pii-core`, `pii-graph`, `pii-observe`, `pii-store`.
Used by: `pii-cli`, `pii-server`.

18 source files; 13 integration test files under `tests/`. Test with `cargo test -p pii-learn`.

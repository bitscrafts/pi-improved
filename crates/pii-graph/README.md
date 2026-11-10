# pii-graph

Graph engine: graph model, TOML loader, expression language, node harness ABI, and static validator.

Public modules: `agent_calls`, `blackboard`, `cron`, `effect`, `expr`, `fragment`, `journal`, `node`, `node_id`, `nodes`, `nodes_require`, `nodes_route`, `render`, `run`, `run_helpers`, `sched`, `sop`, `spec`, `template`, `toml_load`, `validate`.

Depends on: `pii-abi`, `pii-core`, `pii-observe`, `pii-store`.
Used by: `pii-cli`, `pii-learn`, `pii-server`.

44 source files; 50 integration test files under `tests/`. Test with `cargo test -p pii-graph`.

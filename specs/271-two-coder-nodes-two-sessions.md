# Spec 271: two coder nodes of one run get two pi sessions — the default session file carries the node id

## Overview
**Project**: <repo>
**Phase**: production — composition, shared state (Opus context review 2026-10-24, Q5 and risk 1: "pi session collision")

A pi coder's default session file is `<workdir>/.pii-sessions/<spec-stem>-<run>.jsonl` (`crates/pii-server/src/node_kinds/nodes/agent_worker.rs:31-43`, `pi_session_path`). The key is `(spec, run)`. Two coder nodes of one run on one spec and workdir therefore resolve to the SAME file, and `write_pi_session_header` leaves an existing file untouched (`agent_worker.rs:45-53`), so the second node silently RESUMES the first node's conversation. For the composition work this is the dangerous case: a `graphs/docs.toml` coder called after `spec.toml`'s implementation coder would continue the implementation session and edit code with that context. Claude is unaffected: it mints a fresh session id on every spawn when none is given (`agent_worker.rs:350-351`).

The key must be `(run, node)`. The obstacle is that a node cannot learn its own id: `build_nodes` calls `factory.build(&n.cfg)` with the node's config only (`crates/pii-graph/src/run_helpers.rs:42-43`), `NodeInput` carries `input`, `payload` and `nodes` (`crates/pii-abi/src/node.rs:13-22`), and `TraceCtx` carries the run and a span (`crates/pii-core/src/principal.rs:36-41`). This spec supplies the id through the node's built config and has the coder compute its own path, so `WorkerSpec` and `agent_worker.rs` are NOT edited (that file is 546 lines, and `pi_spec()`, `spec()` in three existing tests build a `WorkerSpec`; a new field would break them).

## Verified Premises
- **CONFIRMED** (read, 2026-10-25) `run_helpers.rs:36-47` `build_nodes`: `factory.build(&n.cfg)?` — the cfg is the node's own table, no id. A node's `NodeSpec` has `id` and `cfg` (`n.id`, `n.cfg`).
- **CONFIRMED** (read) `agent_worker.rs:31-43` as above; `WorkerSpec` (`:192-211`) has `session: Option<String>` ("Session file (pi); defaults to one beside the spec's name") and `run: Option<String>`; there is no node field.
- **CONFIRMED** (read) `coder/graph_node.rs:48-51` sets `run: Some(ctx.trace().run...)` on the `WorkerSpec`, and `:84-92` builds a SECOND `WorkerSpec` (`planned`) with `workdir: CONTAINER_WORKDIR` for a container run, so a default session path must be derived from the workdir of EACH spec (host path for the host, `/work/...` for the planned one). An explicit `session` passes through as written, today, in both.
- **CONFIRMED** (read) tests that pin the CURRENT shape and must keep passing UNEDITED: `crates/pii-server/tests/coder_session_is_per_run.rs` (two runs, two files, header plus one pid line, explicit session pins) and `agent_session_header.rs:60-77` (`WorkerSpec::from_cfg` with `run` in the cfg and NO node id gives `.../207-the-coder-node-spawns-an-agent-r000000000042.jsonl`).
- **CONFIRMED** (read) `validate.rs` and `toml_load.rs` have no unknown-cfg-key rejection (`woven-from` is an arbitrary key on any kind), and the built cfg is not what the journal hashes (`spec.to_json()` is taken from the spec, `run/mod.rs:96`).
- **INFERRED, MEASURE FIRST** that no node factory rejects or mis-reads an extra `node-id` key in the config it is built with. Run `cargo test -q -p pii-graph -p pii-server` after the injection and before anything else; if a factory refuses an unknown key or a test compares a built cfg byte for byte, STOP AND ASK naming the file.
- **NOT CLAIMED** that a coder's output carries its session handle, so this spec does NOT add `session = "$.nodes.code.session"` sharing; the explicit share is a literal `session = "<path>"` (already supported) and is tested here as the control. Sharing by expression is a follow-up that first measures what the coder returns.

## Requirements
### Functional
- **R1. The node id reaches the node.** `build_nodes` passes each factory a copy of the node's cfg with `node-id = "<node id>"` added when the cfg is a table and does not already carry that key; a cfg that already has `node-id` is passed unchanged. The spec's own `NodeSpec.cfg`, its hash and the journal are not changed. The helper is `with_node_id(cfg: &Value, id: &NodeId) -> Value` in a NEW file `crates/pii-graph/src/node_id.rs`, called from `build_nodes` (at most 3 added lines in `run_helpers.rs`).
- **R2. The coder's default session carries the node.** New file `crates/pii-server/src/node_kinds/nodes/coder/session_path.rs` with `node_session_path(workdir, spec_path, run, node) -> String` = `<workdir>/.pii-sessions/<spec-stem>-<run>-<node>.jsonl`, where `<node>` is the node id with every character outside `[A-Za-z0-9_.-]` replaced by `_` (so an id can never add a path separator or `..` segment: an id that is only dots, or empty, becomes `node`). In `coder/graph_node.rs`, when the coder's cfg has NO `session` (after the spec-257 resolution) and the agent is pi and the cfg carries `node-id`, the host `WorkerSpec` gets `session = Some(node_session_path(<host workdir>, ...))` and the container-planned one gets the path for `CONTAINER_WORKDIR`. A cfg with an explicit `session`, an agent that is not pi, or no `node-id` behaves EXACTLY as today.
- **R3. Loops keep their conversation.** A node that runs again (a loop) resolves to the same path both times, so a fix loop resumes. Two nodes of one run resolve to two paths. Two RUNS of one node resolve to two paths (the run stays in the name).
- **R4. Tests, with controls.** New `crates/pii-graph/tests/build_injects_node_id.rs` (a stub factory that records the cfg it is built with) and `crates/pii-server/tests/coder_session_per_node.rs` (real graph, the fake agent of `coder_session_is_per_run.rs`, copy its helpers; edit no existing test). Graph: (a) a node's factory sees `node-id` equal to its id; (b) CONTROL: a cfg that already carries `node-id = "mine"` is built with `mine`; (c) the spec's cfg and its hash are unchanged by the build (compare `spec.to_json()` before and after). Server: (d) two pi coder nodes `code` and `docs` in one run on one spec and workdir journal two DIFFERENT `coder.spawned.session` paths, and both files exist with the pi header (this fails today); (e) CONTROL: the same two nodes with the SAME literal `session = "<path>"` journal the identical path and the file holds the header plus BOTH pids; (f) CONTROL: one node run twice in a loop journals the same path both times; (g) two runs of one node journal two paths; (h) a node id `a/../b` yields a path whose file name is `...-a_.._b.jsonl` under `.pii-sessions` (no traversal, the file is inside the workdir); (i) CONTROL: a claude coder is unchanged (its `coder.spawned` has no `session` path derived from the node; assert the pi-only branch); (j) `node_session_path` unit cases: the shape for a plain id, `..`, empty and a unicode id.

### Non-Functional
- Do NOT edit `agent_worker.rs`, `WorkerSpec`, any existing test, `coder_session_is_per_run.rs`, `agent_session_header.rs`, `coder_tree_digest.rs`. `tree.rs` already skips `.pii-sessions`. Every file under the 450-line gate (effective lines; `graph_node.rs` is 262). No new dependency.
- The container path: a coder with `container_image` plans its session under `CONTAINER_WORKDIR`. Cover it with a unit assertion on the two computed paths (host and `/work`), not by running a container.

## TDD Contract
| Test | Expected |
|---|---|
| `a_factory_sees_its_node_id` | cfg has `node-id` == id |
| `CONTROL_an_existing_node_id_is_kept` | `mine` stays |
| `the_spec_and_its_hash_are_unchanged` | `to_json()` equal |
| `two_nodes_get_two_sessions` | two different journalled paths, both files with header |
| `CONTROL_one_literal_session_is_shared` | identical path, header + both pids |
| `CONTROL_a_loop_resumes_its_own_file` | same path twice |
| `two_runs_get_two_sessions` | different paths |
| `a_hostile_node_id_cannot_leave_the_workdir` | `...-a_.._b.jsonl`, inside workdir |
| `CONTROL_claude_is_unchanged` | no node-derived session |
| `the_planned_path_follows_the_container_workdir` | host and `/work` forms |

**The requirement a plausible implementation satisfies only in appearance:** changing only the HOST path. A container coder is planned with `workdir = /work`, so a path computed from the host workdir (or left as the old `(spec, run)` default) puts the second container node back on the first one's file. Test `the_planned_path_follows_the_container_workdir` fails it.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test build_injects_node_id 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test coder_session_per_node 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test -f crates/pii-graph/src/node_id.rs`
- [ ] `test -f crates/pii-server/src/node_kinds/nodes/coder/session_path.rs`
- [ ] `cargo test -q -p pii-server --test coder_session_is_per_run 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test agent_session_header 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test coder_tree_digest 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `git diff --quiet -- crates/pii-server/src/node_kinds/nodes/agent_worker.rs crates/pii-server/tests/coder_session_is_per_run.rs crates/pii-server/tests/agent_session_header.rs crates/pii-server/tests/coder_tree_digest.rs`
- [ ] `cargo clippy -q -p pii-graph -p pii-server --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit `agent_worker.rs`, `WorkerSpec` or the four test files named in the last criterion. Do NOT change a session path when the cfg has an explicit `session`, the agent is not pi, or the cfg has no `node-id`. Do NOT put the node id into the journalled spec or its hash. Do NOT commit.

## Follow-ups (not this spec)
1. Sharing by expression (`session = "$.nodes.code.session"`): first measure what the coder node RETURNS (whether the handle is in its output value), then spec it with the control that two such nodes get the identical path.
2. Claude resume across a crash (`agent_worker.rs:351` mints a new id on re-execution; nothing reads the journalled `claude_session_id` back).
3. `graphs/docs.toml` called from `spec.toml`, which this spec unblocks for pi.

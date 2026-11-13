# Spec: a spec drafted by a graph is written in the daemon's workspace, whatever directory the daemon runs from
#: depends=412 status=pending block=-

## Overview
Measured 2026-11-12 on row 413 (`graphs/github-issue.toml`), the graph that drafts a spec from a `ready` issue. Its `write` node is `kind = "ledger"`, `op = "spec.write"`, `workdir = "."`. Run by a
daemon whose workspace is `<dir>/ws`, the run failed with `capability denied: fs_write`: `write_spec` (`crates/pii-server/src/node_kinds/nodes/plan.rs`) joins `workdir` onto `specs/` as a plain
`Path`, so `"."` means the process's current directory, not the workspace the run's `fs_write` scope covers. In production it works only if the daemon was started from its workspace; a daemon started
anywhere else (a systemd unit, a test) is denied. The graph cannot name the workspace: no expression key exposes it, and a path in a shipped graph is exactly what spec 411 removed from the repository's scripts.

`ledger` is built without the workspace (`LedgerFactory` in `node_kinds/nodes/ledger.rs`; `registry.rs` already hands it to `agent`, `tool` and `heal`). This row gives `spec.write` the workspace and makes a RELATIVE
`workdir` mean "inside the workspace".

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 412

## Requirements
- R1 `spec.write` resolves a relative `workdir` against the daemon's workspace (`EngineConfig::workspace`). `"."` is the workspace; `"sub"` is `<workspace>/sub`.
- R2 An absolute `workdir` is used as given, as today: every existing graph and test that passes one behaves as before. The run's `fs_write` scope still decides what it may write.
- R3 A relative `workdir` that leaves the workspace (`..`, or a symlink out of it) is refused with a reason naming the path, and nothing is written. The refusal is the node's `err` port, not a panic and not a write outside.
- R4 The refusal of a missing `workdir` ("plan needs a workdir") is unchanged.
- R5 No other node kind's behaviour changes. `plan` (the older node that drafts and writes in one step) keeps its current resolution unless it shares the same function, in which case its tests still pass unedited.

- R6 `crates/pii-server/src/lib.rs` is at its recorded size (669 lines) and the file-size ratchet refuses growth (measured 2026-11-13: the first pass added one argument line there and the gate failed `lint-files` and `crate-audit`).
  The engine's call that registers the queue's nodes reaches the workspace WITHOUT a new line in `lib.rs`: a wrapper in `registry.rs` takes the pieces `lib.rs` already holds (`&cfg`, queue, memory, heals, log) and builds the
  gate runner from `cfg.workspace` itself, so the call in `lib.rs` is shorter than today's, never longer. `register_queue_nodes` keeps its current signature for its other callers.

## Architecture
```
registry.rs   LedgerFactory { ..., workspace }           (the value agent/tool/heal already receive)
ledger.rs     "spec.write" => SpecWriteNode { cfg, workspace }
plan.rs       write_spec(ctx, finding, root, reply, workspace): root = workspace.join(root) when root is relative
```
Key decisions:
- The engine learns what "here" is. Rejected: `workdir = "/abs/path"` in the graph (a machine path in a shipped file). Rejected: an input key the router adds to every channel run (a path into every channel graph's input,
  including graphs that hand their input to a model). Rejected: making the daemon `chdir` to its workspace (process-global, and the other tests share a process).
- A relative path inside the workspace is the only new meaning; absolute paths and the scope check are untouched, so this widens nothing.

## TDD Contract
New tests in `crates/pii-server/tests/spec_write_workspace.rs`, over a daemon with workspace `<tmp>/ws` and the process's directory elsewhere.
| Test Name | Input | Expected Output |
|---|---|---|
| `a_relative_workdir_is_the_workspace` | `spec.write` with `workdir = "."` | the spec lands in `<ws>/specs/`, and the run succeeds |
| `an_absolute_workdir_is_used_as_given` | `workdir = "<ws>"` (absolute) | the same file, same result |
| `CONTROL_a_relative_workdir_that_leaves_the_workspace_is_refused` | `workdir = ".."` | `err` port with the path in the reason; no file written outside `ws` |
| `CONTROL_a_symlink_out_of_the_workspace_is_refused` | `<ws>/out -> /tmp/x`, `workdir = "out"` | refused; nothing written under `/tmp/x` |
| `a_missing_workdir_still_refuses_with_the_existing_reason` | no `workdir` | "plan needs a workdir" |
| `an_absolute_workdir_outside_the_workspace_is_left_to_the_scope_not_refused_as_leaving_it` | absolute `workdir` in another directory | not refused as "leaves the workspace"; the scope denies the write; nothing created there (added after mutation C survived: an absolute path inside the workspace cannot tell the two apart) |

Acceptance: the five pass; each of these mutations makes at least one fail (the mutation script is the orchestrator's): the join removed (relative stays relative); the containment check removed; the absolute case also joined.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-server --test spec_write_workspace 2>&1 | grep -qE 'test result: ok. [5-9] passed'`

## Guardrails
- Keep green: `cargo test -q -p pii-server --test plan_writes_a_spec`, `--test self_improve_composes`, and every test that already starts a graph with a `spec.write` node. Do NOT edit an existing test.
- Do NOT grow `crates/pii-server/src/lib.rs`: it must be 669 lines or fewer after the change. Do NOT raise a baseline.
- Do NOT change what `fs_write` allows, and do NOT add a capability.
- `crates/pii-server/src/node_kinds/nodes/plan.rs` and `ledger.rs`: if the change takes either past 450 lines, make the module a folder in the same row.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-server/src/lib.rs`
- `crates/pii-server/src/node_kinds/nodes/plan.rs`
- `crates/pii-server/src/node_kinds/nodes/ledger.rs`
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-server/tests/spec_write_workspace.rs`

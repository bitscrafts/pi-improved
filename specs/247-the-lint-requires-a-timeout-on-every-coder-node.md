# Spec 247: the lint requires a timeout on every coder node

## Overview
**Project**: /projects/pi-improved
**Phase**: post-vision hardening — agent management (defect 17 in `_docs/05-operations/agent-management-findings-2026-10-23.md`)

Lint rule V10 (spec 162) says a node that can block on a process must carry a positive `timeout_ms`. It finds such nodes through
their factory's declared effects (`Effects::SPAWNS_PROC`). The `coder` node declares only `Effects::MUTATES_EXTERNAL`
(`crates/pii-server/src/node_kinds/nodes/coder.rs`, `fn effects`), so V10 never looks at it, although a coder node spawns an agent
that can run for as long as it likes. Measured: `graphs/coder-demo.toml`, `graphs/coder-podman.toml` and `graphs/coder-selffix.toml`
have no `timeout_ms` and `pii graph lint` accepts all three. (The daemon still bounds the agent through a default in
`agent_timeout(cfg)`, spec 207; the lint is the author-facing statement that a bound was chosen on purpose.)

Do NOT change the coder's declared effects: `SPAWNS_PROC` makes a node not parallel-safe, which would change how the scheduler
treats every coder graph. The rule is widened instead: V10 also applies to a node whose KIND is `coder`.

## Requirements
### Functional
- **R1.** In `crates/pii-graph/src/validate.rs`, the V10 check also fires for a node whose `kind == "coder"`, with the same message
  shape: `V10: node '<id>' spawns a process and requires a positive 'timeout_ms'`. A coder node with a positive `timeout_ms`
  passes. A node of any other kind is judged exactly as today.
- **R2.** The three shipped graphs that lack it gain `timeout_ms`: `graphs/coder-demo.toml`, `graphs/coder-podman.toml`,
  `graphs/coder-selffix.toml`. Value `1800000` (30 minutes) in each, on the `code` node, next to `env_pass`/`criteria`. No other edit.
  `graphs/claude-probe.toml` already has one and is not touched.
- **R3.** `pii graph lint` on each of the four shipped coder graphs (`claude-probe`, `coder-selffix`, `coder-demo`, `coder-podman`)
  prints no `V10` line.

### Non-Functional
- No file over 450 effective lines. `crates/pii-graph/src/validate.rs` is 520 raw lines today: check
  `cargo run -q -p xtask -- lint-files` before finishing, and if the file is over, move the V10 block into a function in a new
  sibling file rather than growing it.
- clippy clean (`-D warnings`), no `unwrap`/`expect`/`panic` in library code.

## Architecture
```
validate.rs  V10:  spawns = factory.effects() has SPAWNS_PROC   OR   node.kind == "coder"
                   if spawns and timeout_ms is not a positive integer -> error V10
```

## TDD Contract
New test file `crates/pii-graph/tests/coder_requires_timeout_lint.rs`. Copy its header and the `TwoPort` stand-in factory from
`crates/pii-graph/tests/coder_err_edge_lint.rs` (same crate, same helpers: it builds a `GraphSpec` and calls `validate`). The
stand-in is registered under the kind `"coder"` with `Effects::PURE`, exactly like the real coder's lack of `SPAWNS_PROC`.

| Test name | Input | Expected |
|---|---|---|
| `a_coder_node_without_timeout_is_a_v10_error` | one `coder` node, no `timeout_ms` | `validate` returns an error containing `V10` and the node id |
| `a_coder_node_with_zero_timeout_is_a_v10_error` | `timeout_ms = 0` | error containing `V10` |
| `a_coder_node_with_a_timeout_passes_v10` (control) | `timeout_ms = 1800000` | no error containing `V10` |
| `a_non_coder_node_without_timeout_is_not_a_v10_error` (near miss) | a pure `transform`-like stand-in, no `timeout_ms` | no error containing `V10` |
| `the_shipped_coder_graphs_pass_v10` | load the four shipped graph files by path (`graphs/<name>.toml` relative to `CARGO_MANIFEST_DIR/../..`) with the builtin registry plus the stand-in `coder` | none of them yields a `V10` error |

Every "error containing" assertion must also assert the message names the node (`n.id`), as the existing V10 message does.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test coder_requires_timeout_lint 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `! printf '%s\n' claude-probe coder-selffix coder-demo coder-podman | xargs -I{} cargo run -q -p pii-cli --bin pii -- graph lint graphs/{}.toml 2>&1 | grep -q V10`
- [ ] `! grep -c 'timeout_ms' graphs/coder-demo.toml graphs/coder-podman.toml graphs/coder-selffix.toml graphs/claude-probe.toml | grep -q ':0$'`
- [ ] `cargo test -q -p pii-graph 2>&1 | grep -E "^test result" | grep -vq FAILED`
- [ ] `cargo clippy -q -p pii-graph --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files`

## Guardrails
- Do NOT change `Effects` on the coder factory, `Effects::SPAWNS_PROC`, or any other node's lint behaviour.
- Do NOT change any existing test; add the new file only. If an existing test fails because of R1, STOP and report which one
  and why, in your final message, instead of editing it.
- Do NOT touch V1–V9 or V11.
- Do NOT commit. Leave all work in the working tree.

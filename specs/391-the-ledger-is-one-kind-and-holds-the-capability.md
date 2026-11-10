# Spec: the ledger is one kind, and it holds the capability

## Overview
Spec 383 step 2. Four kinds today do one thing — change stored state that other runs read:
`select` (claim a queue row), `record` (write its verdict), `enqueue` (add a row), `heal_record` (write a
heal attempt). Each is a separate kind, each is a separate module, and **none of them holds a
capability**: any graph that loads can claim a queue row or write a verdict. Opus 045 and 046 both
flagged this, and the second finding is the security one.

**Project**: <repo>
**Phase**: 383 step 2 of 4
**Depends on**: 383 step 1 (transform computes) — done

## Requirements
### Functional
- R1 one kind `ledger` with an `op` field: `queue.claim`, `queue.set_status`, `queue.put`,
  `heal.record_run`. The four old kinds are derived from it and refuse with a message naming the new
  spelling, so an old graph fails at load saying what to write instead.
- R2 `ledger` requires the capability bit `LEDGER` (new, value 1024). `cap.rs` gains the constant and
  the `memory_read`-style name mapping; no graph gets the bit for free.
- R3 a `ledger` node's `op` is checked at LOAD: an op that is not one of the four is refused, naming
  the node and the op.
- R4 the effect class of every op stays what it was (`record` and `enqueue` mutate; `select` writes a
  claim), so the replay refusal that exists today does not weaken.

### Non-Functional
- R5 the old module files do not double the logic: one implementation behind the one kind.
- R6 every stored-graph still loads and every existing test that names the old kinds is updated to the
  new spelling in the same change.

## TDD Contract
| Test name | Input | Expected |
|---|---|---|
| `a_graph_without_the_bit_cannot_claim` | a graph with a `ledger` node, no LEDGER cap | load refused, names the capability |
| `the_four_ops_are_the_whole_surface` | `op = "queue.steal"` | refused at load, names node + op |
| `an_old_kind_names_its_replacement` | `kind = "select"` | refused, message says `ledger` with `op = "queue.claim"` |
| `CONTROL_a_graph_with_the_bit_claims_once` | the real `queue.toml` | claims, and a second claim of the same row does not double-take |

## Exit Criteria
- [ ] `grep -q 'const LEDGER' crates/pii-abi/src/cap.rs`
- [ ] `cargo test -q -p pii-server --test ledger_kind 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-server --test queue_calls_spec 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `test "$(grep -rl 'kind = "select"' graphs/ | wc -l)" = 0`
- [ ] `./target/debug/pii graph lint graphs/queue.toml`
- [ ] `./target/debug/pii graph lint graphs/self-improve.toml`

## Guardrails
- Do NOT give `ledger` the bit by default in any test fixture: the point is that a graph must be granted
  it, and a fixture that grants it silently re-creates today's hole.
- Do NOT keep the four old kinds alive as aliases that also hold the bit.
- Do NOT change what an op writes; this row changes the vocabulary and the capability, not the store.

## Files
- `crates/pii-abi/src/cap.rs`
- `crates/pii-server/src/node_kinds/nodes/queue.rs`
- `crates/pii-server/src/node_kinds/nodes/heal.rs`
- `crates/pii-server/src/node_kinds/nodes/ledger.rs`
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-server/tests/ledger_kind.rs`
- `graphs/queue.toml`
- `graphs/self-improve.toml`

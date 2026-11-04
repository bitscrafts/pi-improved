# Spec: the work ledger is one kind, and it has a capability

## Overview

Derivation 046: `select`, `record`, `enqueue` and `heal_record` are four names for one transaction on the
work ledger — claim a row with a lease by run id, set its status, put a row, record a heal run. They
became four kinds because each arrived with its own graph, not because they touch different things.

Measured while reviewing the derivation: **none of the four holds a capability today** — `select` and
`record` require nothing (`queue.rs:96,327`), and `heal_record` requires nothing (`heal.rs:296`). Any
graph, including a sample or a user graph, can claim a queue row.

## Requirements

- **R1 — one kind `ledger`, four ops**: `queue.claim`, `queue.set_status`, `queue.put`,
  `heal.record_run`. Each keeps its current semantics and ports exactly; only the name changes.
- **R2 — a capability.** `CapKind::Ledger` (or an existing bit if one is the honest fit) is required by
  every `ledger` node and checked at load. The ladder is unchanged: a graph may hold it or not.
- **R3 — the four old kinds are deleted**, and every graph using them is rewritten to `ledger` with the
  same pins, ports and caps. The lint from 383 reports any leftover.
- **R4 — the transaction is still atomic**: a claim with its lease is one write, as today. A crash
  between two nodes must not leave a half-claimed row — that is why this is a kind and not a graph of
  read-compute-write nodes.
- **R5 — replay safety is preserved**: the kind's effect class is unchanged, so a resumed run does not
  re-claim or re-record.

## TDD contract

| Test | Input | Expected |
|---|---|---|
| `ledger_claim_is_one_transaction` | two claims for one row | second refuses, naming the lease holder |
| `ledger_requires_the_capability` | a graph without the bit | refused at load, naming the node and the bit |
| `CONTROL_a_graph_holding_the_bit_claims_normally` | the same graph with the bit | claims |
| `the_four_old_kinds_are_gone` | `graphs/` | no node of kind `select`/`record`/`enqueue`/`heal_record` |

## Exit Criteria

- [ ] `grep -q '"ledger"' crates/pii-server/src/engine/registry.rs`
- [ ] `grep -qE 'Ledger|ledger' crates/pii-abi/src/cap.rs`
- [ ] `! grep -rqE 'kind = "(select|record|enqueue|heal_record)"' graphs/`
- [ ] `cargo test -q -p pii-server --test ledger_kind 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`

## Guardrails

- Do NOT give the ledger a capability that a *coder* implicitly holds: a coder implementing a spec must
  not be able to claim rows by existing.
- Do NOT split the transaction across nodes to avoid a new kind.
- Do NOT rename without the capability: a rename alone leaves the exposure.

## Files

- `crates/pii-server/src/node_kinds/nodes/queue.rs`
- `crates/pii-server/src/node_kinds/nodes/heal.rs`
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-abi/src/cap.rs`
- `crates/pii-server/tests/ledger_kind.rs`
- `graphs/queue.toml`
- `graphs/self-improve.toml`

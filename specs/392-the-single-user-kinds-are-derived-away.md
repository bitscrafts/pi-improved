# Spec: the single-user kinds are derived away

## Overview

Derivation 046, test 3: of the 23 kinds, eight are used by exactly one graph. Four are derivable with
no loss; one moves to a test fixture; the rest are already covered by another row.

```
worker       → coder          both of its graphs mean "an external agent, bounded"
classify     → route          pure branching with several arms
plan         → agent          a model call plus string building
memory       → remember       one op (consolidate); its only loadable user is a starter graph
debug_crash  → a test fixture  its one user exists to panic a node
```

## Requirements

- **R1 — each derivation is a rewrite of the graph, not a shim.** The kind is deleted from the registry
  and the graph is expressed with the target kind. No alias that keeps the old name alive.
- **R2 — behaviour is preserved and proved**: each rewritten graph keeps its entry, its exit, its ports
  and its verdicts, and its existing test passes unchanged.
- **R3 — `starter/crash.toml` becomes a test fixture**, registered only inside the test that uses it,
  so the shipped library contains no kind whose purpose is "a node can panic".
- **R4 — the lint from 383 goes green** on the derived set: no kind outside the fourteen, and no kind
  with exactly one user left unaccounted.

## Exit Criteria

- [ ] `! grep -rqE 'kind = "(worker|classify|plan|memory|debug_crash)"' graphs/`
- [ ] `! grep -q '"worker"\|"classify"\|"plan"\|"memory"\|"debug_crash"' crates/pii-server/src/engine/registry.rs`
- [ ] `cargo run -q -p xtask -- lint-graph-kinds`
- [ ] `test ! -f graphs/starter/crash.toml`

## Guardrails

- Do NOT keep `memory` as an alias of `remember`: two names for one resource is how the count grows.
- Do NOT delete a kind while a graph still names it — the lint reports it; rewrite first.
- Do NOT put the crash fixture back in `graphs/` to make a test simpler.

## Files

- `crates/pii-server/src/engine/registry.rs`
- `graphs/starter/crash.toml`
- `crates/pii-server/tests/crash_fixture.rs`
- `docs/03-reference/graph-vocabulary.md`

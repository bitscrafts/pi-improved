# Spec: sdd is a graph, not a node's name

**Project**: /projects/pi-improved · **Phase**: 0.2.1 · **Source**: user directive — *"one graph that do tdd, one graph
that do sdd, and other graphs composing them!"* — and what `graphs/night.toml` actually says today.

## Why this exists

`night.toml` has a node whose **id is `sdd`**, and its kind is `call` with `graph = "queue"`. So the night claims to run
SDD and runs the queue instead. The name is a lie in the only place an operator would look.

`graphs/spec.toml` is the closest thing to an SDD graph: the spec is the contract, `code` implements, `verify` runs the
criteria, `docs` writes documentation, `reverify` re-runs them. But it does **not** do TDD (spec 372 gives it the RED
step) and it declares no `exit`, so nothing can compose it.

The graph the night should call is one that: takes a spec, proves each criterion RED before the code exists, implements,
proves GREEN, writes the docs, and reports one of three verdicts.

## Requirements

- **R1** `graphs/sdd.toml` exists and is that graph: `contract` (judge the spec's criteria are present and runnable) →
  `tdd` (`call` of `graphs/tdd.toml`, pinned) → `verify` (judge) → `docs` (`call`) → `reverify` (judge) → `done`;
  ports `unverified` (work exists, criteria fail) and `refused` (the run could not proceed), exactly as `spec.toml`
  has them today.
- **R2** `sdd` declares `exit = "done"` so the night can call it.
- **R3** `sdd` holds every capability `tdd` and `docs` need, or the `call` ladder refuses it — the ladder is the proof.
- **R4** `graphs/night.toml`'s `sdd` node calls **`sdd`**, re-pinned to its sha256. A node named `sdd` that calls
  something else is the defect this spec removes.
- **R5** `queue` keeps working: it is not replaced for row-by-row work, and `spec.toml` stays as the hand-driven path
  until spec 364 folds it into the composition.

## Files

- `graphs/sdd.toml`
- `graphs/night.toml`
- `graphs/tdd.toml`
- `crates/pii-server/tests/night_calls_sdd.rs`
- `crates/pii-server/tests/night_graph_composes.rs`

## Exit Criteria

- [ ] `test -f graphs/sdd.toml`
- [ ] `grep -q 'graph = "sdd"' graphs/night.toml`
- [ ] `grep -q 'graph = "tdd"' graphs/sdd.toml`
- [ ] `grep -q 'exit = "done"' graphs/sdd.toml`

## Guardrails

- Do NOT rename the night's node away from `sdd`: make the name true instead.
- Do NOT widen `night`'s caps beyond what `sdd` needs; the ladder exists to catch exactly that.
- Do NOT let `sdd` report `done` on a spec whose criteria were never seen RED — that is what R1's `tdd` call is for.

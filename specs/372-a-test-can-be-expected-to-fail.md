# Spec: A test can be expected to fail

**Project**: <repo> · **Phase**: 0.2.1 · **Source**: user directive 2026-11-03 — *"one graph that do
tdd, one graph that do sdd, and other graphs composing them!"* — and the gap found 2026-11-04.

## Why this exists

There is no TDD graph, and the engine cannot express one. `judge_spec.rs` parses a spec's `## Exit Criteria` and runs
each command, reporting `ok` when they pass and `fail` when any fails. There is no way to say **"this must fail"**, so
the one thing TDD is named for — watching the test go RED before writing the code — cannot be proven.

Without that, work reported as "tested" may be a test written after the fact against code that already passed it. The
repo has paid for this shape before: a check that passes while the live path is broken is worse than no check.

## Requirements

- **R1** The judge accepts `expect = "fail"`: the node reports `ok` only when **every** criterion fails, and `fail`
  when any criterion passes. The default stays `pass`, so every existing graph is unaffected.
- **R2** The refusal is loud: a criterion that could not run at all (exit 127, missing binary) is neither a pass nor a
  proper failure, and the node says so on its own port rather than counting it as RED.
- **R3** `graphs/tdd.toml` implements the loop, as data:
  `write_test` (coder) → `red` (judge, expect fail) → `implement` (coder) → `green` (judge) → `done`, with ports for
  the two ways it can go wrong: `not-red` (the test passed before the code existed, so it proves nothing) and
  `still-red` (the implementation did not make it pass).
- **R4** `tdd` declares `exit = "green"` so it can be composed by an SDD graph.

## Files

- `crates/pii-server/src/node_kinds/nodes/judge_spec.rs`
- `crates/pii-server/src/node_kinds/nodes/queue.rs`
- `graphs/tdd.toml`
- `crates/pii-server/tests/judge_expects_a_failure.rs`

## Exit Criteria

- [ ] `test -f graphs/tdd.toml`
- [ ] `grep -q 'expect' crates/pii-server/src/node_kinds/nodes/judge_spec.rs`
- [ ] `cargo test -q -p pii-server --test judge_expects_a_failure 2>&1 | grep -q 'test result: ok'`
- [ ] `grep -q 'not-red' graphs/tdd.toml`

## Guardrails

- Do NOT change the default: a graph that does not say `expect` keeps today's meaning.
- Do NOT let a command that never ran count as a RED pass; R2 exists because that is the obvious shortcut.
- The near-miss control: a test that passes before the implementation must end on `not-red`, not `done`.

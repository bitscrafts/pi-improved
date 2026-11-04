# Spec: a transform can compute a value

## Overview

The one genuine addition in derivation 046. `transform` today resolves a path, a literal, or a plain
value; it cannot compute. There is no arithmetic, no concatenation, no comparison across two nodes'
values. Anything that needs one reaches for `tool` and a shell string — the escape hatch that made an
earlier "two-kind vocabulary" possible.

The measured workaround is in the tree: `boss-watch.toml:29-36` builds a value with shell.

## Requirements

- **R1 — `let` and `out`**: a `transform` node may bind intermediate values (`let = { name = "expr" }`)
  and produce a result (`out = "expr"`). Supported in the expression language: integer and string
  literals, path reads, `==`, `!=`, `<`, `>`, `+` (numbers add, strings concatenate), and a length.
- **R2 — arithmetic is exact and typed**: integer only, with an overflow refusal naming the expression;
  no floats (a float in a routing value is a comparison that lies).
- **R3 — a bad expression fails at LOAD**, naming the node and the expression — never at run time, and
  never as an empty value.
- **R4 — the graph that needs it is the proof**: `boss-watch.toml`'s shell workaround becomes a
  `transform` with `let`/`out`, and its behaviour is unchanged (same ports, same verdicts).
- **R5 — no capability change**: computation touches nothing and needs nothing.

## TDD contract

| Test | Input | Expected |
|---|---|---|
| `integers_add` | `out = "input.a + input.b"` with 2, 3 | 5 |
| `strings_concatenate` | `out = "input.a + input.b"` with "x", "y" | "xy" |
| `comparison_routes` | `out = "input.stamp == input.head"` | true / false |
| `an_unknown_name_fails_at_load` | `out = "input.nope + 1"` | refused at load, named |
| `overflow_is_refused` | two i64 maxima added | refused, naming the expression |
| `CONTROL_a_float_is_refused` | `out = "1.5 + 1"` | refused: floats are not part of the language |

## Exit Criteria

- [ ] `cargo test -q -p pii-graph --test expr_compute 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `grep -q 'overflow' crates/pii-graph/src/expr.rs`
- [ ] `! grep -q 'bash' graphs/boss-watch.toml`

## Guardrails

- Do NOT add a function library. Each function is a new language; the target is the smallest set that
  lets a graph compare and combine the values it already has.
- Do NOT allow a transform to run a command, read a file, or touch a capability.
- Do NOT evaluate lazily at run time: an expression that cannot be checked at load is a bug waiting to
  produce an empty value in production (measured: `$.nodes.inventory.payload.stdout`).

## Files

- `crates/pii-graph/src/expr.rs`
- `crates/pii-graph/tests/expr_compute.rs`
- `graphs/boss-watch.toml`

# Spec: The queue implements a spec by calling the graph that already does

**Project**: /projects/pi-improved · **Phase**: 0.2.1 · **Source**: user directive 2026-11-03 — *"create a graph to
implement specs? then use composability to integrate into the flow"*; and the standing rule that inclusion is `call`
(spec 255) and a second inclusion mechanism must never be built.

## Verified premises (measured 2026-11-03)

- `graphs/spec.toml` **is** the "implement any spec" graph (its own header, spec 257): `code` (coder) → `verify`
  (judge) → `docs` (a `call` of the docs graph) → `reverify` (judge) → `done` / `unverified` / `refused`.
  `grep -n '^id = ' graphs/spec.toml` → code, verify, docs, reverify, done, unverified, refused.
- It has **no `exit` node**: `grep -c '^exit' graphs/spec.toml` → 0. `queue.toml` gained one (`exit = "record"`) for
  spec 343 R6 precisely so that it could be inlined by `call`; `spec.toml` never did, so nothing can call it.
- `graphs/queue.toml` re-implements the same shape inline: `select` → `dispatch` (coder) → `judge` → `record`. That is
  the duplication this spec removes.
- Both graphs declare `caps = ["spawn", "proc"]`; the coder node now needs `fs_read`/`fs_write` (measured 2026-11-03:
  without them the agent has no write tool and stops).
- The cap ladder is enforced: a `call` whose granted caps do not cover the callee is refused with
  `graph.call.widen` naming the missing capability.

## Measured 2026-11-04: why the first attempt left the row Running

`graphs/spec.toml` was given `exit = "done"` and `queue.toml` was wired `implement.ok -> record`. The composition
loads and the happy path works, but `a_refusing_callee_fails_the_row` fails: the row stays `Running`, never `Parked`.

The mechanism is the splice's exit rule (`crates/pii-graph/src/fragment/splice.rs`): **a call returns through every
node that declares an `ok` port and has no outgoing `ok` edge** — not only the node the callee names in `exit`.
`spec.toml` ends in three nodes; `done` is a `transform` that declares `ok` and returns, while `unverified` and
`refused` are `kind = "refuse"`, declare no `ok`, and therefore **dead-end inside the splice**. A refused
implementation fires no edge, the run stops, and the claimed row is never recorded.

Two ways out — choose one and say why, because it changes the callee's contract:

1. **Every verdict returns on `ok`.** Make `unverified`/`refused` declare an `ok` port (or be `transform`s carrying a
   verdict value). All three then splice back, but the host must distinguish them by the value that arrived, because
   they share a port.
2. **One verdict node.** Collapse the tail into a single terminal node with ports `ok`, `unverified`, `refused`; the
   host wires each to the outcome it wants. Explicit vocabulary, and the queue can park on `refused` and mark
   `unverified` distinctly, but it changes `spec.toml`'s tail and every host that calls it.

Whichever you pick, the criterion is the same: a refused implementation must leave the row `Parked`, and an unfinished
one must not be recorded as `done`.

## Requirements

**R1.** `graphs/spec.toml` declares `exit = "done"` and grants `fs_read`/`fs_write` at the graph level, on `code` and on
`docs` if that call needs them. Its behaviour when run directly is unchanged: same entry, same refusals, same ports.

**R2.** `graphs/queue.toml` replaces its inline `dispatch` (coder) and `judge` nodes with one `call` of `spec`, pinned
by sha256, receiving the claimed row's `spec` path and the configured workhorse model/provider, and exposing the callee's
verdict through the call's id.

**R3.** `record` consumes the call's verdict: a passing verdict records `Done`; a refusing or unverified verdict records
`Failed` with the callee's refusal port, exactly as today. The row's own `criteria` are no longer run by the queue — the
spec graph's own `verify`/`reverify` judges do that, and `record` trusts their verdict. **A row whose spec has no
criteria must be refused at `queue.add` time** (already true) so this transfer of judgement cannot silently pass.

**R4.** The pins are honest: after any edit to either graph, the pin in the caller is updated in the same commit and
`pii graph lint <caller>` exits 0.

**R5.** Nothing new is invented: no new node kind, no second inclusion mechanism, no copy of `spec.toml`'s logic.

## The requirement a plausible implementation satisfies in appearance only

A worker can add `exit = "done"` and `call spec`, and leave `record` reading the *row's* criteria as before — then two
judges run (the call's and the queue's), the row fails when the call's verdict said pass, and nothing explains why. The
test below asserts `record` consumes the CALL's verdict: the queue no longer runs the row's criteria itself, and the
verdict that reaches `record` is the callee's.

## TDD contract

| file | test | assertion |
|---|---|---|
| `crates/pii-server/tests/queue_calls_spec.rs` | `a_row_is_implemented_by_the_spec_graph` | a row whose spec the stub coder satisfies → the run's trace shows `call`, the callee's `done`, and `record` with a passing verdict |
| same | `CONTROL_the_queue_runs_no_judge_of_its_own` | the trace contains no `judge` node in the queue's own namespace; the only verdict is the callee's |
| same | `a_refusing_callee_fails_the_row` | the callee refuses → the row records `Failed` naming the refusal port |
| `crates/pii-server/tests/spec_graph_is_callable.rs` | `spec_declares_an_exit_and_its_caps_cover_its_coder` | `exit` is `done`; `call spec` from a host holding `spawn,proc,fs_read,fs_write` is accepted; a host without `fs_write` is refused with `graph.call.widen` |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test queue_calls_spec 2>&1 | grep -qE 'test result: ok. 3 passed'`
- [ ] `cargo test -q -p pii-server --test spec_graph_is_callable 2>&1 | grep -qE 'test result: ok. 1 passed'`
- [ ] `grep -q '^exit = "done"' graphs/spec.toml`
- [ ] `grep -q 'kind = "call"' graphs/queue.toml`

## Guardrails

- Do NOT create a second implement-a-spec graph, and do NOT copy `spec.toml`'s nodes into the queue.
- Do NOT drop the pin: `call` requires `sha256` of the callee.
- Do NOT let the queue run the row's criteria AND the callee's judge; the verdict has one owner (R3).
- Do NOT change `spec.toml`'s behaviour when run directly.
- The worker does not commit.

## Files

- `graphs/spec.toml`
- `graphs/queue.toml`
- `graphs/night.toml`
- `crates/pii-server/tests/queue_calls_spec.rs`
- `crates/pii-server/tests/spec_graph_is_callable.rs`

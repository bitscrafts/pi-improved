# Spec 270: a run journals each call before any node runs, and `run trace` shows it

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composition, auditability (Opus context review 2026-10-24, Q7). Depends on spec 269 (`CallRecord`, A134): queue it after A134.

Spec 269 makes the loader return a `CallRecord` per call and prints it in `pii graph explain`. A RUN still does not say it was composed: the journal holds nodes named `docs__in`, `docs__fmt` and no record of which graph, at which digest, with which `needs`. This spec puts the record into the run's own journal, before any node executes, and renders it in `pii run trace`, using the mechanism `[[attach]]` weaving already uses.

The precedent (read 2026-10-25): `crates/pii-graph/src/run/mod.rs:102-115` — `run` appends `RunEvent::Started`, then for every node whose OWN cfg carries `woven-from` appends `RunEvent::NodeObserved { node, kind: "woven", fields: { "woven-from": origin } }`, then `jr.flush()`, all before the first node. The daemon reaches the same function (`crates/pii-server/src/engine/runs.rs:152` `pii_graph::run::run`), as does the CLI, and the reader is `crates/pii-cli/src/cmd_trace/analysis.rs:121`. Nothing was needed in the engine for `woven`; nothing is needed here. The mark lives IN the loaded graph (a cfg key on the call's head node), so every runner and a resume see the same thing, and no loader return value has to be threaded through the daemon.

## Verified Premises
- **CONFIRMED** (read) `run/mod.rs:103-115` as above; `RunEvent::NodeObserved { node, kind: String, fields: Value }` (`crates/pii-abi/src/journal.rs:181`) takes any kind string. NO change to `RunEvent` is needed.
- **CONFIRMED** (read) `runs.rs:152` calls `pii_graph::run::run(...)`; `woven_graph_can_start.rs` (pii-server test) reads `kind == "woven"` from a daemon run's journal, so a mark in a node's cfg reaches a daemon journal.
- **CONFIRMED** (read) `grep` for an unknown-cfg-key rejection in `validate.rs` and `toml_load.rs` finds none; `woven-from` is itself an arbitrary cfg key on arbitrary node kinds.
- **CONFIRMED** (read) `analysis.rs:121-135`: `kind == "woven"` and `"worker.start.refused"` have their own arms; every other `NodeObserved` goes to `observe_call(...)`, the agent-call path.
- **CONFIRMED** (read) `call.rs` is 446 lines against the 450 gate and spec 269 may add up to 3. THIS spec may add NONE to `call.rs`; if the head node is built in `call.rs` (`splice`, around line 257), move that one construction into `call_needs.rs` (the chain builder lives there) so `call.rs` does not grow, and say so.
- **INFERRED, MEASURE FIRST** that `observe_call` ignores a kind it does not know: read it before adding the arm, and make the new arm come BEFORE the generic one regardless. STOP AND ASK only if `observe_call` panics on an unknown kind.
- **NOT CLAIMED** that resume re-journals the mark: `woven` is journalled by `run` at start only, and so is this.

## Requirements
### Functional
- **R1. The mark.** The head node `<call>__in` of every expanded call carries a cfg key `call-from` whose value is a table with the fields of that call's `CallRecord` (spec 269): `graph`, `sha256` (the FULL 64 hex), `caps` (array of names), `needs` (array, or ABSENT when the sub-graph declares none), `exit`, `input_keys` (array), `parent` (the parent call's id, or ABSENT). Built from the verified record, not from the host text. `call_from_fields(&CallRecord) -> serde_json::Value` lives in `call_record.rs`.
- **R2. The journal.** In `run/mod.rs`, in the same loop that journals `woven`, a node whose cfg has an object under `call-from` appends `RunEvent::NodeObserved { node: <that node's id>, kind: "graph.call", fields: <that object> }`, before `jr.flush()` and before any node runs, in spec node order (an outer call before the call it holds). The `woven` behaviour and its test stay as they are. The loop's added code lives in a NEW function in a NEW file `crates/pii-graph/src/run/call_events.rs` called from `run/mod.rs` (one call, at most 4 added lines in `mod.rs`).
- **R3. The trace shows it.** `pii run trace <run>` prints, for each `graph.call` observation, one line in the SAME shape as spec 269's explain line: `call <call> -> graph <graph> sha256 <first 12 hex> caps [<names, comma>] needs [<keys, comma>|none] exit <exit>` plus ` (in <parent>)` for a nested call, where `<call>` is the head's id with its `__in` suffix removed. A new arm in `analysis.rs` BEFORE the generic `NodeObserved` arm; it must not feed `observe_call`. A run with no call prints no such line.
- **R4. Tests, with controls.** New `crates/pii-graph/tests/run_journals_calls.rs`, `crates/pii-server/tests/daemon_journals_calls.rs`, `crates/pii-cli/tests/trace_prints_calls.rs` (new files; edit no existing test). Library (runs the expanded graph in process, reads the journal): (a) one call journals one `graph.call` for the head node with the record's fields, `sha256` equal to `sha256_hex` of the sub-graph file; (b) the event precedes every `NodeStarted` (compare sequence numbers); (c) a nested call journals the outer before the inner and the inner has `parent`; (d) CONTROL: a graph with no call journals no `graph.call`; (e) CONTROL: a graph with an attach and no call still journals `woven` and no `graph.call`; (f) a sub-graph without `needs` has NO `needs` field, one with `needs = []` has `[]`; (g) `needs` and `parent` fields are absent, not null, when not applicable. Daemon: (h) the same host started through the engine journals the same event (read the journal as `woven_graph_can_start.rs` does). CLI: (i) `run trace` on a composed run prints the exact R3 line; (j) a nested call prints `(in <parent>)`; (k) CONTROL: `run trace` on a run with no call prints no line starting `call `.

### Non-Functional
- Every file under the 450-line gate; `call.rs` must not grow. No new dependency. Do not change `RunEvent`, `crates/pii-abi/src/journal.rs`, the engine, or any node kind.
- Do not edit `run/mod.rs`'s `woven` loop (add the new call beside it), `woven_graph_can_start.rs`, `trace_names_woven_nodes.rs`, or any test file of specs 255 to 269.

## TDD Contract
| Test | Expected |
|---|---|
| `a_call_journals_one_record` | one `graph.call`, fields = record, full sha256 == `sha256_hex(file)` |
| `the_record_precedes_every_node` | its seq < every `NodeStarted` seq |
| `a_nested_call_journals_outer_first` | outer then inner, inner has `parent` |
| `CONTROL_no_call_no_event` | no `graph.call` |
| `CONTROL_an_attach_alone_still_journals_woven` | `woven` present, `graph.call` absent |
| `needs_absent_versus_empty` | field absent vs `[]` |
| `absent_fields_are_not_null` | `needs`/`parent` keys absent |
| `the_daemon_journals_the_same_event` | same fields via the engine |
| `trace_prints_the_call_line` | exact R3 text |
| `trace_prints_the_parent_of_a_nested_call` | `(in <parent>)` |
| `CONTROL_trace_prints_no_call_line_without_a_call` | none |

**The requirement a plausible implementation satisfies only in appearance:** journalling in the CLI's `graph explain` path or in the loader instead of in `run`. The event then exists for a person who ran `explain` and not for a run the daemon started or resumed. Test (h) fails it, because the daemon never calls `explain`.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test run_journals_calls 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-server --test daemon_journals_calls 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-cli --test trace_prints_calls 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test -f crates/pii-graph/src/run/call_events.rs`
- [ ] `grep -q "graph.call" crates/pii-graph/src/run/call_events.rs`
- [ ] `grep -q "fn call_from_fields" crates/pii-graph/src/fragment/call_record.rs`
- [ ] `cargo test -q -p pii-server --test woven_graph_can_start 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-cli --test trace_names_woven_nodes 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-graph --test call_records 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `git diff --quiet -- crates/pii-abi/src/journal.rs crates/pii-server/tests/woven_graph_can_start.rs crates/pii-cli/tests/trace_names_woven_nodes.rs`
- [ ] `test "$(wc -l < crates/pii-graph/src/fragment/call.rs)" -le 450`
- [ ] `cargo clippy -q -p pii-graph -p pii-server -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT touch `RunEvent`, `crates/pii-abi/src/journal.rs`, the engine or any node kind. Do NOT edit the `woven` loop or the test files named above. Do NOT let `call.rs` exceed 450 lines. Do NOT put the mark in a value (`value`/`expr`) of the head: a transform evaluates those. Do NOT commit.

## Follow-ups (not this spec)
The digest of the expanded `GraphSpec` beside each record; the per-key source (`in` or host input) and a value hash for each key, journalled by the `require` node; resume re-reading the record.

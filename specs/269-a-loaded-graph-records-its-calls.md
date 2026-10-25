# Spec 269: a loaded graph says which graphs it called — the call record, and `graph explain` prints it

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composition, auditability (Opus context review 2026-10-24, Q7: "journal one `graph.call` event per call")

After spec 255 expands a `call` node the result is one flat `GraphSpec`; nothing in it says that `docs__fmt` came from graph `note`, at which sha256, with which caps and which `needs`. `Woven` (`crates/pii-graph/src/fragment/weave.rs:36`) carries `sequence: Vec<Inlined>` for `[[attach]]` weaving and nothing for calls, so the three consumers of the loader (`pii graph explain`/`check`/`show` in `crates/pii-cli`, the daemon's run start, resume) cannot tell a composed run from a plain one. This spec adds the RECORD at load and prints it in `pii graph explain`. Journalling it into the run (a `NodeObserved { kind = "graph.call" }`) is the NEXT spec, because the site where the engine appends the first event of a run has not been measured yet (see Follow-ups); this spec does not touch the engine, the journal or `RunEvent`.

## Verified Premises
- **CONFIRMED** (read, 2026-10-25) `weave.rs:36-41`: `pub struct Woven { pub spec: GraphSpec, pub sequence: Vec<Inlined> }`; `load_host_with_graphs` (`weave.rs:105-122`) calls `expand_host(...)` and, for a graph with no `attach`, returns `Woven { spec, sequence: Vec::new() }`. The `expand_host` result is `Option<String>` (the expanded text), so today no record survives.
- **CONFIRMED** (read) `call.rs` computes, per call, everything a record needs: the call id, the graph name (`graph`), the pinned digest (`sha256`, verified against `sha256_hex` of the file bytes), the granted caps (`caps`), the sub-graph's `needs` (`call_needs::parse`), the `exit` node, and the mapped `in` keys.
- **CONFIRMED** (read) `crates/pii-abi/src/journal.rs:85` `RunEvent` has `NodeObserved { node, kind, fields }` (spec 217) holding `coder.*` observations; it is the natural carrier for the journal spec, and this spec does not change the enum.
- **CONFIRMED** (read) `Woven` is constructed in `weave.rs` (two sites) and read at `cmd_graph_installed.rs:270` and `main.rs:82` (`woven.spec, woven.sequence`).
- **NOT CLAIMED / INFERRED, MEASURE FIRST** whether other code constructs `Woven { .. }` by struct literal (a new field would then break it): run `grep -rn "Woven {" crates --include=*.rs` before editing; if any site outside `weave.rs` builds one, add the field there too and say so in the report. STOP AND ASK only if a site is in a file you may not edit.
- **NOT CLAIMED** that `graph explain` prints `sequence` today; read `crates/pii-cli/src/main.rs` around line 76-140 first and follow its existing print style.

## Requirements
### Functional
- **R1. A call record.** New `pub struct CallRecord { pub call: String, pub graph: String, pub sha256: String, pub caps: Vec<String>, pub needs: Option<Vec<String>>, pub exit: String, pub input_keys: Vec<String>, pub parent: Option<String> }` in a NEW file `crates/pii-graph/src/fragment/call_record.rs`, re-exported from `fragment` and the crate root next to `Woven`. `parent` is the id of the call that contains this one for a nested call, `None` for a top-level one. `Debug, Clone, PartialEq, Eq`.
- **R2. The loader returns them.** `Woven` gains `pub calls: Vec<CallRecord>`, in the order the calls were expanded (host order, a nested call after the call that holds it). `expand_host` returns the records beside the expanded text (change its return to a small struct or a tuple; its callers are inside `fragment`). A graph with no call has `calls == []`; a graph with calls but no `attach` returns them; a graph with both returns them too (the attach path must not drop them). The expanded graph and every existing error stay byte-for-byte what spec 261/267 produce.
- **R3. `graph explain` prints them.** After the existing graph output, `pii graph explain` prints one line per record, in this exact shape: `call <call> -> graph <graph> sha256 <first 12 hex> caps [<names, comma>] needs [<keys, comma>|none] exit <exit>` with ` (in <parent>)` appended for a nested call. A graph with no call prints no `call` line and nothing else new.
- **R4. Tests, with controls.** New `crates/pii-graph/tests/call_records.rs` (library) and `crates/pii-cli/tests/graph_explain_prints_calls.rs` (runs the BUILT binary, like `graph_commands_expand_calls.rs`; do NOT edit that file). Library: (a) a host with one call yields one record with the graph name, the sha256 equal to `sha256_hex` of the sub-graph file, caps, `needs`, `exit`, the mapped input keys, `parent == None`; (b) two calls yield two records in host order; (c) a nested call yields a record with `parent == Some(<outer call id>)` after the outer one; (d) CONTROL: a host with no call has `calls == []`; (e) CONTROL: a sub-graph with no `needs` has `needs == None`, and one with `needs = []` has `Some([])`; (f) a host with a call AND an `attach` returns the record (the attach path does not drop it); (g) the sha256 in the record is the one the pin verified: after a pin mismatch there is no record (the load fails). CLI: (h) `graph explain` on a host with one call prints exactly the R3 line for it; (i) a nested call prints `(in <parent>)`; (j) CONTROL: a graph with no call prints no line starting `call `.

### Non-Functional
- Keep every file under the 450-line gate (`call.rs` is at 446: add NO more than 3 lines to it; the record is built in `call_record.rs` and `call_needs.rs`). No new dependency. Do not change what `expand_host` expands, any error text, or `RunEvent`.
- Do not edit `call_needs.rs`'s load-time checks, `crates/pii-graph/tests/call_inlines_a_graph.rs`, `call_needs.rs` (test), `call_requires_needs.rs`, `call_needs_holder.rs`, `graph_commands_expand_calls.rs`.

## TDD Contract
| Test | Expected |
|---|---|
| `one_call_one_record` | graph, sha256 == `sha256_hex(file)`, caps, needs, exit, input keys, `parent` None |
| `two_calls_in_host_order` | two records in order |
| `a_nested_call_names_its_parent` | `parent == Some(outer)` after the outer record |
| `CONTROL_no_call_no_records` | `calls == []` |
| `CONTROL_needs_none_versus_empty` | `None` vs `Some([])` |
| `a_call_with_an_attach_keeps_its_record` | record present |
| `a_pin_mismatch_yields_no_record` | load error, no record |
| `explain_prints_the_call_line` | exact R3 text |
| `explain_prints_the_parent_of_a_nested_call` | `(in <parent>)` |
| `CONTROL_explain_prints_no_call_line_without_a_call` | no line starting `call ` |

**The requirement a plausible implementation satisfies only in appearance:** filling the record from the host TOML's `call` table instead of from what the loader verified. The `sha256` the host WRITES and the digest of the file it READ are equal only when the pin held; a record taken from the host text would show a digest nothing checked. Test (a) compares against `sha256_hex` of the file, and (g) shows a mismatch yields no record.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test call_records 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-cli --test graph_explain_prints_calls 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test -f crates/pii-graph/src/fragment/call_record.rs`
- [ ] `grep -q "pub calls" crates/pii-graph/src/fragment/weave.rs`
- [ ] `cargo test -q -p pii-graph --test call_needs 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-graph --test call_requires_needs 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-cli --test graph_commands_expand_calls 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `git diff --quiet -- crates/pii-graph/tests/call_inlines_a_graph.rs crates/pii-graph/tests/call_needs.rs crates/pii-graph/tests/call_requires_needs.rs crates/pii-graph/tests/call_needs_holder.rs crates/pii-cli/tests/graph_commands_expand_calls.rs crates/pii-abi/src/journal.rs`
- [ ] `cargo clippy -q -p pii-graph -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT touch `RunEvent`, `crates/pii-abi/src/journal.rs`, the engine, or any node kind. Do NOT change an expansion result or an error text. Do NOT edit the test files named above. Do NOT commit.

## Follow-ups (not this spec)
1. Journal each record as `NodeObserved { node = "<call>__in", kind = "graph.call", fields = { graph, sha256, caps, needs, exit, input_keys, parent } }` at the engine's run start: first MEASURE where the engine appends the run's first events (`crates/pii-server/src/engine/runs.rs`, `crates/pii-server/src/lib.rs:434`) and how resume replays an observation.
2. `pii run explain` rendering that line; the digest of the expanded `GraphSpec` beside it.
3. Per-key source (`in` versus host input) and a value hash per key at the `require` node.

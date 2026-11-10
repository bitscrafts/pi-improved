# pi-improved — spec-42: failover reason on the operator surface (A5)

- **Project**: `<repo>`
- **Priority**: HIGH — Track A5 of research doc 025 §5, the last item in
  Track A before Phase 5.2 structural work begins. Per 025's own risk
  section: *"Losing the reason. A silent switch makes cost off-peak/
  failover analysis impossible; A5 is mandatory, not polish."*
- **Depends on**: spec-40's `fail` port / `FailurePayload` and spec-41's
  `graphs/starter/failover.toml`, both already landed in the working tree.

## Overview

025 defines A5's scope precisely: *"confirm the reason reaches the
journal/payload a human can read; document the ladder's journal
signature."* Two parts, not one:

1. **Confirm** — the `fail` port's `FailurePayload{status,reason,provider}`
   already lands in the journal for free, because `AgentNode::run` returns
   it as `Outcome::Next{port:"fail", value}` and the engine's existing
   `RunEvent::NodeFinished{port,value,value_hash,dur_ms}` journals every
   `Outcome::Next` unconditionally (spec-40's own T6 already proved this at
   the unit level — one node, one journal read). What is NOT yet proven is
   that a human actually reading `pii graph trace`'s output — the CLI
   surface, not a Rust test — can see the reason without writing a `jq`
   filter first. This spec adds the missing operator-facing convenience,
   not new plumbing.
2. **Document** — `_docs/06-research/006-pii-graph-engine.md` §7 already
   states the generic `NodeFinished{port,value,...}` journal shape but says
   nothing about what a `fail`-port `value` specifically contains. That
   documentation gap is real: an operator debugging a failover run today
   has no doc pointing them at `value.reason`/`value.status`/
   `value.provider`.

### What this is not

This is not a new RPC method, a new store table, a new UI, or a change to
how failures are classified or routed — A2/A3/A4 already built all of
that. A5 is the literal narrow scope 025 gives it: a CLI convenience to
surface what already exists, plus the documentation of its shape.

## Requirements

### Functional

- **R1 (`pii graph trace --reason <node>` query)**: `cmd_trace.rs` gains a
  fifth query variant, following the exact existing pattern of
  `--node-status`/`--count-finished` (one new `Query` enum variant, one new
  arg-parse arm in `parse_query`, one new match arm in `answer`):
  ```rust
  Reason(String),
  ```
  parsed the same way as `NodeStatus`: `"--reason" => Ok(Query::Reason(next_value(args, "--reason", TRACE_USAGE)?))`.
  Behavior: find the LATEST `NodeFinished { node: n, port, value, .. }`
  event for the given node (same "last one wins" convention
  `node_status`/`last_window` already use — do not invent a different
  selection rule). If `port != "fail"` OR no such event exists, print the
  literal string `no-failure` and the query still exits 0 (mirrors
  `node_status`'s own "pending" default for "nothing to report" — a missing
  reason is not a CLI error, it is the correct answer for a node that never
  failed). If a `fail`-port event exists, deserialize `value` as
  `FailurePayload` and print it as one line of JSON: `{"status":...,
  "reason":"...","provider":"..."}` (i.e. `serde_json::to_string(&payload)`,
  the SAME wire shape the journal already stores — this command is a
  convenience read, not a reformat).
- **R2 (usage string updated)**: `TRACE_USAGE` gains
  `| --reason <node>` in its query list, matching the existing
  `--node-status <node>` phrasing exactly (same bracket/pipe style).
- **R3 (doc 006 §7 gains the journal signature)**: append one new
  subsection immediately after the existing journal-events list (after the
  line ending `... Finished { terminal, output_hash }`.), titled `#### 7.1
  Failover reason on the fail port (A5)`, documenting:
  - The `fail` port only ever appears on an `agent`-kind node
    (`AgentNode::run`'s `route_provider_failure`, spec-40).
  - Its `NodeFinished.value` shape: `FailurePayload{status:Option<u16>,
    reason:FailoverReason,provider:String}` — link to
    `crates/pii-abi/src/node.rs` (`FailurePayload`) and
    `crates/pii-abi/src/failover.rs` (`FailoverReason`, `classify`,
    `Recovery`) as the source of truth for the field types, not a copy of
    every enum variant (enums drift; a source-file pointer does not).
  - How to read it operationally: `pii graph trace <run_id> --db <path>
    --reason <node>` (R1's new query) is the one-line way; the general
    dump (`pii graph trace <run_id> --db <path>`, no query) shows it inline
    as part of the full `NodeFinished` event for anyone who wants the raw
    journal.
  - One concrete example using `graphs/starter/failover.toml`'s own
    `primary`/`secondary` node ids (the real, checked-in graph from A4 —
    not an invented example), e.g. showing what
    `pii graph trace <run> --db .pidag/pidag.redb --reason primary` prints
    when `primary` fails with a classified `Overloaded` reason.
- **R4 (no change to classification, routing, or the `fail` port itself)**:
  this spec touches zero files under `crates/pii-server/src/agent.rs`,
  `crates/pii-abi/src/{node,failover,error}.rs`, or
  `graphs/starter/failover.toml` — those are A2/A3/A4's landed work,
  frozen. If reading the journal reveals the reason is NOT actually present
  as A2/A3 claim, STOP and report — do not "fix" A2/A3 inside an A5 spec.

### Non-Functional

- **N1**: `cmd_trace.rs` stays ≤ 450 lines including the new query (it was
  well under before; R1 adds roughly 15–20 lines).
- **N2**: no new dependency in `pii-cli`.
- **N3**: no `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!` in
  `cmd_trace.rs` — R1's implementation must handle "no such node" /
  "malformed value" the same defensive way `node_status`/`last_window`
  already do (silent fallback to the "nothing to report" case, never a
  panic on unexpected journal content).
- **N4**: quality gate stays green; test count only goes up.

## Architecture

```
crates/pii-cli/src/cmd_trace.rs
  Query::Reason(String)                                          (R1)
  parse_query: "--reason" -> Query::Reason(next_value(...))       (R1)
  answer: Query::Reason(node) => println!("{}", reason_for(&journal, &node))  (R1)
  reason_for(journal, node) -> String                             (R1)
    - find latest NodeFinished for `node`
    - port != "fail" or none found -> "no-failure"
    - port == "fail" -> serde_json::to_string(&value) (the FailurePayload, as-is)

_docs/06-research/006-pii-graph-engine.md
  §7.1 Failover reason on the fail port (A5)                      (R3)
    - shape, source-of-truth pointers, one real example
```

Key decision: **`reason_for` reuses the exact "last event wins" convention
`node_status` and `last_window` already establish in this file** — A5 does
not introduce a second selection rule for "which event counts" (e.g. first
vs. last occurrence). A node in a `retry` loop (per `graphs/starter/
failover.toml`'s own `retry_primary` back-edge) may have multiple
`NodeFinished` events for the same node id across attempts; "the latest
one" is the only answer consistent with what an operator watching a live
run actually wants to know — the CURRENT/most-recent disposition, not the
first attempt's stale one.

## TDD Contract

New tests in `crates/pii-cli`'s existing test conventions — check whether
`cmd_trace.rs` has inline `#[cfg(test)]` tests today or is tested via
`crates/pii-cli/tests/*.rs` integration tests, and follow whichever pattern
already exists (do not introduce a second harness style for one new query).

| Test | Setup | Expects |
|---|---|---|
| T1 `trace_reason_prints_no_failure_for_never_failed_node` | a journal with only `NodeFinished{port:"ok",...}` for node `n` | `pii graph trace ... --reason n` prints exactly `no-failure` |
| T2 `trace_reason_prints_failure_payload_for_fail_port` | a journal with `NodeFinished{port:"fail", value: {"status":503,"reason":"overloaded","provider":"ollama"}}` for node `n` | prints that exact JSON object (byte-identical to what's stored, not reformatted) |
| T3 `trace_reason_uses_latest_event_across_retries` | a journal with TWO `NodeFinished` events for node `n` — first `port:"fail"` (`reason:"overloaded"`), second `port:"ok"` (the retry succeeded) | prints `no-failure` (the LATEST event is `ok`, not the earlier `fail` — proves the "last wins" rule, the case a naive "any fail event ever" implementation gets wrong) |
| T4 `trace_reason_missing_node_prints_no_failure` | a journal that never mentions node `missing` at all | prints `no-failure`, exit 0 (not an error — R1's explicit "no such event" case) |

T3 is the acceptance test: a plausible wrong implementation returns the
FIRST `fail` event it finds (or any `fail` event ever, ignoring order),
which would report a node as currently-failed when it actually recovered
on retry — exactly the kind of stale-reason bug 025's risk section warns
against ("losing the reason" cuts both ways: reporting a stale one is as
bad as reporting none).

## Exit Criteria

- [ ] `cargo test -p pii-cli --quiet trace_reason_prints_no_failure_for_never_failed_node 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R1 (T1)
- [ ] `cargo test -p pii-cli --quiet trace_reason_prints_failure_payload_for_fail_port 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R1 (T2)
- [ ] `cargo test -p pii-cli --quiet trace_reason_uses_latest_event_across_retries 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R1 (T3, the acceptance test)
- [ ] `cargo test -p pii-cli --quiet trace_reason_missing_node_prints_no_failure 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R1 (T4)
- [ ] `grep -q -- "--reason" crates/pii-cli/src/cmd_trace.rs` — R1/R2 wired
- [ ] `grep -q "Failover reason on the fail port" _docs/06-research/006-pii-graph-engine.md` — R3
- [ ] `grep -q "primary" _docs/06-research/006-pii-graph-engine.md` — R3's example uses the real A4 graph, not an invented one
- [ ] `[ "$(wc -l < crates/pii-cli/src/cmd_trace.rs)" -le 450 ]` — N1
- [ ] `! grep -qE "unwrap\(|expect\(|panic!|todo!|unimplemented!" crates/pii-cli/src/cmd_trace.rs` — N3
- [ ] `cargo test --workspace --lib --bins --tests` — 0 failures (N4)
- [ ] `cargo test --workspace --doc` — 0 failures (N4)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N4)
- [ ] `cargo fmt --all --check` — clean (N4)
- [ ] `cargo run -q -p xtask -- silent-noop-check` — green (N4)

## Guardrails

- **G1 — do NOT edit this spec.** Report a wrong premise instead of coding
  around it.
- **G2 — do NOT commit.** Leave the tree for orchestrator review.
- **G3 — do NOT touch `crates/pii-server/src/agent.rs`,
  `crates/pii-abi/src/{node,failover,error}.rs`, or
  `graphs/starter/failover.toml`** (R4) — those are frozen, landed A2/A3/A4
  work.
- **G4 — do NOT weaken, skip, or delete an existing `cmd_trace.rs` test** —
  every existing query (`--node-status`, `--count-finished`,
  `--max-batch-width`, `--terminal`, `--output`, `--overlapped`) must keep
  passing unmodified.
- **G5 — do NOT touch `specs/`.**
- **G6 — never panic, never guess.** No `unwrap`/`expect`/`panic!`/indexing
  in `cmd_trace.rs` (existing convention: the file already has zero of
  these outside tests — keep it that way).
- **G7 — no AI-assistant or model-vendor names** in code comments, docs, or
  any commit message (project hard rule).

## Files to Modify

- `crates/pii-cli/src/cmd_trace.rs` — `Query::Reason` (R1), usage string
  (R2), tests (T1–T4).
- `_docs/06-research/006-pii-graph-engine.md` — new §7.1 (R3).

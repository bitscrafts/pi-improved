# pi-improved — spec-41: failover ladder graph (A4)

- **Project**: `<repo>`
- **Priority**: HIGH — Track A4 of research doc 026 §5.1. Depends on A3
  (spec 40, `AgentNode`'s `fail` port + `FailurePayload{status,reason,
  provider}`), already landed in the working tree.
- **Depends on**: spec-40's `fail` port (`crates/pii-server/src/agent.rs`)
  and spec-39's `FailoverReason`/`Recovery` (`crates/pii-abi/src/
  failover.rs`). Do not re-derive either; consume them as they exist.

## Overview

A2 gave `PiiError::Provider` a classifiable reason. A3 gave `AgentNode` a
`fail` port carrying that reason as `{status, reason, provider}` JSON. Until
now nothing in `graphs/` has ever consumed that port — there is no example
graph demonstrating what a failover chain looks like, and no test proving
the `route`+`retry` machinery (both already exist in `pii-graph`, used
today by `graphs/starter/heal-fix.toml` for an unrelated purpose) compose
correctly against `fail`'s new payload shape.

This spec is deliberately an **example graph plus a graph-level test**, not
a new node kind or new engine feature — 026's acceptance line names exactly
this: `graphs/starter/failover.toml`, sequential model nodes, a `route` on
`$.payload.reason`, a compress branch, and a `retry`-shaped back-edge for
`RetrySame`. Everything it needs (the `route` node, per-node `loop_budget`,
data edges) is already implemented and already proven by `heal-fix.toml`'s
own `route_probe`/`retry` pattern (`graphs/starter/heal-fix.toml:26-32,
77-80,150-160`) — this spec follows that exact precedent, adapted to
`fail`'s payload instead of a shell exit code.

## Requirements

### Functional

- **R1 (the graph file)**: create `graphs/starter/failover.toml` — a
  checked-in example graph (same directory and convention as `heal-fix.toml`
  /`heal-watch.toml`/`verify-fix.toml`, not a test-tmpdir artifact). Shape,
  following 026's own description literally:
  ```
  primary (agent) --ok--> done
  primary --fail--> route_failure (route on $.payload.reason)
    reason == "rate_limit"      --> retry_primary  (loop_budget on primary)
    reason == "overloaded"      --> retry_primary
    reason == "server_error"    --> retry_primary
    reason == "timeout"         --> retry_primary
    reason == "context_overflow" --> compress   (dead end — Compress never advances)
    reason == "payload_too_large" --> compress  (dead end — Compress never advances)
    (fallback, everything else: auth/billing/upstream_rate_limit/
     model_not_found/unknown) --> secondary (agent, a second model)
  secondary --ok--> done
  secondary --fail--> route_failure_2 (same shape, one level: on secondary's
    exhaustion the chain ends — 026 does not ask for an unbounded ladder,
    "sequential model nodes" plural means exactly two: primary, secondary)
  route_failure_2 fallback --> exhausted (gate/terminal — chain has nothing
    left to try)
  ```
  The `route`'s `on` expression is `$.payload.reason` (matches
  `FailurePayload`'s serde field name, `snake_case` values like
  `"rate_limit"`, `"context_overflow"` — verify against
  `crates/pii-abi/src/failover.rs`'s `#[serde(rename_all = "snake_case")]`
  on `FailoverReason`, do not guess the wire casing).
  `retry_primary` is a node the same shape as `heal-fix.toml`'s own `retry`
  node (`kind = "transform"`, `expr = "$.payload"`, a data edge back to
  `primary`) — `primary` gets a `loop_budget` (2, giving one retry attempt
  before falling through) exactly as `heal-fix.toml`'s `fix` node does
  (`loop_budget = 2`). `compress` is a `transform` node that is
  **deliberately a dead end** (no outbound edge) — R3 below explains why
  this is correct, not a bug.
- **R2 (`route` rules cover every `FailoverReason` variant)**: the
  `route_failure` node's `rules` array has one `{eq = "<snake_case
  variant>", port = "<target>"}` entry per `RetrySame`/`Compress` reason
  (`rate_limit`, `overloaded`, `server_error`, `timeout` → `retry`;
  `context_overflow`, `payload_too_large` → `compress`), plus exactly one
  fallback rule with no `eq` (matches `RouteNode`'s own fallback semantics,
  `crates/pii-graph/src/nodes.rs:112-117`) routing every `NextProvider`
  reason (`auth`, `billing`, `upstream_rate_limit`, `model_not_found`,
  `unknown`) to `secondary`. Do not enumerate the `NextProvider` reasons as
  individual `eq` rules — the fallback IS the `NextProvider` semantics
  (anything not explicitly retried or compressed advances the chain), and
  enumerating them individually would silently break the moment
  `FailoverReason` gains a twelfth variant (`ALL`'s own doc comment: "every
  variant, in declaration order" — this graph must not need updating every
  time that array grows unless the new variant needs `RetrySame`/`Compress`
  handling).
- **R3 (why `compress` is a dead end)**: per spec-39/40's own recovery
  taxonomy, `Compress` "never advances the chain" (`failover.rs:31,33`
  doc comments, already landed) — a `ContextOverflow`/`PayloadTooLarge`
  failure is not fixed by trying a different provider (the payload is still
  too large) or by retrying the same one (it will overflow again). The
  *correct* recovery is compacting the conversation/payload and re-entering
  the graph from `primary`, but a compaction implementation is out of scope
  for A4 (026 does not ask for it here — `AgentNode` already has its own
  internal `Compaction` mechanism for token-budget exhaustion, wiring
  `Compress`-classified failures into it is a follow-on, not this spec).
  `compress` exists in this graph as the **named landing point** 026's
  acceptance test checks for ("never advances the chain" — i.e. reaches
  `compress` and stops, does not reach `done`) — it is not a placeholder to
  "fill in later" within this spec's scope.
- **R4 (`pii graph validate` passes)**: the file must pass
  `pii graph validate graphs/starter/failover.toml` with exit 0 — this is
  026's own literal acceptance line. Validation checks (per
  `crates/pii-graph/src/validate.rs`, already implemented, not modified by
  this spec): every node kind is registered, every edge references an
  existing node/port, every cycle (the `retry_primary → primary` back-edge)
  has a `loop_budget` on its re-entered node.
- **R5 (graph-level integration test)**: a new test in
  `crates/pii-server/tests/` (following the existing `Engine`/fixture
  pattern used by `tests/model_routing.rs`/`tests/fail_port.rs` — spec 40's
  own new test file is the freshest precedent for how to force a
  `ScriptedProvider`-style failure through a real `Engine` run) that:
  1. Loads `graphs/starter/failover.toml` (the real checked-in file, not a
     copy — the test's whole point is proving *this* file behaves as
     described, per FINDINGS.md's own rule: assert on what the consumer
     received against the real artifact, not a fixture standing in for it).
  2. Runs it with a provider forced to fail with a `RetrySame`-classified
     reason (e.g. `ContextOverflow`... no — `ContextOverflow` is
     `Compress`; use `Overloaded`/503, which is `RetrySame`) on `primary`'s
     first call, succeeding on the retry. Asserts the run reaches `done`
     and the journal shows `primary` was entered twice (the retry fired).
  3. Runs it again with `primary` failing with a `NextProvider`-classified
     reason (e.g. `Auth`/401) and `secondary` succeeding. Asserts the run
     reaches `done` via `secondary`, and the journal shows `primary` was
     entered exactly once (no wasted retry against a `NextProvider`
     failure — the chain advanced, it didn't retry a doomed provider).
  4. Runs it a third time with `primary` failing with a
     `Compress`-classified reason (`ContextOverflow`). Asserts the run
     reaches `compress` and does **not** reach `done` — this is 026's
     "never advances the chain" acceptance line made concrete.

### Non-Functional

- **N1**: no engine/`pii-graph` code changes — `route`, `loop_budget`, data
  edges are all pre-existing and unmodified by this spec. If validating the
  graph reveals a genuine engine gap (something 026's description implies
  that `pii-graph` cannot actually do), STOP and report it rather than
  patching the engine inside a spec scoped to an example graph + test.
- **N2**: the new test file stays ≤ 450 lines.
- **N3**: no `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!` in the new
  test file outside the standard `#[allow(clippy::unwrap_used, ...)]`
  test-module convention already used by every other `tests/*.rs` file in
  this crate (see `zz_verify_d3_roundtrip.rs`'s header for the exact
  attribute — though that file itself was removed as throwaway scratch,
  its attribute line is the pattern; `tests/fail_port.rs` from spec 40 is
  the live, current example to copy from).
- **N4**: quality gate stays green; test count only goes up.

## Architecture

```
graphs/starter/failover.toml   (new, checked-in — R1/R2/R3)
  primary (agent) --ok--> done
  primary --fail--> route_failure
    rate_limit/overloaded/server_error/timeout --> retry_primary --> primary (loop_budget=2)
    context_overflow/payload_too_large --> compress  (dead end, R3)
    fallback (auth/billing/upstream_rate_limit/model_not_found/unknown) --> secondary
  secondary (agent) --ok--> done
  secondary --fail--> route_failure_2
    same RetrySame/Compress rules --> retry_secondary / compress
    fallback --> exhausted (terminal — nothing left to try)

crates/pii-server/tests/failover_graph.rs   (new — R5)
  loads the real graphs/starter/failover.toml
  T1: RetrySame reason -> retry fires -> reaches done via primary
  T2: NextProvider reason -> no retry -> reaches done via secondary
  T3: Compress reason -> reaches compress, never reaches done
```

Key decision: **this spec does not implement compaction.** `Compress`
routing to a dead-end node is the correct, complete behavior 026 asks A4 to
demonstrate — "never advances the chain" is the requirement, not an
oversight to patch. A follow-on spec (not numbered here — out of Track A's
own scope per the 026 plan) would wire `compress` into `AgentNode`'s
existing internal `Compaction` struct and loop back to `primary`; doing
that inside A4 would silently expand this spec beyond what 026 asked for
and beyond what T3 tests for.

## TDD Contract

| Test | Setup | Expects |
|---|---|---|
| T1 `retry_same_reason_retries_primary_then_succeeds` | Load real `graphs/starter/failover.toml`; `primary`'s provider fails first call with `PiiError::Provider(ProviderFailure{status:Some(503),...})` (`Overloaded`→`RetrySame`), succeeds second call | Run reaches `done`; journal shows `primary` node entered twice (initial + retry) |
| T2 `next_provider_reason_advances_to_secondary_without_retry` | `primary`'s provider always fails with `PiiError::Provider(ProviderFailure{status:Some(401),...})` (`Auth`→`NextProvider`); `secondary`'s provider succeeds | Run reaches `done`; journal shows `primary` entered exactly once, `secondary` entered exactly once |
| T3 `compress_reason_never_advances_the_chain` | `primary`'s provider always fails with `PiiError::Provider(ProviderFailure{status:None, message:"llamacpp: prompt too long",...})` (`ContextOverflow`→`Compress`) | Run reaches `compress`; run does **not** reach `done`; journal has no `NodeFinished` for `secondary` |
| T4 `failover_toml_validates_clean` (shell-level, exit-criterion only) | `pii graph validate graphs/starter/failover.toml` | exit 0 — the literal 026 acceptance line |

T3 is the acceptance test that catches a plausible wrong implementation:
someone routing `compress` onward to `secondary` "to be safe" (defeating
the entire point of the `Compress` recovery strategy, which exists
precisely because trying a different provider does not fix an oversized
payload).

## Exit Criteria

- [ ] `pii graph validate graphs/starter/failover.toml` — exit 0 (R4, T4, the literal 026 acceptance line)
- [ ] `cargo test -p pii-server --quiet retry_same_reason_retries_primary_then_succeeds 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R5 (T1)
- [ ] `cargo test -p pii-server --quiet next_provider_reason_advances_to_secondary_without_retry 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R5 (T2)
- [ ] `cargo test -p pii-server --quiet compress_reason_never_advances_the_chain 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R5 (T3)
- [ ] `[ -f graphs/starter/failover.toml ]` — R1 exists
- [ ] `grep -q '"\$.payload.reason"' graphs/starter/failover.toml` — R1/R2 uses the right expr
- [ ] `[ "$(wc -l < crates/pii-server/tests/failover_graph.rs 2>/dev/null || echo 999)" -le 450 ]` — N2
- [ ] `cargo test --workspace --lib --bins --tests` — 0 failures (N4)
- [ ] `cargo test --workspace --doc` — 0 failures (N4)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N4)
- [ ] `cargo fmt --all --check` — clean (N4)
- [ ] `cargo run -q -p xtask -- silent-noop-check` — green (N4)

## Guardrails

- **G1 — do NOT edit this spec.** If `pii graph validate` reveals the
  described shape is impossible with the current engine, STOP and report;
  do not silently redesign the graph to route around an engine gap, and do
  not patch `pii-graph` to make an impossible shape possible — that is a
  different, larger spec.
- **G2 — do NOT commit.** Leave the tree for orchestrator review.
- **G3 — do NOT modify `crates/pii-graph`** (the `route`/`loop_budget`/
  data-edge machinery this spec uses is already complete per N1).
- **G4 — do NOT modify `graphs/starter/heal-fix.toml`** or any other
  existing graph — this spec adds one new file.
- **G5 — do NOT implement compaction/compress-then-retry.** `compress` is a
  deliberate dead end per R3; T3 is the regression guard for this.
- **G6 — do NOT touch `specs/`.**
- **G7 — never panic, never guess.** No `unwrap`/`expect`/`panic!`/indexing
  outside the standard test-module `#[allow(...)]` convention.
- **G8 — no AI-assistant or model-vendor names** in code comments, docs, or
  any commit message (project hard rule).

## Files to Modify

- `graphs/starter/failover.toml` — new (R1, R2, R3).
- `crates/pii-server/tests/failover_graph.rs` — new (R5, T1–T3).

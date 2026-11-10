# pi-improved — spec-40: failure-as-payload (A3)

- **Project**: `<repo>`
- **Priority**: HIGH — Track A3 of research doc 026 §5.1, Phase 5.2 (F-11 →
  structural debt ratchet) is explicitly deferred until this lands so the
  `agent.rs` split does not churn the file this spec must edit. **Must land
  before any `agent.rs` split spec.**
- **Depends on**: spec-39 (A2, `PiiError::Provider(ProviderFailure)` +
  `classify_provider`) — already implemented in the working tree at
  `crates/pii-abi/src/{error,failover}.rs`. Do not re-derive those types;
  import and use them as they exist today.

## Overview

`AgentNode::run` (`crates/pii-server/src/agent.rs:175-`) today has exactly
three output ports (`ok`, `err`, `budget`) and one retry policy applied
uniformly to *any* retryable `PiiError`: on `chat_streaming` failure, retry
once via `chat`; if that also fails, emit `Outcome::Next { port: "err", value:
{"error": e2.to_string()} }` (`:261-271`); if the *first* error was not
retryable, `return Err(e)` (`:272`) — a hard node failure the scheduler
treats as a crashed node, not a graph-routable outcome.

Since spec-39, `PiiError::Provider(ProviderFailure)` carries a classifiable
reason (`classify_provider`) whose `Recovery` is one of `RetrySame` (already
handled — the existing retry-once path), `NextProvider`, or `Compress`. The
latter two currently fall into `:272`'s `return Err(e)` — a `NextProvider`-
or `Compress`-classified provider failure crashes the node today instead of
giving a graph author anywhere to route it. There is no `graphs/starter/
failover.toml` yet (that's A4) because there is nothing in the graph model
for it to route *from* — this spec is what creates that seam.

This spec adds a fourth port, `fail`, carrying `{status, reason, provider}`
extracted from a `PiiError::Provider` specifically. Errors that are **not**
`PiiError::Provider` (tool errors, budget errors, internal bugs) are
untouched — they keep today's `err`/`budget`/hard-`Err` behavior exactly as
is. This is the "non-provider errors remain Err" half of the 026 acceptance
line; only the provider-failure path changes.

## Requirements

### Functional

- **R1 (new `fail` port)**: `AgentFactory::ports` (`agent.rs:111-117`) gains
  a fourth port:
  ```rust
  fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
      Ok(vec![
          PortName::from("ok"),
          PortName::from("err"),
          PortName::from("budget"),
          PortName::from("fail"),
      ])
  }
  ```
  Appended last (do not reorder `ok`/`err`/`budget` — `graphs/starter/*.toml`
  fixtures and existing tests reference them positionally in places; a
  reorder is an unforced compatibility break this spec does not need to
  make).
- **R2 (payload shape)**: a new type in `crates/pii-abi/src/node.rs`
  (same file as `Outcome`/`NodeInput` — the ABI's node-contract surface,
  not a new module):
  ```rust
  /// Payload emitted on a node's `fail` port for a classified provider
  /// failure (A3). Distinct from the free-form `{"error": String}` shape
  /// the `err` port already uses for non-provider failures.
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
  pub struct FailurePayload {
      /// HTTP status if the provider gave one (`None` for e.g. a local
      /// context-overflow with no HTTP leg, matching `ProviderFailure::status`).
      pub status: Option<u16>,
      /// The classified reason as its `Debug`/`serde` tag — reuses
      /// `FailoverReason`'s own derive, not a re-stringified copy.
      pub reason: FailoverReason,
      /// `ProviderFailure::provider` (e.g. `"ollama"`, `"llamacpp"`), passed
      /// through unchanged.
      pub provider: String,
  }
  ```
  `FailurePayload` derives the same trait set as sibling ABI types
  (`Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema`); it
  imports `crate::failover::FailoverReason` (already `pub` from spec-37).
  Serializes as a JSON object with keys `status`, `reason`, `provider` —
  `reason`'s own serde shape is whatever spec-37 already gave
  `FailoverReason` (do not add a `#[serde(...)]` attribute to the field;
  inherit the type's own representation).
- **R3 (`AgentNode::run` routes `PiiError::Provider` to `fail`, not `Err`)**:
  the two `Err` exits currently reachable for a provider call
  (`agent.rs:263-272`, both the retryable-then-fails-again arm at `:265-270`
  and the not-retryable arm at `:272`) are replaced with a single classify
  step. New logic (replacing lines 261-273):
  ```rust
  let resp = match self.provider.chat_streaming(ctx, &req, &mut on_delta) {
      Ok(r) => r,
      Err(e) if e.is_retryable() => match self.provider.chat(ctx, &req) {
          Ok(r) => r,
          Err(e2) => return Ok(route_provider_failure(e2)),
      },
      Err(e) => return Ok(route_provider_failure(e)),
  };
  ```
  with a new free function (private to `agent.rs`, placed near the other
  small helpers such as `resolve_model`):
  ```rust
  /// A2/A3 seam: `PiiError::Provider` is classified and routed to the
  /// `fail` port; every other error kind keeps the pre-A3 `err` port shape
  /// (`{"error": <Display>}`) — this function narrows, it does not widen,
  /// what counts as a routable failure.
  fn route_provider_failure(e: PiiError) -> Outcome {
      if let PiiError::Provider(ref pf) = e {
          let reason = pii_abi::classify_provider(pf);
          return Outcome::Next {
              port: PortName::from("fail"),
              value: serde_json::to_value(pii_abi::FailurePayload {
                  status: pf.status,
                  reason,
                  provider: pf.provider.clone(),
              })
              .unwrap_or_else(|_| serde_json::json!({"error": e.to_string()})),
          };
      }
      Outcome::Next {
          port: PortName::from("err"),
          value: serde_json::json!({ "error": e.to_string() }),
      }
  }
  ```
  Note the behavior change from today: previously a **non-retryable**
  `PiiError::Provider` (e.g. an `Auth`/`NextProvider` classification, which
  spec-39 made `is_retryable() == false`) hit `:272`'s hard `return Err(e)`
  — a crashed node. After R3 it is classified once and routed to `fail`
  without ever attempting the doomed retry (a `NextProvider`/`Compress`
  failure retrying the *same* provider is not going to succeed — R3 does
  not add a retry for these, it routes them directly, matching
  `is_retryable() == false`'s own meaning). A **retryable**
  `PiiError::Provider` (`RetrySame`) still gets the existing retry-once
  behavior — R3 only changes what happens once both attempts are exhausted;
  if the second attempt's error is `PiiError::Provider`, it now also routes
  to `fail` (previously it went to `err` unconditionally at `:265-270`,
  losing the classification). A non-`Provider` error at either arm keeps
  going to `err`, unchanged.
- **R4 (`unwrap_or_else` in R3 is a real fallback, not decoration)**: 
  `serde_json::to_value` on a struct made of `Option<u16>`/`FailoverReason`/
  `String` cannot practically fail, but this codebase's `deny(unwrap_used)`
  lint (`Cargo.toml` workspace lints) means a bare `.unwrap()` will not
  compile clean under `-D warnings`; the `unwrap_or_else` fallback exists to
  satisfy that lint honestly (degrade to the `err`-shaped payload with the
  original error's `Display`) rather than to handle a reachable case — do
  not write a test asserting the fallback path is reachable, because it is
  not; T5 in the TDD contract below asserts the *primary* path only.
- **R5 (re-export)**: `crates/pii-abi/src/lib.rs`'s existing re-export line
  gains `FailurePayload` alongside `Outcome`/`NodeInput` (same `pub use
  node::{...}` statement, do not add a second one).
- **R6 (journal visibility)**: no new journal-side change is required — the
  existing `RunEvent::NodeFinished { port, value, .. }` machinery
  (`crates/pii-graph`) already journals whatever port/value a node's
  `Outcome::Next` carries; routing to `"fail"` with a `FailurePayload` value
  is journaled for free by the mechanism that already journals `"budget"`
  today. Verify this with T6 (a journal-level test), do not add new
  journal-writing code.

### Non-Functional

- **N1**: `crates/pii-abi/src/node.rs` stays ≤ 450 lines including tests.
- **N2**: `crates/pii-server/src/agent.rs` — this spec's own diff must not
  push the file's `too_many_lines`-suppressed `run` function meaningfully
  larger; R3's replacement is a net-neutral-to-smaller change (extracting
  the classify branch into `route_provider_failure` reduces the inline
  match arms from ~12 lines to ~5). If `cargo xtask lint-files` reports a
  *new* violation on `agent.rs` after this spec, that is a regression to
  fix before the exit criteria are satisfied — not something to suppress
  with a new `#[allow]`.
- **N3**: no `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!` in any file
  this spec touches, except the one `unwrap_or_else`-guarded fallback in R4
  (which is not an `unwrap`/`expect` — the lint distinction is exact, use
  `unwrap_or_else`, never bare `unwrap`).
- **N4**: quality gate stays green (`cargo fmt --all --check`, `cargo
  clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace
  --lib --bins --tests`, `cargo test --workspace --doc`); test count only
  goes up.
- **N5**: `grep -c 'PortName::from("fail")' crates/pii-server/src/agent.rs`
  is exactly `1` in non-test code (R3's single `route_provider_failure`
  construction site) — this is the literal check that the `fail` port has
  exactly one producer, not scattered ad hoc across the file.

## Architecture

```
crates/pii-abi/src/node.rs
  FailurePayload { status, reason: FailoverReason, provider }   (R2)

crates/pii-abi/src/lib.rs
  pub use node::{..., FailurePayload};                          (R5)

crates/pii-server/src/agent.rs
  AgentFactory::ports() -> [ok, err, budget, fail]               (R1)
  route_provider_failure(PiiError) -> Outcome                    (R3)
    - PiiError::Provider(pf) => classify_provider(&pf) => fail port, FailurePayload
    - anything else          => err port, {"error": Display}     (unchanged from pre-A3)
  AgentNode::run()'s chat_streaming/chat retry arms call route_provider_failure
    instead of constructing Outcome::Next{"err",...} / return Err(e) inline
```

Key decisions:

- **`fail` is additive, `err` keeps its exact pre-existing shape for
  non-provider errors.** 026's own acceptance line says "non-provider
  errors remain `Err`" — read literally that would mean tool/internal
  errors keep failing the *node*, not routing to `err`. But `agent.rs`
  today already routes retry-exhausted errors (provider or not) to the
  `err` *port*, not a hard `Err` — R3 preserves that existing behavior for
  non-`Provider` errors exactly, and only narrows the hard-`Err` path
  (`:272`, unconditional today) to just non-`Provider` errors, which is
  the only reading consistent with what the code actually does today (a
  spec author asserting `err` should stop existing would be an
  unreviewed, unrequested behavior change to something A3 was not asked to
  touch).
- **One classify call, one routing function.** `route_provider_failure` is
  the single seam both retry arms call through — this is what makes N5's
  "exactly one producer" check meaningful, and what makes A4 (the failover
  ladder graph, next spec) able to document "the `fail` port's payload
  shape is `FailurePayload`, produced from exactly one call site" instead
  of auditing multiple.
- **No retry-then-fail for non-retryable `Provider` failures.** Spec-39
  already decided `is_retryable() == false` for `NextProvider`/`Compress`
  recoveries — R3 respects that by not attempting a same-provider retry for
  them (retrying a `NextProvider` failure against the *same* provider
  contradicts its own classification). This was already true of the
  pre-A3 code's *first* `match` arm (`Err(e) if e.is_retryable()`) — R3
  does not change when a retry is attempted, only what happens on ultimate
  failure.

## TDD Contract

New tests live in `crates/pii-server/src/agent.rs mod tests` (R3) and
`crates/pii-abi/src/node.rs mod tests` (R2), following the existing
in-file `#[cfg(test)] mod tests` convention already used by both files
(see `agent.rs`'s existing `test_ctx`/`MemProvider`-style fixtures around
line 1100 for the pattern to match — do not introduce a parallel test
harness).

| Test | Input | Expects |
|---|---|---|
| T1 `failure_payload_roundtrips` | `FailurePayload { status: Some(429), reason: FailoverReason::UpstreamRateLimit, provider: "ollama".into() }` (construct `FailoverReason` via spec-37's `classify` on a real 429 body, not a bare enum literal, so the test also proves the field types line up) | `serde_json::to_string` then `from_str` round-trips `==` original |
| T2 `agent_factory_ports_include_fail` | `AgentFactory::ports(&serde_json::json!({}))` | returned vec contains `PortName::from("fail")`, and still contains `ok`, `err`, `budget` (all four, order as R1 specifies) |
| T3 `provider_failure_with_context_overflow_routes_to_fail` | `AgentNode` built with a provider whose `chat`/`chat_streaming` both return `Err(PiiError::Provider(ProviderFailure { status: None, origin: Origin::Provider, provider: "llamacpp".into(), message: "llamacpp: prompt too long".into() }))` (mirrors spec-39's own llama.cpp producer) | `node.run(...)` returns `Ok(Outcome::Next { port, value })` where `port == PortName::from("fail")` and `value` deserializes to `FailurePayload { status: None, reason: FailoverReason::ContextOverflow, provider: "llamacpp".into() }` — **the acceptance test named in 026 itself** ("`ScriptedProvider` failing once with `ContextOverflow` routes to `fail`") |
| T4 `provider_failure_5xx_routes_to_fail_after_retry_exhausted` | provider's `chat_streaming` returns `Err(PiiError::Provider(status: Some(503), ...))` (retryable, `ServerError`→`RetrySame`), and the retry-arm `chat` call *also* fails with the same kind | `run(...)` returns `Ok(Outcome::Next { port: "fail", .. })` with `reason == FailoverReason::ServerError` — proves the retry-exhausted `Provider` path also classifies, not just the immediately-non-retryable path (T3) |
| T5 `non_provider_error_still_routes_to_err` | provider's `chat_streaming`/`chat` both return `Err(PiiError::Invalid("bad request shape".into()))` (not `Provider`) | `run(...)` returns `Ok(Outcome::Next { port: "err", value })` where `value == json!({"error": "bad request shape"})`'s shape (i.e. still the pre-A3 free-form shape) — the negative case proving R3 does not widen what routes to `fail` |
| T6 `fail_port_outcome_is_journaled` | run a tiny one-node graph (`AgentNode` wired as the entry, provider forced to a `Provider`/`ContextOverflow` failure as in T3) through the existing graph-test harness used by `crates/pii-graph/tests/` (or an equivalent `pii-server` integration test using `Engine`), read the journal back | the journal's `NodeFinished`/equivalent event for this node shows `port == "fail"` and a value containing `"reason":"ContextOverflow"` — proves R6 (no new journal code needed, the existing mechanism already carries it) |

T3 is the literal 026 acceptance test; T5 is the requirement a plausible
implementation gets wrong (routing every classify-able-looking error to
`fail`, silently swallowing the "non-provider errors remain [their
existing behavior]" half of the contract).

## Exit Criteria

- [ ] `cargo test -p pii-abi --lib --quiet failure_payload_roundtrips 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R2 (T1)
- [ ] `cargo test -p pii-server --lib --quiet agent_factory_ports_include_fail 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R1 (T2)
- [ ] `cargo test -p pii-server --lib --quiet provider_failure_with_context_overflow_routes_to_fail 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R3, the 026 acceptance test (T3)
- [ ] `cargo test -p pii-server --lib --quiet provider_failure_5xx_routes_to_fail_after_retry_exhausted 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R3 retry-exhausted path (T4)
- [ ] `cargo test -p pii-server --lib --quiet non_provider_error_still_routes_to_err 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R3 negative case (T5)
- [ ] `cargo test -p pii-server --quiet fail_port_outcome_is_journaled 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R6 (T6, integration-level, no `--lib` restriction since it likely lives under `tests/`)
- [ ] `[ "$(grep -c 'PortName::from("fail")' crates/pii-server/src/agent.rs)" -eq 1 ] || grep -n 'PortName::from("fail")' crates/pii-server/src/agent.rs | grep -vq '#\[cfg(test)\]'` — N5 (adjust to count only non-test occurrences if T2/T3/etc.'s own assertions also construct the literal; the intent is one *producer* in `run`/`route_provider_failure`, test code constructing the same literal to assert against is expected and fine)
- [ ] `grep -q "FailurePayload" crates/pii-abi/src/lib.rs` — R5 re-export
- [ ] `! grep -qE "unwrap\(|expect\(|panic!|todo!|unimplemented!" crates/pii-abi/src/node.rs` — N3
- [ ] `[ "$(wc -l < crates/pii-abi/src/node.rs)" -le 450 ]` — N1
- [ ] `cargo test --workspace --lib --bins --tests` — 0 failures (N4)
- [ ] `cargo test --workspace --doc` — 0 failures (N4)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N4)
- [ ] `cargo fmt --all --check` — clean (N4)
- [ ] `cargo run -q -p xtask -- silent-noop-check` — green (N4)
- [ ] `cargo run -q -p xtask -- lint-files` — no new violation vs the pre-spec baseline (N2)

## Guardrails

- **G1 — do NOT edit this spec.** Report a wrong premise instead of coding
  around it.
- **G2 — do NOT commit.** Leave the tree for orchestrator review.
- **G3 — do NOT weaken, skip, or delete an existing test.** Every existing
  `agent.rs` test (the `ok`/`err`/`budget` port tests, the memory-injection
  test near line 1100, the compaction tests) must still pass unmodified —
  R3 changes *how* `err`/`fail` routing is decided, not the pre-existing
  `ok`/`budget` paths, and existing `err`-port tests that used a non-
  `Provider` error must keep passing exactly as written.
- **G4 — do NOT touch `specs/`, `graphs/`, or any Phase 5.2/A4/A5 file.**
  Building `graphs/starter/failover.toml` is A4, a separate spec that
  consumes this one's `fail` port — do not create it here.
- **G5 — do NOT reorder or rename the existing `ok`/`err`/`budget` ports.**
  `fail` is appended, not inserted.
- **G6 — do NOT widen what routes to `fail`** beyond `PiiError::Provider`
  variants. A tool error, an internal bug, a budget exhaustion — none of
  these become `fail`; T5 is the literal regression guard for this.
- **G7 — never panic, never guess.** No `unwrap`/`expect`/`panic!`/indexing
  in any file this spec touches, tests included (the one sanctioned
  `unwrap_or_else` in R4 is not a panic — it degrades gracefully).
- **G8 — no AI-assistant or model-vendor names** in code comments, docs, or
  any commit message (project hard rule).

## Files to Modify

- `crates/pii-abi/src/node.rs` — `FailurePayload` (R2), test additions (T1).
- `crates/pii-abi/src/lib.rs` — re-export extension (R5).
- `crates/pii-server/src/agent.rs` — `AgentFactory::ports` (R1),
  `route_provider_failure` (R3), retry-arm rewire (R3), test additions
  (T2–T5).
- A `crates/pii-server/tests/*.rs` file (existing or new, integration-level)
  for T6 — reuse an existing test file's `Engine`/graph-harness pattern
  (e.g. `tests/model_routing.rs` or `tests/session.rs`'s fixture style)
  rather than inventing a new harness; if none fits, a new
  `tests/fail_port.rs` is acceptable.

# pi-improved — spec-37: failover-reason-classify (A1)

- **Project**: `<repo>`
- **Priority**: HIGH — Track A1 of research doc 025 §5. Everything in Track A
  and Track B depends on it. Small, pure, no runtime behaviour change.

## Overview

Doc 025 §3: Pii throws away the *type* of a provider failure. `ollama.rs:124`
and `:218` map `429 || >= 500` to `PiiError::Unavailable("ollama: upstream
status N")`, and `PiiError` only offers a 2-way `is_retryable()` /
`is_terminal()` split. A graph cannot branch on a reason Pii never produces
(025 §4.2), so failover-as-a-graph (A4) and routing (B1–B3) are blocked on
one thing: a typed `FailoverReason` and a pure `classify()` that produces it.

This spec ports the *idea* of Hermes's `agent/error_classifier.py` — every
failure is typed, and **the type selects the recovery** — trimmed to the
classes Pii can actually produce (025 §6 "over-engineering before need").
The load-bearing rule is that `ContextOverflow` / `PayloadTooLarge` recover
by **compressing the request, never by advancing the provider chain**: they
are properties of the request, so failing over reproduces the identical
failure on every hop and burns the chain for nothing (025 §2.1, §4.5).

## Requirements

### Functional

- **R1 (`FailoverReason`)**: a new module `crates/pii-abi/src/failover.rs`
  declares `pub enum FailoverReason` with exactly these 11 variants, in this
  order: `Auth`, `Billing`, `RateLimit`, `UpstreamRateLimit`, `Overloaded`,
  `ServerError`, `Timeout`, `ContextOverflow`, `PayloadTooLarge`,
  `ModelNotFound`, `Unknown`. Derives: `Debug, Clone, Copy, PartialEq, Eq,
  Hash, Serialize, Deserialize, JsonSchema` with
  `#[serde(rename_all = "snake_case")]`. Every variant carries a `///` doc
  comment stating its producer and its recovery (missing_docs is `deny`).
- **R2 (wire name)**: `pub const fn as_str(self) -> &'static str` returns the
  snake_case name (`"rate_limit"`, `"upstream_rate_limit"`, …) and
  `impl Display` writes the same string. `pub const ALL: [FailoverReason; 11]`
  lists every variant once. These are what A3 puts in `$.payload.reason` and
  what A4's `route` rules compare with `eq = "rate_limit"` — the serde form
  and `as_str()` must be identical.
- **R3 (`Recovery`)**: `pub enum Recovery { RetrySame, NextProvider, Compress }`
  (same derives as R1) and `pub const fn recovery(self) -> Recovery` on
  `FailoverReason`, a total `match` with this mapping:

  | reason | recovery | why |
  |---|---|---|
  | `Auth` | `NextProvider` | single key per provider (025 §3); the next entry has its own |
  | `Billing` | `NextProvider` | credit exhaustion does not clear on retry |
  | `RateLimit` | `RetrySame` | backoff on the same provider first (`NodeSpec.retry`), the ladder moves on when the budget ends |
  | `UpstreamRateLimit` | `NextProvider` | our key is healthy, the *model* is throttled — a different model is the fix |
  | `Overloaded` | `RetrySame` | 503/529 is transient |
  | `ServerError` | `RetrySame` | 500/502 is transient |
  | `Timeout` | `RetrySame` | transient |
  | `ContextOverflow` | `Compress` | **request property — never advance the chain** |
  | `PayloadTooLarge` | `Compress` | **request property — never advance the chain** |
  | `ModelNotFound` | `NextProvider` | deterministic on this entry |
  | `Unknown` | `NextProvider` | never guess retryable (025 §2.1 `unknown -> fall over`) |

- **R4 (`Origin`)**: `pub enum Origin { Provider, Upstream }` (`Debug, Clone,
  Copy, PartialEq, Eq`). `Provider` = the endpoint we called produced the
  error itself; `Upstream` = an aggregator relayed an error from the model
  behind it (Hermes: OpenRouter's "Provider returned error" + `metadata.raw`).
  A1 does **not** detect origin — the caller states it (A2 will extract it
  from the response body). No `Default` impl: the caller must decide.
- **R5 (`classify`)**:
  `pub fn classify(status: Option<u16>, message: &str, origin: Origin) -> FailoverReason`.
  Pure, total, deterministic, `#[must_use]`; never panics for any input
  (any `u16`, any `&str` including empty / non-ASCII / multi-KB). Matching is
  ASCII-case-insensitive substring (`message.to_ascii_lowercase().contains`),
  no regex, no new dependencies. Rules, evaluated top to bottom, **first match
  wins**:

  | # | status | message condition | origin | → reason |
  |---|---|---|---|---|
  | S1 | `401` | any | any | `Auth` |
  | S2 | `403` | BILLING pattern | any | `Billing` |
  | S3 | `403` | otherwise | any | `Auth` |
  | S4 | `402` | any | any | `Billing` |
  | S5 | `404` | contains `"model"` **and** any of `not found`, `does not exist`, `no such`, `unknown`, `invalid`, `unsupported` | any | `ModelNotFound` |
  | S6 | `404` | otherwise | any | `Unknown` (wrong URL path, not a missing model) |
  | S7 | `408` | any | any | `Timeout` |
  | S8 | `413` | any | any | `PayloadTooLarge` |
  | S9 | `429` | OVERLOADED pattern | any | `Overloaded` (some providers reuse 429 for capacity) |
  | S10 | `429` | otherwise | `Upstream` | `UpstreamRateLimit` |
  | S11 | `429` | BILLING pattern | `Provider` | `Billing` |
  | S12 | `429` | otherwise | `Provider` | `RateLimit` |
  | S13 | `400` | CONTEXT pattern | any | `ContextOverflow` |
  | S14 | `400` | otherwise | any | `Unknown` |
  | S15 | `500` / `502` | CONTEXT pattern | any | `ContextOverflow` (proxies surface overflow as 5xx) |
  | S16 | `500` / `502` | otherwise | any | `ServerError` |
  | S17 | `503` / `529` | CONTEXT pattern | any | `ContextOverflow` |
  | S18 | `503` / `529` | otherwise | any | `Overloaded` |
  | S19 | other `500..=599` | any | any | `ServerError` |
  | S20 | any other `Some(_)` (incl. 1xx/2xx/3xx, other 4xx) | any | any | `Unknown` |
  | M1 | `None` | TIMEOUT pattern | any | `Timeout` |
  | M2 | `None` | RATE_LIMIT pattern | `Upstream` | `UpstreamRateLimit` |
  | M3 | `None` | RATE_LIMIT pattern | `Provider` | `RateLimit` |
  | M4 | `None` | OVERLOADED pattern | any | `Overloaded` |
  | M5 | `None` | CONTEXT pattern | any | `ContextOverflow` |
  | M6 | `None` | otherwise | any | `Unknown` |

  `Origin` influences **only** S10/M2. M2/M3 run **before** M5 on purpose:
  a throttle message like `"throttling: too many tokens, please wait"` must
  back off, not compress (Hermes `_RATE_LIMIT_PATTERNS` note).
- **R6 (pattern lists)**: private `const` slices of lowercase `&str`, exactly:
  - `BILLING`: `insufficient credits`, `insufficient_quota`, `insufficient
    balance`, `credit balance`, `credits exhausted`, `payment required`,
    `exceeded your current quota`, `out of funds`, `key limit exceeded`,
    `spending limit`.
  - `RATE_LIMIT`: `rate limit`, `rate_limit`, `too many requests`, `throttl`,
    `requests per minute`, `tokens per minute`, `resource_exhausted`.
  - `OVERLOADED`: `overloaded`, `at capacity`, `over capacity`.
  - `CONTEXT`: `context length`, `context size`, `maximum context`, `context
    window`, `too many tokens`, `prompt too long`, `prompt is too long`,
    `input is too long`, `max_model_len`, `n_ctx`, `maximum allowed input
    length`, `exceeds the maximum number of input tokens`. (`prompt too long`
    is a real Pii producer: `llamacpp.rs:102`/`:130`.)
  - `TIMEOUT`: `timed out`, `timeout`, `deadline exceeded`. (`deadline
    exceeded` is the `Display` of `PiiError::Timeout`, `error.rs:22`.)
  Adding a pattern requires a TDD row; the lists are part of the contract.
- **R7 (re-export)**: `lib.rs` gains `mod failover;` and
  `pub use failover::{classify, FailoverReason, Origin, Recovery};`.

### Non-Functional

- **N1**: no changes outside `crates/pii-abi/src/failover.rs` and
  `crates/pii-abi/src/lib.rs`. `PiiError` is untouched — `classify` does not
  take a `PiiError`; wiring producers to it is spec A2.
- **N2**: no new dependency in `crates/pii-abi/Cargo.toml` (lib.rs: "must
  never gain a new one"). `serde`/`serde_json`/`schemars` are already there.
- **N3**: `failover.rs` ≤ 450 lines including tests; no `unwrap`/`expect`/
  `panic!`/`todo!` anywhere in the file (workspace lints deny them; tests use
  `assert_eq!` on `Option`/`Result` values instead).
- **N4**: quality gate stays green; test count only goes up; `silent-noop-check`
  stays green.

## Architecture

One new leaf module in the ABI crate, no runtime consumers yet:

```
crates/pii-abi/src/failover.rs
  FailoverReason (11)  --recovery()-->  Recovery (3)
  Origin (2)  ─┐
  status, msg ─┴─> classify() -> FailoverReason      (pure; no I/O; no PiiError)
```

Key decisions:

- **Classify from primitives, not from `PiiError`.** Today the status code is
  buried in message text (`"ollama: upstream status 429"`) and origin is not
  recorded at all, so a `classify(&PiiError)` would have to regex the message —
  fragile and untestable for `UpstreamRateLimit`. Taking `(status, message,
  origin)` makes every rule reachable by a unit test with no provider change,
  and gives A2 an exact contract to feed. Rejected: parsing `status N` out of
  the message (a hack A2 makes unnecessary).
- **Trimmed taxonomy, and why two proposed variants were dropped.** Hermes
  has 16 reasons earned by hundreds of provider quirks. `AuthPermanent`
  ("auth failed after refresh") is not produced by Hermes's classifier at all
  — it is set by callers after a credential refresh fails, and Pii has no
  credential refresh or pool (025 §3). A rule for it would be untestable
  from primitives, so it is omitted along with `Recovery::RotateCredential`
  and `Recovery::Abort` (nothing would map to them). Every `Recovery` variant
  is reachable and tested (T19). Grow the enum when a real producer appears.
- **`Unknown -> NextProvider`, deliberately.** Connection failures
  (`dial.rs:39` `"net: no address reachable"`) classify as `Unknown`, and
  moving to the next provider is the *correct* action for an unreachable
  host — no `Transport` variant is needed in A1.
- **No `is_retryable()` bridge.** `Recovery` is the strategy; the existing
  2-way helpers stay as they are for the scheduler's `RetryPolicy`.

## TDD Contract

All tests live in `failover.rs` `mod tests`, one `#[test]` per row, named
exactly as given (so the exit criteria can filter on them). `P` = `Origin::Provider`,
`U` = `Origin::Upstream`.

| Test | Input `(status, message, origin)` | Expects |
|---|---|---|
| T1 `classify_401_is_auth` | `(Some(401), "invalid api key", P)` | `Auth` |
| T2 `classify_403_billing_text_is_billing` | `(Some(403), "Key limit exceeded", P)` | `Billing` |
| T3 `classify_403_plain_is_auth` | `(Some(403), "forbidden", P)` | `Auth` |
| T4 `classify_402_is_billing` | `(Some(402), "", P)` | `Billing` |
| T5 `classify_404_naming_model_is_model_not_found` | `(Some(404), "model 'foo' not found, try pulling it first", P)` | `ModelNotFound` |
| T6 `classify_404_bare_is_unknown` | `(Some(404), "404 page not found", P)` | `Unknown` |
| T7 `classify_408_is_timeout` | `(Some(408), "", P)` | `Timeout` |
| T8 `classify_413_is_payload_too_large` | `(Some(413), "", P)` | `PayloadTooLarge` |
| T9 `classify_429_provider_is_rate_limit` | `(Some(429), "Too Many Requests", P)` | `RateLimit` |
| T10 `classify_429_upstream_is_upstream_rate_limit` | `(Some(429), "Provider returned error", U)` | `UpstreamRateLimit` |
| T11 `classify_429_overloaded_text_is_overloaded` | `(Some(429), "server is overloaded", U)` | `Overloaded` (S9 beats S10) |
| T12 `classify_429_billing_text_is_billing` | `(Some(429), "insufficient credits", P)` | `Billing` |
| T13 `classify_503_and_529_are_overloaded` | `(Some(503), "", P)` and `(Some(529), "", P)` | both `Overloaded` |
| T14 `classify_500_and_502_are_server_error` | `(Some(500), "", U)` and `(Some(502), "", P)` | both `ServerError` (origin ignored off 429) |
| T15 `classify_400_context_text_is_context_overflow` | `(Some(400), "This model's maximum context length is 8192 tokens", P)` | `ContextOverflow` |
| T16 `classify_500_context_text_is_context_overflow` | `(Some(500), "context length exceeded", P)` | `ContextOverflow` (S15 beats S16) |
| T17 `classify_message_only_timeout` | `(None, "deadline exceeded after 5000ms", P)` | `Timeout` |
| T18 `classify_message_only_prompt_too_long` | `(None, "llamacpp: prompt too long", P)` | `ContextOverflow` |
| T19 `every_recovery_is_reachable_and_compress_is_exact` | iterate `ALL` | exactly `{ContextOverflow, PayloadTooLarge}` map to `Compress`; ≥1 maps to `RetrySame`; ≥1 maps to `NextProvider` |
| T20 `compress_reasons_never_advance_chain` | `ContextOverflow`, `PayloadTooLarge` | `recovery() == Compress`, and `!= NextProvider` asserted explicitly |
| T21 `unknown_is_next_provider_not_retry` | `Unknown`; also `(None, "net: no address reachable for host", P)` and `(Some(418), "", P)` and `(Some(200), "", P)` | classify → `Unknown`; `Unknown.recovery() == NextProvider` |
| T22 `throttle_text_beats_context_text` | `(None, "Throttling error: too many tokens, please wait", P)` | `RateLimit` (M3 before M5) |
| T23 `wire_name_matches_serde_for_every_variant` | iterate `ALL` | `serde_json::to_value(r).ok() == Some(json!(r.as_str()))`, `to_string() == as_str()`, and `from_value` round-trips |
| T24 `all_lists_every_variant_once` | `ALL` | `len() == 11`, all `as_str()` distinct; plus an exhaustive `match` over one variant with all 11 arms so adding a variant fails to compile until `ALL` is revisited |
| T25 `classify_never_panics` | every `status in 0..=999` × messages `["", "ünïcödé ☃", "x".repeat(65_536)]` × both origins | returns (any value); no panic |

**Acceptance tests: T19/T20 (Compress is exact and never NextProvider) and
T10/T11 (origin decides 429).** The requirement a plausible implementation
gets wrong is R3's `Compress` row — a naive port maps every "error" to
`NextProvider` because "the provider failed"; T20 fails on that version.
T22 fails on a message-only path that checks context patterns first.

## Exit Criteria

Machine-checked; test filters require a non-zero pass count (non-vacuity).
`cargo test --workspace` is split into `--doc` and `--lib --bins --tests`
because the combined form has a pre-existing pii-abi doctest build-ordering
flake (HANDOFF "GATE GOTCHA", 2026-09-11).

- [ ] `cargo test -p pii-abi --lib --quiet failover 2>&1 | grep -qE "^test result: ok\. (2[5-9]|[3-9][0-9])"` — T1–T25 all present and green (R1–R7)
- [ ] `cargo test -p pii-abi --lib --quiet compress_reasons_never_advance_chain 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R3 Compress (T20)
- [ ] `cargo test -p pii-abi --lib --quiet every_recovery_is_reachable_and_compress_is_exact 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R3 total + exact (T19)
- [ ] `cargo test -p pii-abi --lib --quiet classify_429_upstream_is_upstream_rate_limit 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R4/R5 origin (T10)
- [ ] `cargo test -p pii-abi --lib --quiet throttle_text_beats_context_text 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R5 rule order (T22)
- [ ] `cargo test -p pii-abi --lib --quiet wire_name_matches_serde_for_every_variant 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R2 (T23)
- [ ] `cargo test -p pii-abi --lib --quiet classify_never_panics 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R5 total (T25)
- [ ] `grep -q "pub use failover::{classify, FailoverReason, Origin, Recovery}" crates/pii-abi/src/lib.rs` — R7
- [ ] `[ "$(grep -c '^    [A-Z][A-Za-z]*,$' crates/pii-abi/src/failover.rs)" -ge 11 ]` — R1 variants declared one per line
- [ ] `! grep -qE "unwrap\(|expect\(|panic!|todo!|unimplemented!" crates/pii-abi/src/failover.rs` — N3
- [ ] `! grep -qE "^(regex|once_cell|lazy_static|strum)" crates/pii-abi/Cargo.toml` — N2 no new deps
- [ ] `[ "$(wc -l < crates/pii-abi/src/failover.rs)" -le 450 ]` — N3 file gate
- [ ] `[ -z "$(git status --porcelain crates | sed '/crates\/pii-abi\//d')" ]` — N1 nothing else touched
- [ ] `cargo test --workspace --lib --bins --tests` — 0 failures (N4)
- [ ] `cargo test --workspace --doc` — 0 failures (N4)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N4)
- [ ] `cargo fmt --all --check` — clean (N4)
- [ ] `cargo run -q -p xtask -- silent-noop-check` — green (N4)

## Guardrails

- **G1 — do NOT edit this spec.** Report a wrong premise instead of coding
  around it (e.g. if a pattern list entry provably mis-fires on a real
  producer message, say so; do not silently reorder rules).
- **G2 — do NOT commit.** Leave the tree for orchestrator review.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT touch `specs/`**, `crates/pii-model`, `crates/pii-graph`,
  `crates/pii-server`, or any `graphs/`. Provider wiring (A2), payload
  surfacing (A3), the ladder graph (A4) and config are separate specs; this
  one is a pure type + function.
- **G5 — do NOT modify `PiiError`** or its helpers, and do not add a
  `From<PiiError>` / `classify_error(&PiiError)` bridge. Doing so would bake
  in message-text parsing that A2 replaces.
- **G6 — do NOT add a dependency** to `pii-abi` (regex, strum, once_cell…).
  Pattern matching is `to_ascii_lowercase()` + `contains`; enumeration is the
  hand-written `ALL` constant.
- **G7 — never panic, never guess.** Unrecognised input is `Unknown`. No
  `unwrap`/`expect`/`panic!`/indexing in the module, tests included.
- **G8 — no AI-assistant or model-vendor names** in code comments, docs, or
  any commit message (project hard rule).

## Files to Modify

- `crates/pii-abi/src/failover.rs` — new: `FailoverReason`, `Recovery`,
  `Origin`, `classify`, pattern consts, `mod tests` (T1–T25).
- `crates/pii-abi/src/lib.rs` — `mod failover;` + re-export (R7).

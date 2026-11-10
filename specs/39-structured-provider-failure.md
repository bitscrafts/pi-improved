# pi-improved — spec-39: structured-provider-failure (A2)

- **Project**: `<repo>`
- **Priority**: HIGH — Track A2 of research doc 025 §5 / 026 report F-11.
  A1 (spec 37, `classify()`) landed with zero non-test callers: nothing
  feeds it structured data. A2 is the planner decision the 026 report
  flagged as a prerequisite for any further Track A spec (A3/A4/A5) — it
  decides the shape once, so downstream specs implement against a fixed
  contract instead of re-deciding it per call site.

## Overview

`PiiError` today has no slot for a provider failure's `(status, origin,
provider, message)` — `OllamaProvider::send` (`ollama.rs:124-128,218-222`)
discards the real HTTP status and body text, collapsing every `429`/`5xx`
into `PiiError::Unavailable("ollama: upstream status {status}")`, a string
`classify()` would have to regex to recover the status it already had before
formatting. `LlamaCppProvider` has the same shape problem without even an
HTTP status: `"llamacpp: prompt too long"` (`llamacpp.rs:102,130`) is a
`PiiError::Internal` with no reason code at all, despite being a textbook
`ContextOverflow` producer `CONTEXT` already pattern-matches (spec 37 R6).

This spec adds a `PiiError::Provider(ProviderFailure)` variant carrying the
primitives `classify()` already accepts, makes both providers construct it
instead of formatting the status away, and adds `classify_provider()` so a
caller holding a `PiiError` — not raw `(status, message, origin)` — can reach
a `FailoverReason` in one call. It does **not** wire the reason into a graph
node port (A3), build the failover ladder graph (A4), or add an operator
surface (A5) — those are separate specs so this one's diff stays reviewable
and A3 does not need to touch files this spec is still landing.

## Requirements

### Functional

- **R1 (`ProviderFailure`)**: a new struct in `crates/pii-abi/src/failover.rs`
  (same module as `classify`/`FailoverReason` — A1's own module, so A2 does
  not create a second failover-adjacent file):
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
  pub struct ProviderFailure {
      pub status: Option<u16>,
      pub origin: Origin,
      pub provider: String,
      pub message: String,
  }
  ```
  `origin` reuses A1's existing `Origin` enum (no new derives needed there —
  it already has `Clone, Copy, PartialEq, Eq`; A2 does not add
  `Serialize`/`Deserialize`/`JsonSchema` to `Origin` unless R1's own derive
  requires it, in which case add exactly those three, nothing else).
  `provider` is the provider's `ModelProvider::name()` string (`"ollama"`,
  `"llamacpp"`), not a display label. Every field is `pub` (this is a data
  carrier, not a type with invariants — unlike `FailoverReason`, there is no
  behaviour to protect behind privacy).
- **R2 (`PiiError::Provider`)**: `crates/pii-abi/src/error.rs` gains a new
  variant:
  ```rust
  #[error("provider failure: {0:?}")]
  Provider(ProviderFailure),
  ```
  appended after `Unavailable` (last position — do not reorder existing
  variants, several `match`es elsewhere are exhaustive and a mid-list
  insertion would silently compile if they have a wildcard arm, masking a
  gap this spec must not introduce). `is_retryable`/`is_terminal` are
  extended: `Provider(ProviderFailure { .. }) => reason(self).recovery() ==
  Recovery::RetrySame` for `is_retryable`, and `Recovery::NextProvider ==
  false` — i.e. `Provider` is retryable exactly when its classified reason's
  recovery is `RetrySame`, and **never** terminal (a `NextProvider`/
  `Compress` failure is not "never succeeds", it needs different handling,
  which is exactly why `is_terminal` must stay `false` for it — a caller
  checking `is_terminal()` before giving up must not give up on a provider
  failure that the failover ladder (A4) can still recover from elsewhere in
  the chain).
- **R3 (`classify_provider`)**: `crates/pii-abi/src/failover.rs` gains
  `pub fn classify_provider(f: &ProviderFailure) -> FailoverReason` —
  `classify(f.status, &f.message, f.origin)`. A thin adapter, not a
  reimplementation; if `classify`'s signature ever changes this is the one
  call site that must change with it. Re-exported: `lib.rs`'s existing
  `pub use failover::{classify, FailoverReason, Origin, Recovery};` gains
  `ProviderFailure, classify_provider`.
- **R4 (Ollama constructs `ProviderFailure`, not a formatted string)**:
  `crates/pii-model/src/ollama.rs`'s `send` (both call sites: the
  non-streaming path `:124-128` and the streaming path `:218-222`) replace
  `PiiError::Unavailable(format!("ollama: upstream status {}", resp.status))`
  with:
  ```rust
  return Err(PiiError::Provider(ProviderFailure {
      status: Some(resp.status),
      origin: Origin::Provider,
      provider: String::from("ollama"),
      message: String::from_utf8_lossy(&resp.body).into_owned(),
  }));
  ```
  This captures the response body Ollama actually returns (today discarded
  entirely on the error path) so `classify`'s message-pattern rules (BILLING,
  RATE_LIMIT, etc.) have real text to match instead of the synthetic
  `"upstream status N"` string that matches nothing. The `!(200..300)` branch
  (`:130-135`, `:225-229`) is unchanged — a non-retryable 4xx client error
  (bad request shape) is not a provider failure in the failover sense, it is
  a Pii-side bug, so it stays `PiiError::Invalid`.
- **R5 (llama.cpp constructs `ProviderFailure` for context overflow)**:
  `crates/pii-model/src/llamacpp.rs`'s two `"llamacpp: prompt too long"`
  sites (`:102`, `:130`) replace `PiiError::Internal("llamacpp: prompt too
  long".into())` with:
  ```rust
  PiiError::Provider(ProviderFailure {
      status: None,
      origin: Origin::Provider,
      provider: String::from("llamacpp"),
      message: String::from("llamacpp: prompt too long"),
  })
  ```
  `classify_provider` on this maps to `ContextOverflow` via spec 37's M5 rule
  (the `CONTEXT` pattern list's literal entry `"prompt too long"` matches) →
  `Compress` recovery. This is the one site the 026 report explicitly calls
  out (`CONTEXT` pattern comment: `"prompt too long" is a real Pii producer:
  llamacpp.rs:102/:130`) as proof the taxonomy is real, not speculative — R5
  is what actually wires that producer up. Every other `PiiError::Internal(format!(...))` site in
  `llamacpp.rs` (backend init, context, tokenize, batch add, decode, grammar
  — nine sites) is **not** touched: those are genuine internal faults (a
  malloc failure, a tokenizer bug), not provider failures a failover chain
  could route around, and forcing them into `ProviderFailure` would be
  exactly the "message-string parsing" anti-pattern this spec exists to kill
  in the other direction.
- **R6 (no other `PiiError::{Internal,Invalid,Unavailable}(format!(...))`
  site is touched)**: the 026 report's count of 357 such sites is not this
  spec's scope to reduce — R4 and R5 fix the two sites that are genuinely
  provider failures with a real classify producer; the remaining count drops
  by exactly these (Ollama: 2, llama.cpp: 2 — `4` net removed from the
  `grep -c` baseline). A future spec may widen `ProviderFailure` construction
  to more providers/sites; this one proves the shape works end to end on the
  two it can verify against real classify producers today.

### Non-Functional

- **N1**: `crates/pii-abi/Cargo.toml` gains no new dependency (R1 reuses
  `serde`/`schemars` already present for `FailoverReason`).
- **N2**: `crates/pii-abi/src/failover.rs` stays ≤ 450 lines including tests
  (spec 37 left it well under; R1/R3 add roughly 60 lines including tests —
  verify with `wc -l` in the exit criteria, do not assume).
- **N3**: no `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!` in any file
  this spec touches (workspace lint, `deny`).
- **N4**: quality gate stays green; test count only goes up;
  `cargo xtask silent-noop-check` stays green; `cargo xtask lint-files`
  reports no new violation (R4/R5's diffs are additive, not large enough to
  push `ollama.rs`/`llamacpp.rs` over their existing effective-line count by
  more than a few lines — check `xtask lint-files` output, not just line
  count, since the 450-line gate is *effective* lines).
- **N5**: `grep -rn 'PiiError::Unavailable(format!("ollama: upstream status'
  crates/pii-model` returns **zero** hits after this spec (R4 replaced both
  occurrences) — this is the literal regression test for "A2 actually wired
  the provider, not just added an unused type".

## Architecture

```
crates/pii-abi/src/failover.rs (A1's module, extended by A2)
  ProviderFailure { status, origin, provider, message }  (R1)
  classify_provider(&ProviderFailure) -> FailoverReason  (R3, thin wrapper over classify)

crates/pii-abi/src/error.rs
  PiiError::Provider(ProviderFailure)                    (R2)

crates/pii-model/src/ollama.rs      -- constructs ProviderFailure on 429/5xx (R4)
crates/pii-model/src/llamacpp.rs    -- constructs ProviderFailure on context overflow (R5)
```

Key decisions:

- **`ProviderFailure` lives in `failover.rs`, not `error.rs`.** It is
  data `classify_provider` consumes and `FailoverReason` is produced from —
  co-locating it with `classify`/`FailoverReason`/`Origin` keeps the whole
  failover-adjacent surface in one file, and `error.rs` only needs to know
  the variant exists, not the taxonomy behind it. `error.rs` imports
  `crate::failover::ProviderFailure` the same way it would any sibling type.
- **`Provider(ProviderFailure)` over overloading `Unavailable`.** The 026
  report's explicit recommendation (5.1 A2 row: "preferred over overloading
  `Unavailable`"). `Unavailable` today means "retry the same call, transient"
  — folding a `NextProvider`/`Compress` failure into it would make
  `is_retryable()` lie for two of three recovery strategies. A distinct
  variant keeps `is_retryable`/`is_terminal` honest per R2.
- **Only Ollama and llama.cpp, only their real classify producers.** R4/R5
  are deliberately narrow: they wire the two sites that already have a
  matching `classify()` rule proven by spec 37's own tests (`Overloaded`/
  `RateLimit`/`ServerError`/`Billing`/`Auth` family for Ollama's HTTP status;
  `ContextOverflow` for llama.cpp's message). Widening to every
  `PiiError::Internal`/`Invalid` site in `pii-model` (or scripted providers,
  or future providers) is out of scope — R6 states the count explicitly so a
  reviewer can verify nothing beyond R4/R5 moved.
- **No behaviour change for genuine internal faults.** llama.cpp's other
  nine `Internal(format!(...))` sites (backend/context/tokenize/batch/decode/
  grammar) stay exactly as they are — R5's own text explains why: they are
  not provider failures a failover chain could route around.

## TDD Contract

All new tests live in `crates/pii-abi/src/failover.rs mod tests` (R1/R3) and
`crates/pii-abi/src/error.rs mod tests` (R2), one `#[test]` per row, named
exactly as given.

| Test | Input | Expects |
|---|---|---|
| T1 `provider_failure_roundtrips_through_classify_provider` | `ProviderFailure { status: Some(429), origin: Origin::Upstream, provider: "ollama".into(), message: "Provider returned error".into() }` | `classify_provider(&f) == FailoverReason::UpstreamRateLimit` (matches spec 37 T10's raw-primitive equivalent — proves the adapter is a pure pass-through, no extra logic) |
| T2 `classify_provider_matches_classify_for_every_status_class` | for each of spec 37's T1–T18 `(status, message, origin)` triples, build the equivalent `ProviderFailure` and assert `classify_provider(&f) == classify(status, message, origin)` | equal for all 18 |
| T3 `provider_error_is_retryable_iff_recovery_is_retry_same` | `PiiError::Provider(ProviderFailure { status: Some(500), .. })` (→ `ServerError` → `RetrySame`) | `is_retryable() == true`, `is_terminal() == false` |
| T4 `provider_error_next_provider_is_not_retryable_or_terminal` | `PiiError::Provider(ProviderFailure { status: Some(401), .. })` (→ `Auth` → `NextProvider`) | `is_retryable() == false`, `is_terminal() == false` (the R2 "never terminal" rule — the case a naive port gets wrong by making `NextProvider` terminal) |
| T5 `provider_error_compress_is_not_retryable_or_terminal` | `PiiError::Provider(ProviderFailure { status: None, message: "llamacpp: prompt too long".into(), .. })` (→ `ContextOverflow` → `Compress`) | `is_retryable() == false`, `is_terminal() == false` |
| T6 `ollama_429_constructs_provider_failure_with_body_text` | fake transport returns `429` with body `b"insufficient credits"` | `OllamaProvider::chat(...)` returns `Err(PiiError::Provider(ProviderFailure { status: Some(429), message, .. }))` where `message.contains("insufficient credits")`; `classify_provider` on the returned failure `== FailoverReason::Billing` (proves R4 actually carries real body text, not a synthesized string) |
| T7 `ollama_5xx_constructs_provider_failure` | fake transport returns `503` with body `b"overloaded"` | returned `ProviderFailure.status == Some(503)`; `classify_provider(...) == FailoverReason::Overloaded` |
| T8 `ollama_4xx_non_retryable_stays_invalid` | fake transport returns `400` with an arbitrary body | `Err(PiiError::Invalid(_))`, **not** `Provider(_)` — R4's `!(200..300)` branch is untouched |
| T9 `llamacpp_context_overflow_constructs_provider_failure` | a context too small for the prompt (existing `llamacpp_requires_path`-style fixture, or a scripted/real batch-add failure path) | `LlamaCppProvider::chat(...)` (or the internal helper under test) returns `Err(PiiError::Provider(ProviderFailure { status: None, provider, message, .. }))` where `provider == "llamacpp"` and `classify_provider(...) == FailoverReason::ContextOverflow` |
| T10 `no_ollama_upstream_status_string_remains` | `grep -c 'upstream status' crates/pii-model/src/ollama.rs` (shell, exit-criteria level, not a Rust test) | `0` — the old formatted string is gone, not just unreachable dead code |

T6 and T9 are the acceptance tests: **the requirement a plausible
implementation gets wrong is constructing `ProviderFailure` with a
re-synthesized message (`format!("status {status}")`) instead of the real
HTTP body / real error text** — that would compile, pass T2/T3/T4/T5 (which
only exercise the adapter and the `PiiError` methods), and still fail the
026 report's actual complaint (providers still throw away the information
`classify` needs). T6 fails on that version because `message.contains(...)`
checks the real body text survived, not just that *a* string is present.

## Exit Criteria

Machine-checked; test filters require a non-zero pass count (non-vacuity).
`cargo test --workspace` is split into `--doc` and `--lib --bins --tests`
matching `quality-gate.sh`'s current two-step form (Phase 5.0.1 fix).

- [ ] `cargo test -p pii-abi --lib --quiet provider_failure 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R1/R3 (T1–T5, name prefix)
- [ ] `cargo test -p pii-abi --lib --quiet classify_provider_matches_classify_for_every_status_class 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R3 (T2)
- [ ] `cargo test -p pii-abi --lib --quiet provider_error_next_provider_is_not_retryable_or_terminal 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R2 (T4, the case a naive port gets wrong)
- [ ] `cargo test -p pii-model --lib --quiet ollama_429_constructs_provider_failure_with_body_text 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R4 (T6)
- [ ] `cargo test -p pii-model --lib --quiet ollama_5xx_constructs_provider_failure 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R4 (T7)
- [ ] `cargo test -p pii-model --lib --quiet ollama_4xx_non_retryable_stays_invalid 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R4 boundary (T8)
- [ ] `cargo test -p pii-model --lib --quiet llamacpp_context_overflow_constructs_provider_failure 2>&1 | grep -qE "^test result: ok\. [1-9]"` — R5 (T9)
- [ ] `! grep -q "upstream status" crates/pii-model/src/ollama.rs` — R4/T10, the literal regression test
- [ ] `[ "$(grep -c 'PiiError::Unavailable(format!("ollama: upstream status' crates/pii-model/src/ollama.rs)" -eq 0 ]` — N5
- [ ] `grep -q "ProviderFailure, classify_provider" crates/pii-abi/src/lib.rs` — R3 re-export
- [ ] `! grep -qE "unwrap\(|expect\(|panic!|todo!|unimplemented!" crates/pii-abi/src/failover.rs` — N3
- [ ] `[ "$(wc -l < crates/pii-abi/src/failover.rs)" -le 450 ]` — N2
- [ ] `cargo test --workspace --lib --bins --tests` — 0 failures (N4)
- [ ] `cargo test --workspace --doc` — 0 failures (N4)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N4)
- [ ] `cargo fmt --all --check` — clean (N4)
- [ ] `cargo run -q -p xtask -- silent-noop-check` — green (N4)
- [ ] `cargo run -q -p xtask -- lint-files` — no new violation vs the pre-spec baseline (N4)

## Guardrails

- **G1 — do NOT edit this spec.** Report a wrong premise instead of coding
  around it.
- **G2 — do NOT commit.** Leave the tree for orchestrator review.
- **G3 — do NOT weaken, skip or delete an existing test** — spec 37's T1–T25
  must still pass unmodified; R2's `is_retryable`/`is_terminal` tests
  (`pii_error_retryable_and_terminal_partition`) must still pass with the
  new variant added to its coverage, not replacing existing assertions.
- **G4 — do NOT touch `specs/`**, `crates/pii-graph`, `crates/pii-server`, or
  any `graphs/`. Payload surfacing on a node's `fail` port (A3), the ladder
  graph (A4), and the operator surface (A5) are separate specs.
- **G5 — do NOT touch any `PiiError::Internal(format!(...))` site in
  `llamacpp.rs` other than the two named in R5** (backend init, context,
  tokenize, batch add ×2, decode ×2, grammar ×2 stay untouched — nine sites,
  verify by counting before and after).
- **G6 — do NOT add a dependency** to `pii-abi` or `pii-model` beyond what
  each crate already has.
- **G7 — never panic, never guess.** No `unwrap`/`expect`/`panic!`/indexing
  in any file this spec touches, tests included.
- **G8 — no AI-assistant or model-vendor names** in code comments, docs, or
  any commit message (project hard rule).

## Files to Modify

- `crates/pii-abi/src/failover.rs` — `ProviderFailure` (R1), `classify_provider` (R3), `mod tests` additions (T1–T5).
- `crates/pii-abi/src/error.rs` — `PiiError::Provider` variant, `is_retryable`/`is_terminal` extension (R2), test additions.
- `crates/pii-abi/src/lib.rs` — re-export extension (R3).
- `crates/pii-model/src/ollama.rs` — R4, both `send` call sites, plus tests (T6–T8).
- `crates/pii-model/src/llamacpp.rs` — R5, both `"prompt too long"` sites, plus a test (T9).

# pi-improved — spec-57: pii-channels test coverage for discord.rs (F-20 tidy)

- **Project**: `<repo>`
- **Priority**: LOW-MEDIUM — Phase 5.3 tail. `pii-channels` has 1/3
  files with inline `#[cfg(test)]` (`cli.rs`); `discord.rs` (the real
  network-facing channel, 158 lines) and `lib.rs` (a pure trait/struct
  definition with nothing to unit test directly — its `Channel` trait
  is exercised through implementors like `DiscordChannel`, not tested
  standalone) have none. This spec covers `discord.rs` only.

## Overview

`DiscordChannel` takes an `Arc<dyn HttpTransport>` (from `pii-core`) as
a constructor argument specifically so it's testable without a real
network call — per the crate's own doc comment: "Every channel goes
through `Arc<dyn HttpTransport>`... which is what keeps this crate
testable with a fake." Write a local fake `HttpTransport` implementor
in `discord.rs`'s own `#[cfg(test)]` module (there is a private `Fake`
in `crates/pii-core/src/net.rs`'s own test module already doing this
same pattern for a different purpose — read it for the shape of
`HttpTransport`'s trait methods, but it is NOT exported/reusable, so
write your own local fake here, don't try to import it).

Ground truth already confirmed by the orchestrator (verify the exact
`HttpTransport`/`HttpRequest`/`HttpResponse` trait/struct shapes
yourself from `pii-core`'s actual source before writing the fake — this
is a starting map, not a substitute):

- `DiscordChannel::from_env(name, transport)` reads `PII_DISCORD_TOKEN`
  (required, else `PiiError::Invalid`), `PII_DISCORD_CHANNEL_ID`
  (required, else `PiiError::Invalid`), `PII_DISCORD_API_BASE`
  (optional, defaults to `DEFAULT_API_BASE`). Reading/setting env vars
  in tests needs care for parallel test execution — either serialize
  these specific tests (a `static Mutex<()>` guard is the crate's likely
  existing convention elsewhere in the workspace; check) or avoid
  `from_env` entirely and construct `DiscordChannel` directly if its
  fields are accessible enough (check visibility — the struct fields
  are private in the summarized source, so `from_env` or an internal
  test-only constructor may be the only way in; if fields are truly
  unreachable from tests, that's worth reporting rather than a fragile
  env-var-based workaround, per this spec's own guardrail below).
- `poll(ctx, cursor)`: GETs `{base}/channels/{id}/messages` (with
  `?after={cursor}&limit=20` when `cursor: Some`, else `?limit=20`),
  auth via `Authorization: Bot {token}` header. On non-success HTTP
  status → `PiiError::Unavailable`. On unparseable JSON body →
  `PiiError::Unavailable`. On a message with a non-numeric `id` →
  `PiiError::Unavailable`. On success: maps Discord's JSON shape
  (`{id, content, author: {username}}`) to `Incoming {channel, chat,
  user, text, cursor}`, and REVERSES the list (Discord returns
  newest-first; the doc comment says the router needs oldest-first) —
  this reversal is a real, specific, easy-to-miss behavior; test it
  explicitly with a multi-message fixture, not just a single-message
  happy path that wouldn't catch an ordering bug.
- `send(ctx, to, text)`: POSTs `{base}/channels/{to}/messages` with a
  JSON body `{"content": text}`. On non-success status →
  `PiiError::Unavailable`.

## Requirements

### Functional

- R1. Add `#[cfg(test)] mod tests` to the end of `discord.rs`. Write a
  local fake `HttpTransport` (e.g. a struct wrapping a
  `RefCell<VecDeque<HttpResponse>>` of canned responses, or whatever
  shape fits the actual trait signature you read from `pii-core`) that
  lets a test control exactly what status/body each call returns.
- R2. Test `poll` success: a fake response with 2-3 Discord-shaped
  messages, assert the resulting `Vec<Incoming>` is in the CORRECT
  (reversed-from-input) order and every field maps correctly.
- R3. Test `poll` error paths: non-success HTTP status →
  `PiiError::Unavailable`; malformed JSON body → `PiiError::
  Unavailable`; a message with non-numeric `id` → `PiiError::
  Unavailable`. Three distinct tests (or one parameterized test
  covering all three cases) — don't collapse them into a single vague
  assertion.
- R4. Test `send` success and its non-success-status error path
  (`PiiError::Unavailable`).
- R5. Test the URL construction: `messages_url` is a private method —
  either test it indirectly (assert the fake transport RECEIVED the
  expected URL string, which the fake should be able to record) or,
  if that's cleaner, make it `pub(crate)`-testable — read the file to
  decide which is the smaller, more idiomatic diff.

### Non-Functional

- N1. Do NOT modify any NON-test code in `discord.rs` — the request/
  response handling logic itself must not change. If `messages_url`'s
  visibility needs to widen slightly (`fn` → `pub(super)` or similar)
  to make it testable, that's an acceptable minimal exception; a
  logic change is not.
- N2. Do NOT touch `cli.rs` (already has tests) or `lib.rs` (nothing to
  unit test standalone — pure trait/struct definitions).
- N3. Standard test-module clippy allows apply
  (`unwrap_used`/`expect_used`/`panic`/`indexing_slicing`).
- N4. If `DiscordChannel`'s fields turn out to be genuinely unreachable
  for a fields-based test constructor AND `from_env`'s env-var
  dependency makes clean test isolation impractical, STOP and report
  this as a real testability gap rather than writing a flaky test that
  races other tests over shared env state.

## Architecture

No new files — appends `#[cfg(test)] mod tests { ... }` to the end of
`crates/pii-channels/src/discord.rs`, matching `cli.rs`'s existing
in-file pattern in the same crate.

## TDD Contract

| Test Name (illustrative) | Input | Expected Output |
|---|---|---|
| `poll_maps_and_reverses_discord_order` | fake returns 3 messages, newest-first | `Vec<Incoming>` in oldest-first order, fields mapped correctly |
| `poll_non_success_status_is_unavailable` | fake returns HTTP 500 | `PiiError::Unavailable` |
| `poll_malformed_body_is_unavailable` | fake returns invalid JSON | `PiiError::Unavailable` |
| `poll_non_numeric_id_is_unavailable` | a message with `id: "not-a-number"` | `PiiError::Unavailable` |
| `send_success` | fake returns HTTP 200 | `Ok(())`, fake recorded the expected POST body/URL |
| `send_non_success_status_is_unavailable` | fake returns HTTP 403 | `PiiError::Unavailable` |

## Exit Criteria

- [ ] `cargo test -p pii-channels`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `grep -q "cfg(test)" crates/pii-channels/src/discord.rs`

## Guardrails

- G1. Do NOT touch `crates/pii-channels/src/cli.rs` or `lib.rs`.
- G2. Do NOT change `discord.rs`'s request/response HANDLING logic —
  only add a test module and, if genuinely needed, a minimal visibility
  widening on one private helper (see N1).
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).

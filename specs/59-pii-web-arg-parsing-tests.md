# pi-improved — spec-59: pii-web flag-parsing test coverage (F-20 tidy)

- **Project**: `/projects/pi-improved`
- **Priority**: LOW — Phase 5.3 tail, smallest remaining item. `pii-web`
  has exactly 2 tests for 297 lines. One of the two existing tests
  (`web_defaults_to_loopback`) is a genuine tautology — it compares two
  string literals to each other, never touches `cmd_web`'s actual
  default-host variable, so it would still pass even if the real
  default silently changed to `"0.0.0.0"`. This spec's real point is
  fixing that, plus adding coverage for `cmd_web`'s flag-parsing logic
  (the one piece of this file testable without a live daemon or a real
  TCP bind — the `api_*` handlers all require a working UDS connection
  to a daemon and are explicitly OUT of scope here, a much bigger
  integration-test undertaking not worth doing in this small spec).

## Overview

`cmd_web(args: &mut std::env::ArgsOs) -> ExitCode` parses `--port`,
`--host`, `--instance`, and rejects unknown flags — before it ever
touches the network or the daemon socket. This parsing logic is cleanly
separable and testable in isolation if extracted from the function that
also does socket resolution + server startup (which genuinely can't run
in a unit test without a real daemon).

Ground truth already confirmed by the orchestrator (verify the exact
current line numbers/behavior yourself before extracting anything):

- Defaults: `port: u16 = 4602`, `host = "127.0.0.1"`, `instance =
  "default"`.
- `--port <v>`: on parse failure, SILENTLY falls back to the default
  4602 (`v.parse().unwrap_or(4602)`) rather than erroring — this is a
  real, specific, worth-testing behavior (a malformed `--port abc`
  does NOT reject the whole command, it silently uses 4602).
- `--host <v>`: sets it directly, no validation at parse time (the
  actual address validity is checked later when building `SocketAddr`,
  which is outside this spec's scope).
- `--instance <v>`: sets it directly.
- Any unrecognized flag: prints `error: unknown flag '{other}'` to
  stderr, returns `ExitCode::from(2)` immediately (does not continue
  parsing remaining args).
- A flag with no following value (e.g. `--port` as the last arg): `next()`
  returns `None`, and each match arm's `if let Some(v) = next()` simply
  skips setting that field — so `--port` with nothing after it silently
  keeps the DEFAULT, it does not error. Confirm this from the actual
  code before asserting it; it's a subtle, easy-to-get-wrong behavior
  worth pinning with a test either way (whatever it actually does).

## Requirements

### Functional

- R1. Extract the flag-parsing loop (currently inline in `cmd_web`,
  the `while let Some(flag) = args.next() { ... }` block) into a small
  private helper, e.g. `fn parse_web_args(args: &mut std::env::ArgsOs)
  -> Result<(u16, String, String), ExitCode>` returning `(port, host,
  instance)` on success or the usage `ExitCode` on an unknown flag —
  read the current code and choose whatever return shape is the
  smallest, most natural diff; this is the ONE legitimate non-test-only
  change this spec makes (call it out explicitly in the summary), done
  specifically so the parsing logic becomes testable independent of
  socket/network setup. `cmd_web` itself calls the new helper and keeps
  its existing socket-resolution/server-startup logic untouched.
- R2. Test the extracted parser directly (constructing a fake
  `std::env::ArgsOs`-equivalent — note `ArgsOs` itself may not be
  directly constructible in a test; if so, either generalize the
  helper's parameter to something test-constructible like `impl
  Iterator<Item = OsString>` while preserving the real call site's
  behavior, or find whatever the smallest working approach is — report
  honestly if `ArgsOs`'s real-world-only nature forces a different
  shape than originally sketched here):
  - No flags → the three defaults (`4602`, `"127.0.0.1"`, `"default"`).
  - `--port 8080` → port `8080`, other defaults.
  - `--port notanumber` → falls back to default `4602` (NOT an error).
  - `--host 0.0.0.0` → host `"0.0.0.0"`.
  - `--instance foo` → instance `"foo"`.
  - Unknown flag `--bogus` → `Err(ExitCode::from(2))`.
  - A flag with no following value (last arg) → whatever the real
    current behavior is (see ground truth above — pin it, don't assume).
- R3. Fix `web_defaults_to_loopback`'s tautology: make it actually
  exercise the real default by calling the extracted parser with no
  args and asserting the returned host equals `"127.0.0.1"` (this
  replaces the old test's body, or the old test can be deleted and
  folded into R2's "no flags" case — implementer's choice, whichever
  reads more naturally, just make sure the net result actually checks
  the real code path).

### Non-Functional

- N1. Do NOT change `cmd_web`'s SOCKET/NETWORK/SERVER-STARTUP behavior
  at all — only the flag-parsing extraction from R1. The daemon-socket
  resolution, `TcpListener::bind`, `axum::serve` call, and all `api_*`
  handlers are completely untouched.
- N2. Standard test-module clippy allows apply.

## Architecture

`cmd_web` gains a small private helper (name/exact shape per R1) for
flag parsing; the existing `#[cfg(test)] mod tests` block at the end of
`main.rs` gains the new test cases and the fixed R3 test. No new files.

## TDD Contract

| Test Name (illustrative) | Input | Expected Output |
|---|---|---|
| `parse_web_args_defaults` | no flags | `(4602, "127.0.0.1", "default")` |
| `parse_web_args_port` | `--port 8080` | port `8080` |
| `parse_web_args_bad_port_falls_back` | `--port notanumber` | port `4602` (default, NOT an error) |
| `parse_web_args_host` | `--host 0.0.0.0` | host `"0.0.0.0"` |
| `parse_web_args_instance` | `--instance foo` | instance `"foo"` |
| `parse_web_args_unknown_flag_errors` | `--bogus` | `Err(ExitCode::from(2))` |
| `parse_web_args_dangling_flag` (name TBD) | a flag with nothing after it | whatever the real current behavior is, pinned |

## Exit Criteria

- [ ] `cargo test -p pii-web`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`

## Guardrails

- G1. Do NOT change any `api_*` handler, `call_daemon`, or the socket/
  server startup sequence in `cmd_web` beyond delegating to the new
  parse helper.
- G2. Do NOT touch `crates/pii-web/src/index.html`.
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).

# pi-improved — spec-45: split `pii-net/src/service.rs` into `service/{mod,conn,http,dechunk,tests}.rs`

- **Project**: `/projects/pi-improved`
- **Priority**: MEDIUM — `pii-net/src/service.rs` is 427 effective lines,
  in the 380-450 warn band (not a hard failure), the third Phase 5.2
  structural-debt item after specs 43 (`agent.rs`) and 44
  (`xtask/src/silent_noop.rs`).

## Overview

`crates/pii-net/src/service.rs` mixes four cohesive-but-separable concerns
in one file: the `Stream`/`Conn`/`GrantInfo` connection primitives and their
lease-accounting `Drop` impl, the `NetService` connect/lease chokepoint, the
`HttpTransport` impl (buffered `request` + streaming `request_streaming`),
and the free-function chunked-transfer-encoding decoder
(`dechunk_step`/`total_len`/`find_crlf`/`read_to_deadline`). This is a
**pure move-and-re-export refactor** — same discipline as specs 43/44: no
behavior change, no test deletions/weakenings, every `pub`/`pub(crate)`
item keeps its exact signature.

Ground truth, read before writing anything else: `crates/pii-net/src/
service.rs` in full (685 raw lines / 427 effective). Confirmed facts:
- `crates/pii-net/src/lib.rs:14` declares `pub mod service;` and `:25`
  re-exports `pub use service::{Conn, GrantInfo, NetConfig, NetService};`
  — these 4 names are the entire external surface; nothing else outside
  the module is used elsewhere in the workspace (verify this claim by
  grepping `pii_net::service::` and `pii_net::{Conn,GrantInfo,NetConfig,
  NetService}` workspace-wide before finishing, don't just trust this
  spec's own claim).
- One test module, `#[cfg(test)] mod streaming_tests { ... }`, starts at
  line 552 (original file) and runs a real loopback `TcpListener` inside
  the module (sanctioned per its own comment: "the loopback fake server
  binds TcpListener inside pii-net's own test module").
- `NetConfig`, `Stream` (+ `impl Read`/`impl Write`/`impl Stream` inherent
  methods), `Conn` (+ `Drop`), `GrantInfo` sit before line 213.
- `impl Clone for NetService`, `pub struct NetService(...)`, `impl
  NetService { fn new, fn connect }` sit at lines 213-345 (approx — verify
  exact boundary by reading the file, do not guess from this summary).
- `impl HttpTransport for NetService { fn request, fn request_streaming }`
  sits at lines 347-471.
- The 4 free functions (`dechunk_step`, `total_len`, `find_crlf`,
  `read_to_deadline`) sit at lines 477-550, called only from
  `request_streaming`/`request` above them.

## Requirements

### Functional

- R1. Convert `crates/pii-net/src/service.rs` into a directory module
  `crates/pii-net/src/service/` with `mod.rs`, `conn.rs`, `http.rs`,
  `dechunk.rs`. `crates/pii-net/src/lib.rs`'s `pub mod service;` and `pub
  use service::{Conn, GrantInfo, NetConfig, NetService};` are unchanged
  (both resolve correctly once `service.rs` becomes `service/mod.rs`,
  regardless of which sub-file a re-exported type physically lives in, as
  long as `mod.rs` re-exports it — see R2/R3).
- R2. `service/mod.rs` keeps: the module doc comment, `mod conn; mod http;
  mod dechunk;` declarations, `pub use conn::{Stream, Conn, GrantInfo};`
  (or whichever subset actually needs re-exporting from `conn.rs` — verify
  against R1's `lib.rs` re-export list), `NetConfig`, `impl Clone for
  NetService`, `pub struct NetService(...)`, and `impl NetService { fn
  new, fn connect }` (the connect/lease chokepoint stays with the type
  it's inherent to, in `mod.rs`, since it's the crate's primary public
  entry point — do not move it to `conn.rs` unless R6 forces it).
- R3. `service/conn.rs` holds `Stream` + its `impl Read`/`impl Write`/
  inherent `impl Stream` block, `Conn` struct + its `Drop` impl + inherent
  methods, and `GrantInfo`. `pub(crate)` or `pub` visibility exactly as in
  the original (do not widen or narrow).
- R4. `service/http.rs` holds `impl HttpTransport for NetService { fn
  request, fn request_streaming }`, moved verbatim except `use` statements.
- R5. `service/dechunk.rs` holds `dechunk_step`, `total_len`, `find_crlf`,
  `read_to_deadline` — moved verbatim, `pub(super)` (these are only called
  from `http.rs`).
- R6. Tests are **not split, not rewritten, not deleted**. The
  `streaming_tests` module moves as one unit — either inline in `mod.rs`,
  or into its own `service/tests.rs` following the exact precedent set by
  spec-43's `agent/tests.rs` and confirmed reusable by spec-44 (an inner
  `#![cfg(test)]` attribute, `xtask/src/lint_files.rs` already supports
  counting this shape as zero effective lines — do not re-implement that
  support, it already exists). Test *content* must be byte-for-byte
  identical; only import paths may change if items moved to sibling
  modules (e.g. `use super::NetService;` stays correct either way since
  `NetService` is re-exported from `mod.rs`).
- R7. No public API surface changes: `pii_net::{Conn, GrantInfo, NetConfig,
  NetService}` (the exact 4 names `lib.rs` re-exports today) must resolve
  identically after the split. Confirm by grepping every call site of
  these 4 names outside `pii-net` itself and checking the build succeeds
  unchanged (a compile pass alone is not sufficient proof — grep first,
  understand what depends on the shape, THEN verify the build).

### Non-Functional

- N1. Each new file individually satisfies `xtask lint-files` (≤450
  effective lines).
- N2. `#![forbid(unsafe_code)]` unaffected — no `unsafe` in the original
  file, none introduced.
- N3. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()` outside
  `#[cfg(test)]` bodies. Exit criterion below excludes the test module
  from the grep, following the exact pattern spec-44 had to correct after
  discovering the original file already had a sanctioned
  `.expect(...)`/`unwrap()` pattern inside its own test module — check
  `service.rs`'s test module for the SAME pre-existing pattern (its
  `#![allow(clippy::unwrap_used, clippy::expect_used, ...)]` line at the
  top of `streaming_tests` strongly suggests it does) before writing the
  final grep-based exit criterion, so this spec does not repeat spec-44's
  mistake of a too-strict criterion.

## Architecture

```
crates/pii-net/src/
├── service.rs                   (DELETED — becomes service/)
└── service/
    ├── mod.rs        — module doc, `mod conn; mod http; mod dechunk;`,
    │                    re-exports, NetConfig, NetService (+ new/connect),
    │                    tests (inline or `mod tests;` -> tests.rs)
    ├── conn.rs        — Stream (+ Read/Write/inherent impls), Conn (+ Drop
    │                    + inherent methods), GrantInfo
    ├── http.rs        — impl HttpTransport for NetService
    └── dechunk.rs     — dechunk_step, total_len, find_crlf, read_to_deadline
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| (all existing tests in `service.rs`'s `streaming_tests` module, unchanged) | — | every test that passed before the move still passes after, byte-for-byte same assertions |
| `pii_net_public_reexports_unchanged` (new or existing, confirm via grep before adding a duplicate) | grep `crates/pii-net/src/lib.rs` for the exact re-export line | `pub use service::{Conn, GrantInfo, NetConfig, NetService};` still present verbatim |

## Exit Criteria

- [ ] `[ ! -f crates/pii-net/src/service.rs ]`
- [ ] `[ -f crates/pii-net/src/service/mod.rs ]`
- [ ] `[ -f crates/pii-net/src/service/conn.rs ]`
- [ ] `[ -f crates/pii-net/src/service/http.rs ]`
- [ ] `[ -f crates/pii-net/src/service/dechunk.rs ]`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `grep -q "pub use service::{Conn, GrantInfo, NetConfig, NetService};" crates/pii-net/src/lib.rs`
- [ ] `cargo test -p pii-net --features testing`

## Guardrails

- G1. Do NOT change any test's assertions, names, or add/remove any test.
- G2. Do NOT change `Conn`'s `Drop` impl's lease-decrement logic (D1's TLS
  handshake failure fix from Phase 5.0 lives in `NetService::connect`,
  which per R2 stays in `mod.rs` — but re-verify the `Drop` impl on `Conn`
  itself, wherever it ends up per R3, is untouched byte-for-byte; this is
  the single most safety-critical piece of logic in this file and the
  most recently touched by a real bug fix).
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. Do NOT touch `crates/pii-net/src/lib.rs` unless the re-export line
  genuinely needs no change (it shouldn't — confirm, don't guess) and no
  other file outside `crates/pii-net/src/service.rs`/`service/`.
- G6. If splitting reveals a genuine coupling problem (e.g. `Conn`'s
  `Drop` impl needs a private field of `Inner` that visibility rules
  won't allow across the new module boundary), STOP and report it rather
  than silently widening visibility — `pub(super)` is preferred over
  `pub(crate)` where it suffices, matching specs 43/44's own guardrail.

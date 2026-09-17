# pi-improved — spec-54: bounded accept pool for pii-net (F16)

- **Project**: `/projects/pi-improved`
- **Priority**: MEDIUM — Phase 5.3 backlog item F16.

## Overview

`crates/pii-net/src/listen.rs`'s `accept_loop` (currently at line ~173)
spawns a raw, unbounded `std::thread::spawn` per accepted TCP
connection (line ~190: `std::thread::spawn(move || serve_conn(stream,
h));`), with no cap on concurrent connection-handler threads. A burst
of connections (malicious or accidental) can spawn unbounded OS
threads. Bound this to a fixed-size worker pool.

Ground truth already confirmed by the orchestrator (verify yourself
before starting):
- `pii-net` is deliberately synchronous (doc comment at
  `crates/pii-net/src/lib.rs:1-4`: "the ONE socket... Synchronous by
  design"). Do NOT introduce `tokio`/async into this crate — a
  synchronous bounded thread pool (channel + N fixed worker threads, or
  a semaphore capping in-flight `std::thread::spawn` calls) is the
  correct shape here, not an async runtime.
- Today's only real consumer is `pii-web` (a read-only, loopback-only
  HTTP viewer per its own doc comment) — low realistic connection
  volume, but the unbounded-spawn pattern is still a real resource-
  exhaustion gap worth closing regardless of current load.
- `serve_conn` (same file) already sets 30s read/write timeouts per
  connection — so a bounded pool's worst case is "at most N connections
  wait up to 30s for a free slot," not an unbounded queue.
- `accept_loop`'s existing shutdown mechanism (`stop:
  std::sync::mpsc::Receiver<()>`, checked via `try_recv()` each loop
  iteration) must keep working — the pool must not prevent `close()`
  (which sends `stop` then joins the accept thread) from returning
  promptly.

## Requirements

### Functional

- R1. Introduce a fixed-size worker pool for `serve_conn` dispatch
  instead of raw `std::thread::spawn` per connection. Suggested shape
  (read the existing code first and choose whichever is the smaller,
  more idiomatic diff): a `crossbeam-channel` (or `std::sync::mpsc`, if
  crossbeam isn't already a workspace dependency — check
  `Cargo.toml`/`Cargo.lock` first) bounded channel of accepted
  `TcpStream`s, with N (see R2) long-lived worker threads spawned once
  at `listen()` time, each looping on `recv()` and calling `serve_conn`.
- R2. Pool size: a small constant, e.g. `const ACCEPT_POOL_SIZE: usize
  = 16;` — pick a number that's clearly a deliberate choice with a
  one-line comment explaining it's a loopback-viewer workload, not
  a public-internet listener, so this is a conservative safety cap, not
  a tuned-for-load number. Do NOT make this configurable via
  `NetConfig` in this spec — that's a follow-up if actually needed; keep
  this spec's diff small and self-contained.
- R3. When the pool is saturated (all N workers busy) and a new
  connection is accepted, back-pressure by blocking on the bounded
  channel's `send()` (i.e. the accept loop itself pauses accepting new
  connections until a worker frees up) rather than spawning an
  unbounded extra thread or dropping the connection silently.
- R4. `close()`'s existing shutdown behavior must still terminate
  promptly: the accept thread must still notice `stop` and return; the
  pool's worker threads do not need to be joined on `close()` (matching
  the existing `join: Option<std::thread::JoinHandle<()>>` pattern,
  which only tracks the accept thread, not per-connection threads
  today) — but do NOT leave the pool workers as detached infinite loops
  with no way to ever terminate if this becomes relevant later; a
  simple approach is fine (e.g. workers naturally exit if the channel
  sender is dropped — verify your chosen channel type's `recv()`
  behavior on a dropped sender before relying on this).
- R5. Existing single-connection behavior (`serve_conn`'s HTTP parsing,
  timeouts, response writing) must be completely unchanged — this spec
  changes ONLY how `serve_conn` gets dispatched to a thread, not what it
  does once running.

### Non-Functional

- N1. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).
- N2. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()`
  outside test modules.
- N3. Do not add tokio or any async runtime to `pii-net` — see ground
  truth above.

## Architecture

```
accept_loop (existing thread, unchanged control flow)
    │
    │ listener.accept() -> TcpStream
    ▼
bounded channel (NEW, capacity = ACCEPT_POOL_SIZE or small multiple)
    │
    ├──► worker thread 1 ─┐
    ├──► worker thread 2  ├─ each loops: recv() -> serve_conn(stream, handler)
    ├──► ...              │
    └──► worker thread N ─┘
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `accept_pool_serves_connections_within_bound` | N+2 concurrent client connections against a listener with pool size N | All N+2 eventually get served (later ones wait, none dropped/errored due to pool saturation alone) |
| `accept_pool_does_not_spawn_unbounded_threads` | Fire M > N connections in a burst, count live threads matching a recognizable name/pattern during the burst | Live worker-thread count stays capped near N (not growing unbounded to M) |
| existing `pii-net` listener/close tests | unchanged | still pass — `close()` still returns promptly and stops accepting new connections |

## Exit Criteria

- [ ] `cargo test -p pii-net`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `! grep -n "std::thread::spawn(move || serve_conn" crates/pii-net/src/listen.rs`
- [ ] `grep -q "ACCEPT_POOL_SIZE" crates/pii-net/src/listen.rs`

## Guardrails

- G1. Do NOT touch `crates/pii-net/src/authorize.rs`, `dial.rs`,
  `grant.rs`, `http_codec.rs`, or any file other than `listen.rs`
  (and a new test file if you choose to add one, or an in-file
  `#[cfg(test)] mod tests` block — either is fine, match the file's
  existing test-module convention if it has one).
- G2. Do NOT change `NetService`'s public API (`listen`, `close`,
  `listeners`, `register_external_listener`, `grants`) — the pool is an
  internal implementation detail of `accept_loop`/`serve_conn`
  dispatch, invisible to callers.
- G3. Do NOT add `tokio` or any async runtime dependency to `pii-net`'s
  `Cargo.toml`.
- G4. Do NOT touch `specs/` (read-only for the workhorse).
- G5. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.

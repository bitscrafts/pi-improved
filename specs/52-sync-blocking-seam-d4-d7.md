# pi-improved — spec-52: D4/D7 sync-blocking seam fixes (channel poll + worker kill)

- **Project**: `<repo>`
- **Priority**: MEDIUM — Phase 5.2 completion. The 026 report's Phase 5.2
  section (not just F-12/F-13/F-14) lists D4/D5/D7 as required items;
  this spec closes D4 and D7 (D5 — `recv_timeout` join — is already
  correctly implemented, see Non-Goals below).

## Overview

Two real synchronization bugs in the daemon's sync/async boundary,
confirmed by direct code reading (ground truth below, verify yourself
before starting):

**D4 — blocking I/O inside a tokio task without `spawn_blocking`.**
`crates/pii-server/src/daemon/router.rs::spawn_all` (line 17) does:
```rust
tokio::spawn(async move {
    loop {
        poll_once(&engine, &ctx, &name);
        tokio::time::sleep(...).await;
    }
})
```
`poll_once` (line 76) calls `channel.poll(&rctx, cursor)` — a **synchronous,
blocking trait method** (`pii_channels::Channel::poll`, e.g.
`DiscordChannel::poll` makes a blocking HTTP call, `CliChannel::poll` does
blocking file I/O). Calling blocking code directly inside an `async
fn`/tokio task without `tokio::task::spawn_blocking` starves the tokio
runtime's worker thread — any other task scheduled on that thread stalls
for the duration of the blocking call. With N channels configured, N
polling tasks each doing this compounds the problem.

**D7 — mutex held across a blocking kill syscall.**
`crates/pii-worker/src/control.rs::death` (line 118) and
`::reload` (around line 208) both do:
```rust
let mut sh = self.inner.state.lock();  // parking_lot::Mutex
...
if let Some(mut c) = sh.child.take() {
    let _ = c.kill();  // pii_core::ChildProc::kill -> blocking kill() + wait()
}
```
`ChildProc::kill` (`crates/pii-core/src/childproc.rs:115`) calls both
`self.inner.kill()` (send SIGKILL) AND `self.inner.wait()` (blocks until
the OS reaps the process) — a genuinely blocking syscall — while `sh`
(the `state` mutex guard) is held. Any other thread trying to acquire
`state` (e.g. `invoke()`'s `self.inner.state.lock()` at the top of the
hot path) blocks for the full duration of the kill+wait, which under load
or a slow-to-die child could be seconds. The existing
`#[allow(clippy::significant_drop_tightening)]` attribute on `death`
acknowledges this drop-timing issue in a code comment but does not fix
it.

## Requirements

### Functional

- R1 (D4 fix). In `router.rs::spawn_all`, wrap the blocking `poll_once`
  call in `tokio::task::spawn_blocking`, `.await`ing the result before
  the sleep:
  ```rust
  tokio::spawn(async move {
      loop {
          let engine2 = Arc::clone(&engine);
          let ctx2 = Arc::clone(&ctx);
          let name2 = name.clone();
          let _ = tokio::task::spawn_blocking(move || {
              poll_once(&engine2, &ctx2, &name2);
          }).await;
          tokio::time::sleep(Duration::from_millis(interval_ms.max(10))).await;
      }
  })
  ```
  (Adjust exact clone/capture shape to what actually compiles — the
  pattern above is illustrative, not verbatim; `poll_once`'s signature
  takes references, so the closure needs owned clones of `engine`/`ctx`/
  `name` moved in, matching how `spawn_all`'s outer closure already
  clones them per-channel.) The polling BEHAVIOR (interval, per-channel
  isolation, error absorption via `ctx.channel_store.record_error`) must
  be unchanged — this is a pure execution-context fix, not a logic
  change.
- R2 (D7 fix, `death`). Restructure `death` so the mutex guard is
  dropped BEFORE calling `c.kill()`: take `sh.child` out while the lock
  is held, then explicitly drop the guard (or narrow its scope with a
  block), THEN call `.kill()` on the extracted `ChildProc` outside the
  lock. The `pid`/`in_flight`/event-push logic that also reads/writes
  `sh` and other `Mutex` fields may still need the lock for their own
  parts — only the actual `.kill()` call must move outside the `state`
  lock's critical section.
- R3 (D7 fix, `reload`). Same restructuring for the `reload` method's
  kill site (~line 220-227): take `sh.child` out under the lock, drop
  the lock, then call `wait_timeout`/`kill` outside it. Note `reload`
  already does `wait_timeout` (bounded, not blocking indefinitely) before
  falling back to `kill()` — preserve that bounded-wait-then-kill
  sequence, just move both operations outside the `state` mutex's
  critical section.
- R4. Do not change `ChildProc::kill`'s own implementation
  (`pii-core/src/childproc.rs:115`) — the fix is entirely about WHERE
  it's called from (outside the lock), not changing what it does.
- R5. Do not change `pii_channels::Channel::poll`'s trait signature or
  any implementor (`CliChannel`, `DiscordChannel`) — they stay
  synchronous; the fix is entirely in how the CALLER (`router.rs`)
  invokes them.

### Non-Goals (do not attempt — already correct or out of scope)

- **D5 is NOT part of this spec.** The 026 report's D5 concern
  ("`recv_timeout` join") is already correctly implemented:
  `pii-worker/src/control.rs:89` and `:60` already use bounded
  `rx.recv_timeout(...)` (not an unbounded `.recv()` or `.join()`), and
  `pii-worker/src/supervisor.rs:307`'s `wait_hello` polls with a bounded
  `recv_timeout(10ms)` inside a deadline loop. Verify this yourself by
  reading those three call sites before starting — if you find an actual
  unbounded blocking `.recv()`/`.join()` this ground truth missed,
  STOP and report it rather than silently fixing it outside this spec's
  scope.
- **F16's bounded accept pool** (`pii-net/src/listen.rs:190`, an
  unbounded `std::thread::spawn` per accepted connection) is a SEPARATE,
  real finding but NOT part of this spec — it's a different subsystem
  (raw TCP accept loop, not the channel-poll/worker-kill seam) and
  deserves its own spec given the connection-pool sizing decisions
  involved. Do not touch `listen.rs` in this spec.

## Architecture

No new files — both fixes are surgical restructurings within existing
files (`crates/pii-server/src/daemon/router.rs`,
`crates/pii-worker/src/control.rs`).

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `poll_once_runs_on_blocking_thread` (new, `pii-server`) | A channel whose `poll` sleeps for e.g. 200ms (a test-only channel implementation or a mock), spawned via the real `spawn_all` inside a single-threaded or minimal-worker tokio runtime | A concurrently-scheduled lightweight tokio task (e.g. one that increments a counter every 10ms) is NOT starved during the 200ms poll — proves the poll runs off the async executor's worker thread |
| `worker_kill_does_not_block_concurrent_invoke` (new, `pii-worker`) | A worker whose child process takes noticeably long to die (or a test double `ChildProc` with an artificial delay in `kill()`/`wait()`), triggering `death()` from one thread while `invoke()` is called from another | `invoke()` returns (with whatever correct error, e.g. `Unavailable`) without waiting for the full kill+wait duration — proves the `state` lock is not held across the kill |

Exact test construction (mock channel/child-proc shape, how to inject an
artificial delay) is left to you — read the existing test doubles in
`pii-worker`'s and `pii-channels`'/`pii-server`'s test modules first and
reuse whatever pattern they already have for simulating slow I/O, rather
than inventing a new one.

## Exit Criteria

- [ ] `grep -q "spawn_blocking" crates/pii-server/src/daemon/router.rs`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `cargo test -p pii-server --lib daemon::router` (existing router test still passes)
- [ ] `cargo test -p pii-worker` (existing supervisor/control tests still pass)

## Guardrails

- G1. Do NOT change `pii_channels::Channel` trait signatures or any
  implementor.
- G2. Do NOT touch `pii-net/src/listen.rs` (F16, separate spec).
- G3. Do NOT touch `pii-worker/src/control.rs`'s `recv_timeout`/deadline
  logic (D5, already correct) — only the two `.kill()` call sites.
- G4. Do NOT touch `specs/` (read-only for the workhorse).
- G5. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G6. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).
- G7. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()`
  outside test modules.

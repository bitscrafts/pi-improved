# Apply restart_backoff_ms in Worker Respawn

- **Project**: `<repo>`
- **Crate**: `pii-worker`
- **Priority**: MEDIUM — a configured knob is silently a no-op. Sourced
  from fable review finding 7,
  `_docs/06-research/032-fable-pii-review-2026-09-18.md` ("Follow-up"
  section) — independently re-verified before writing this spec.

## Overview

`crates/pii-worker/src/control.rs`'s `death()` handler reads
`self.inner.spec.restart_backoff_ms` and passes it to `respawn(sup,
backoff)`. `respawn`'s signature is `fn respawn(sup: Supervisor,
_backoff_ms: u64)` — the leading underscore marks it as deliberately
unused, and the function never sleeps before calling
`spawn_and_hello`. A worker that dies immediately on startup is
respawned back-to-back with zero delay until
`max_restarts_in_window`'s cap trips.

The project's own spec `specs/05-phase3-assistant-gateway.md` (F3.5)
documents the intended behavior: the child "is respawned after
`restart_backoff_ms`". `WorkerSpec`'s default for this field is 50ms
(`spec.rs`, verify current line). The crash-loop cap still bounds the
damage (this is not an unbounded storm), so severity is medium, not
critical — but the configuration knob operators are given today does
nothing.

Ground truth (verify yourself, re-read the file — it has grown since
this spec was drafted):
- `crates/pii-worker/src/control.rs` — `death()`'s respawn-dispatch
  site (search for `restart_backoff_ms` and `respawn(sup, backoff)`),
  and the `respawn` function itself (search for `fn respawn`).
- `crates/pii-worker/tests/supervisor.rs` — the existing
  `restart_completes_under_2s` test (default `restart_backoff_ms: 50`
  in its test `WorkerSpec`, asserting `took_ms < 2000`) — this test
  MUST still pass after your change, since 50ms of actual sleep is
  nowhere near the 2000ms ceiling.

## Requirements

### Functional

- R1. In `respawn`, actually sleep for `backoff_ms` milliseconds
  before calling `spawn_and_hello` — rename the parameter from
  `_backoff_ms` to `backoff_ms` (dropping the underscore now that it's
  used) and add `std::thread::sleep(std::time::Duration::from_millis(backoff_ms))`
  at the top of the function, before the `ctx` lock/clone.
- R2. Remove the now-unnecessary `#[allow(clippy::needless_pass_by_value)]`
  comment/attribute on `respawn` if it was there SPECIFICALLY because
  the parameter was unused (re-read the comment's exact wording before
  removing — if the lint allow is there for an unrelated reason, e.g.
  `sup` itself being passed by value, leave it and only fix the
  `_backoff_ms` → `backoff_ms` rename).
- R3. `took_ms` (the timing metric in the `WorkerEvent::Restarted`
  event) should be measured from a point that INCLUDES the backoff
  sleep — verify where `t0 = Instant::now()` is captured relative to
  where you add the sleep, and place the sleep BEFORE that capture (or
  move the capture earlier) so the reported `took_ms` reflects true
  wall-clock time from death to ready, not just the post-sleep spawn
  time. Get this right — a metric that silently excludes the sleep
  would defeat the purpose of fixing this at all.

### Non-Functional

- N1. `backoff_ms: 0` (if ever configured) must not sleep at all —
  `Duration::from_millis(0)` already behaves this way in `std::thread::sleep`,
  so no special-case is needed, just confirm this with a test.
- N2. Do NOT touch the crash-loop cap logic (the `capped` check
  immediately above the respawn dispatch) — this spec only fixes the
  backoff delay itself.
- N3. Do NOT change `WorkerSpec`'s default `restart_backoff_ms` value
  (50ms) — this spec makes the existing configured value actually
  apply, it does not change what the default is.

## Architecture

Described inline in Overview above (file paths and function names given there constitute the architecture context for this change).

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `respawn_honors_configured_backoff` | a worker with `restart_backoff_ms: 200` that dies and is respawned | the `WorkerEvent::Restarted` event's `took_ms` is `>= 200` |
| `respawn_zero_backoff_does_not_sleep` | a worker with `restart_backoff_ms: 0` that dies and is respawned | respawn happens promptly, no artificial delay (verify via a reasonable upper bound, e.g. `took_ms < 100` for a fast local spawn) |
| `restart_completes_under_2s` | existing test, `restart_backoff_ms: 50` (unchanged) | STILL PASSES — `took_ms < 2000` easily accommodates a real 50ms sleep |

## Exit Criteria

- [ ] `! grep -q "_backoff_ms: u64" crates/pii-worker/src/control.rs`
- [ ] `grep -q "std::thread::sleep" crates/pii-worker/src/control.rs`
- [ ] `cargo test -p pii-worker`
- [ ] `cargo build -p pii-worker`
- [ ] `cargo clippy -p pii-worker -- -D warnings`
- [ ] `cargo fmt --check`

## Guardrails

- Do NOT touch the crash-loop-cap early-return (`if capped { return; }`)
  above the respawn dispatch — this is the exact code class that had a
  real regression earlier tonight (spec-52's D7 fix); be careful not
  to reintroduce anything near it.
- Do NOT modify `spawn_and_hello` itself — only `respawn`'s own body,
  adding the sleep before it calls into `spawn_and_hello`.
- Do NOT modify any file under `specs/`.
- No emojis, no AI-tool mentions in any comment or commit message.
- If `t0`'s capture point turns out to already be BEFORE where you'd
  naturally add the sleep (i.e. R3 is already satisfied by construction
  once you add the sleep in the right place), that's fine — just
  confirm it with the test rather than assuming.

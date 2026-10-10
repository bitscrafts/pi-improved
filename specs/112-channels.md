# Spec: 112 — channels: the trait as an extension point, plus three real adapters

**Project**: /projects/pi-improved
**Phase**: Stage A of the vision plan (`057`), second row. The vision claims "30+ channels"; this spec makes
that claim reachable and honest instead of aspirational: a documented trait anyone can implement, plus real
adapters — and a measured count in `001`.

## What exists (measured 2026-10-10; do not re-derive)
- `crates/pii-channels/src/lib.rs:34` — `pub trait Channel { fn name(); fn kind(); fn poll(&self, ctx, cursor)
  -> Result<Vec<Incoming>, PiiError>; fn send(&self, ctx, to, text) -> Result<(), PiiError>; }`. Clean,
  `Send + Sync`, already third-party-implementable.
- Adapters today: `cli.rs`, `discord.rs` (two). Events already exist and are wired (spec 101):
  `CHANNEL_POLL_FAILED`, `CHANNEL_SEND_FAILED`.
- Config: `[[channel]]` sections (`Config.channel: Vec<ChannelSection>`) with `name`, `kind`,
  `poll_interval_ms`, `graph`; scope per channel is `<runtime_dir>/channels/<name>`.
- The daemon polls each channel and routes an inbound message into a run (F6.5), replying on the same channel.

## Requirements
R1. **A `webhook` channel — the one that makes pii reachable without a vendor SDK.**
    - **Inbound**: a loopback HTTP endpoint (`POST /channel/<name>`) that turns a JSON body into an
      `Incoming`. **It binds 127.0.0.1 by default**; binding wider requires an explicit config key, and in
      that case a **shared secret header is REQUIRED** (reject with 401 and emit a denial event otherwise).
      State the exact keys.
    - **Outbound**: `send` POSTs `{to, text}` to a configured URL through the SAME `Arc<dyn HttpTransport>`
      the engine wires, so the `http:<host>` egress allowlist governs it (a webhook URL outside the allowlist
      must be refused, not attempted).
    - The inbound path must be testable offline (the listener test binds an ephemeral port and posts to it).
R2. **A `telegram` adapter** (Bot API: `getUpdates` for `poll`, `sendMessage` for `send`), written as the
    template a third party can copy: token from config/env, `offset` cursor, long-poll interval, and the
    same offline-test pattern (mock transport, no live network in the gate).
R3. **A `matrix` adapter** (client-server API: `sync` with a `since` cursor, `send` to a room) — the same
    shape, proving the trait holds for a second protocol family (cursor-based sync, not offset polling).
    If Matrix's crypto requirements make a thin adapter dishonest, say so with its evidence and substitute a
    third adapter you CAN make honest (e.g. Slack via `conversations.history` + `chat.postMessage`).
R4. **The extension point is documented**: a new `_docs/0X-guides/writing-a-channel.md` with the trait's
    contract (cursor semantics — `None` = from the beginning, monotone, at-least-once delivery is the
    caller's model; `PiiError::Unavailable` means "retry next tick, never fatal"), the config section, the
    scope rule (`<runtime_dir>/channels/<name>`), the events a channel must emit, and a **copyable adapter
    skeleton** (the telegram one is the reference).
R5. **Every failure is observable and named**: poll failure, send failure, auth rejection, egress denial — all
    through `pii-observe` with names in `ALL` (reuse `CHANNEL_*` where it exists; add and register the new
    ones). A channel that is misconfigured (missing token) must fail LOUDLY at startup or on first poll, and
    never silently poll nothing forever.
R6. **Secrets**: tokens come from env or a 0600 file; never logged, never in health output, never in an event
    field (extend the spec-107 canary test to a channel path if a token could reach one). A channel whose
    token is absent is reported as `Degraded` in `pii health --json`, not as silently healthy.
R7. **`001-pii-vision.md`'s channel claim becomes a number**: update the line to the measured count and name
    the adapters, with the doc pointing at the new guide for the rest.

## Exit Criteria
- [ ] the webhook channel works end to end offline: a POST to the loopback endpoint produces a run and a
      reply POST is attempted to the configured URL (paste both requests from the mock transport)
- [ ] the loopback default is proven: a POST from a non-loopback bind without the secret is refused with 401
      and an event (paste); with the secret it is accepted (paste)
- [ ] the telegram adapter passes its offline tests, including the cursor advancing and a 401 from the API
      mapping to the right error + a `Degraded` health state (paste)
- [ ] the third adapter (matrix or slack) either works the same way or the report states with evidence why it
      was substituted
- [ ] `_docs/0X-guides/writing-a-channel.md` exists with the contract, the skeleton, and the config section
- [ ] a misconfigured channel is `Degraded` in `pii health --json`, not silently healthy (paste)
- [ ] `bash quality-gate.sh` exits 0 (`PASS: lint-observability`, `PASS: coverage-floor` or the documented
      loud skip); `fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report lists files changed, new config keys, new name consts, and the measured channel count

## Guardrails
- **No live network in any gate test**; the adapters are tested against mocked transports.
- A new LISTENER is a new attack surface: loopback by default, secret required if widened, and say in the
  report exactly what is exposed and how it is bounded.
- Do not invent a channel you cannot test offline; an adapter template plus three honest adapters beats ten
  stubs (the vision's "30+" is a destination, not a milestone — count honestly in `001`).
- Never log a token or a full URL that carries one.
- Scratch under `/projects/_tmp/`; keep it small (volume tight), delete your own, no daemon left running.
- Do NOT commit; leave the tree for the orchestrator.

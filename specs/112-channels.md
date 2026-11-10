# Spec: 112 — channels: the trait as an extension point, plus three real adapters

**Project**: <repo>
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
- Scratch under `<scratch>/`; keep it small (volume tight), delete your own, no daemon left running.
- Do NOT commit; leave the tree for the orchestrator.

## Orchestrator verification (2026-10-10)
**My controls (not the worker's word):**
- `cargo test -p pii-channels --test adapters` → **13 passed**; `cargo test -p pii-server --test channel_adapters`
  → **4 passed** — both offline, no network touched.
- **The security seam verified at the source, not from the report**: `webhook.rs:88-102` `authorized()` has all
  five arms (match → Ok, wrong → Err, missing → Err, loopback-and-no-secret → Ok by design, non-loopback-and-no-
  secret → Err) and `same()` (line 161) is a real **length-independent-time** compare (XOR-accumulate over
  `max(len)` with the length difference folded in — not `==` in disguise).
- **Default bind confirmed**: `DEFAULT_BIND = "127.0.0.1:8787"` (`crates/pii-server/src/daemon/adapters.rs:17`),
  matching the config doc at `daemon/config.rs:147`.
- **MUTATION (the one that matters for a security claim)**: replacing `same()`'s body with `true` →
  `non_loopback_bind_refuses_without_the_secret_and_accepts_with_it` **FAILS**; restored → 4 pass. The refusal
  is not decorative.
- **Credential handling verified**: `secret.rs` refuses a secret FILE whose mode is not 0600 (`mode & 0o077 != 0`
  → `Invalid`), which is the property that makes "file" a safe alternative to an env var.

**Honest limits carried forward (from the worker's report, kept verbatim in substance):** Matrix handles
unencrypted rooms only (encrypted events are dropped, documented); the opaque `next_batch` token lives in
memory while the trait cursor is `origin_server_ts`; and **any `PiiError::Denied` from a channel is reported as
`channel.egress.denied`**, so a `Denied` from the CLI channel's file scope would be mislabelled the same way —
a naming imprecision, filed rather than smoothed.

**Note on the worker's `fresh-gate: PASS`** — CORRECTED 2026-10-10 after measuring `xtask/src/fresh_gate.rs`
(`export_head` = `git archive HEAD`, then `if opts.worktree { overlay_worktree(...) }`): the base export is HEAD,
but **`--worktree` overlays the working tree's edited paths on top of it**, so a worker's
`fresh-gate --worktree` DOES cover its own uncommitted work. My earlier note here said the opposite and was
wrong. What fresh-gate cannot do is isolate a run from ANOTHER writer's dirty tree: a concurrent worker's
half-finished edits land in the overlay and fail the gate for reasons that are not yours (measured: spec 115's
mid-edit `cmd_soak.rs` broke a 114 run with `RpcError doesn't implement Display`).

### The fresh-gate criterion CLOSED (2026-10-10, orchestrator, on 112's own HEAD)
`fresh-gate --worktree` on `28f8ff8`: `quality-gate: ALL PASS`, `fresh-gate: PASS`, exit 0, 76G free. This is
the run that counts — the worker's own `fresh-gate: PASS` exported HEAD and therefore tested 111's tree, a
distinction worth remembering every time a worker claims it: **fresh-gate exports the committed HEAD, so a
worker that cannot commit cannot have verified its own work with it.** 112: 9/9.


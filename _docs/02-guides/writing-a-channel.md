# Writing a channel

A channel is anything that implements `pii_channels::Channel`
(`crates/pii-channels/src/lib.rs`). The daemon polls it, starts a run per
inbound message, and replies through the same channel. Shipped kinds: `cli`,
`discord`, `telegram`, `matrix`, `webhook`. `telegram.rs` is the reference to copy.

## The contract

```rust
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;                       // the [[channel]] name
    fn kind(&self) -> &str;                       // "telegram", ...
    fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError>;
    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError>;
    fn health_issue(&self) -> Option<String> { None }   // default: healthy
}
```

- **Cursor.** `None` = from the beginning. Each `Incoming.cursor` is a `u64` that
  is monotone for the channel; the daemon persists the cursor of the last message
  it started a run for and passes it back. Return messages **oldest first**.
  Delivery is **at-least-once**: a message can come back if its run never
  started, so the caller's model must tolerate repeats. If your protocol's cursor
  is not an integer (Matrix's `next_batch`), keep the opaque token in memory and
  use a monotone integer (Matrix: `origin_server_ts`) as the trait cursor.
- **Errors.** `PiiError::Unavailable` = transient, retry next tick, never fatal.
  A misconfiguration is `PiiError::Invalid`. A credential rejection (401/403)
  is `pii_channels::auth_rejected(adapter, status)`. A `PiiError::Denied` from the
  transport is the egress allowlist refusing the host — pass it through unchanged.
- **Blocking.** `poll`/`send` are synchronous and run on the blocking pool; a
  long-poll is fine.
- **Transport.** Make every network call through the `Arc<dyn HttpTransport>` you
  are handed (`pii_core`); never open a socket (the net-check gate forbids it). The
  `http:<host>` allowlist in `[net]` then governs you for free.
- **Scope.** The `Ctx` your methods receive grants `NET` and fs read/write only
  under `<runtime_dir>/channels/<name>`. Keep any file state there.

## Events (all emitted for you by `ObservedChannel`, which wraps every channel)

| name | when |
|---|---|
| `channel.poll.failed` / `channel.send.failed` | any `Err` from `poll` / `send` |
| `channel.auth.rejected` | an error from `auth_rejected`; the webhook's inbound 401 |
| `channel.egress.denied` | a `PiiError::Denied` from the transport |
| `channel.misconfigured` | at startup, when `health_issue()` is `Some` |
| `channel.cursor.lag` (measure) | messages returned by a successful poll |

Events carry channel name, kind and the error text — never message text or a
credential. **Scrub tokens out of every error you build** (`pii_channels::scrub`):
a transport error can echo a URL that contains one.

## Health

`health_issue() -> Some(why)` makes `pii health --json` report the channel
`Degraded` with `why` as the detail. Return it for a missing token and after a
credential rejection (clear it on the next success). A channel with no token must
still construct, fail every `poll` loudly with `Invalid`, and report `Degraded` —
never poll nothing forever in silence.

## Config (`[[channel]]`)

Common: `name`, `kind`, `graph` (run per message), `poll_interval_ms` (default 500).

| kind | keys |
|---|---|
| `telegram` | `token_env` (default `PII_TELEGRAM_TOKEN`), `token_file`, `base_url` |
| `matrix` | `token_env` (default `PII_MATRIX_TOKEN`), `token_file`, `base_url` (homeserver, required), `user_id` (the bot, required) |
| `webhook` | `bind` (default `127.0.0.1:8787`), `url` (outbound), `secret_env`, `secret_file` |
| `discord` | env: `PII_DISCORD_TOKEN`, `PII_DISCORD_CHANNEL_ID`, `PII_DISCORD_API_BASE` |
| `github` | `repo` (`owner/name`, required), `token_env` (default `PII_GITHUB_TOKEN`), `token_file`, `base_url`; env `PII_GITHUB_ALLOW_USERS` (replaces the owner as the allowed labellers of `ready`) |
| `cli` | `inbox`, `outbox` |

Secrets come from an env var or a file with mode 0600 (looser is refused at
startup); never a config value (`pii_channels::secret::resolve`).

**Webhook exposure.** Inbound is `POST /channel/<name>` with
`{"text": "...", "chat": "...", "user": "..."}`. It binds loopback by default;
any non-loopback `bind` makes the `X-Pii-Secret` header mandatory (a request
without it is 401 + `channel.auth.rejected`; no configured secret = every request
refused and the channel `Degraded`). Bodies over 64 KiB are 413; the queue holds
1024 messages (503 beyond). Outbound `send` POSTs `{to, text}` to `url` through the
engine transport, so a host outside `[net]` grants is refused, not dialled.

## Adapter skeleton (from `telegram.rs`)

```rust
pub struct MyChannel {
    name: String,
    transport: Arc<dyn HttpTransport>,
    token: Option<String>,              // None => Degraded + loud poll failure
    rejected: Mutex<Option<String>>,    // set on 401/403, cleared on success
}

impl Channel for MyChannel {
    fn name(&self) -> &str { &self.name }
    fn kind(&self) -> &'static str { "my" }
    fn health_issue(&self) -> Option<String> {
        if self.token.is_none() { return Some("token missing".into()); }
        self.rejected.lock().clone()
    }
    fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        let token = self.token.as_deref().ok_or_else(|| PiiError::Invalid("my: no token".into()))?;
        let offset = cursor.map_or(String::new(), |c| format!("offset={}&", c + 1));
        let req = HttpRequest { method: HttpMethod::Get, url: format!("{BASE}/poll?{offset}"),
            headers: vec![("Authorization".into(), format!("Bearer {token}"))],
            body: Vec::new(), timeout_ms: 35_000 };
        let resp = self.transport.request(ctx, &req)?;       // Denied passes through
        if matches!(resp.status, 401 | 403) { /* set self.rejected */ return Err(auth_rejected("my", resp.status)); }
        if !resp.is_success() { return Err(PiiError::Unavailable(format!("my: status {}", resp.status))); }
        // parse resp.body -> Vec<Incoming>, oldest first, monotone cursor
        unimplemented!()
    }
    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError> { /* POST, same error mapping */ unimplemented!() }
}
```

Then: add the module in `crates/pii-channels/src/lib.rs`, an arm in
`build_raw_channel` (`crates/pii-server/src/daemon/mod.rs`) plus a builder in
`daemon/adapters.rs`, and tests like `tests/adapters.rs` — a scripted mock
`HttpTransport`, no live network: cursor advances, 401 maps to
`is_auth_rejected` and `Degraded`, the token never appears in an event or error.

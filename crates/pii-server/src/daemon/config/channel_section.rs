//! `[[channel]]`: one configured channel. It lives apart from `config.rs` to keep that file under the line limit; `config.rs`
//! re-exports it, so `config::ChannelSection` is unchanged for every caller.

use serde::Deserialize;

/// `[[channel]]` (F5.2).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSection {
    /// Channel name.
    pub name: String,
    /// Channel kind (`discord`, `cli`, `telegram`, `matrix`, `webhook`).
    pub kind: String,
    /// Graph to run per inbound message.
    pub graph: String,
    /// Poll interval in milliseconds.
    #[serde(default = "default_poll_ms")]
    pub poll_interval_ms: u64,
    /// `cli` kind only: the inbox file path.
    #[serde(default)]
    pub inbox: Option<String>,
    /// `cli` kind only: the outbox file path.
    #[serde(default)]
    pub outbox: Option<String>,
    /// `telegram`/`matrix`: env var holding the token (default
    /// `PII_TELEGRAM_TOKEN` / `PII_MATRIX_TOKEN`). The token is never a config value.
    #[serde(default)]
    pub token_env: Option<String>,
    /// `telegram`/`matrix`: a 0600 file holding the token (used when the env var is unset).
    #[serde(default)]
    pub token_file: Option<String>,
    /// `telegram`: Bot API base; `matrix`: the homeserver base URL (required).
    #[serde(default)]
    pub base_url: Option<String>,
    /// `matrix` only: the bot's own user id (its events are skipped).
    #[serde(default)]
    pub user_id: Option<String>,
    /// `webhook` only: listen address, default `127.0.0.1:8787`. A
    /// non-loopback address makes the shared secret mandatory.
    #[serde(default)]
    pub bind: Option<String>,
    /// `webhook` only: URL that `send` POSTs `{to, text}` to (egress-allowlisted).
    #[serde(default)]
    pub url: Option<String>,
    /// `webhook` only: env var holding the shared secret (`X-Pii-Secret`).
    #[serde(default)]
    pub secret_env: Option<String>,
    /// `webhook` only: a 0600 file holding the shared secret.
    #[serde(default)]
    pub secret_file: Option<String>,
    /// `github` only: the repository, `owner/name`, whose `ready` issues the queue reads.
    #[serde(default)]
    pub repo: Option<String>,
    /// Spec 137: `host:port` targets the operator sanctions for this channel's egress, for a host the
    /// channel takes from its environment (`discord` with `PII_DISCORD_API_BASE`).
    #[serde(default)]
    pub hosts: Vec<String>,
}

/// Discord answers `GET /channels/{id}/messages` at roughly five requests per five seconds per route, so
/// one request per second is the ceiling and anything faster is refused with `429 retry_after`.
///
/// Measured 2026-11-02 on production: `poll_interval_ms` defaulted to 500 and the daemon collected **173**
/// `channel.poll.failed 429` in a few hours, never reading a message. 1500 ms leaves the bucket room to
/// refill between reads.
const fn default_poll_ms() -> u64 {
    1_500
}

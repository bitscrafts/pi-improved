//! `pii-channels` (F6.1): inbound polling + outbound reply, no sockets of its
//! own (the webhook channel's listener is bound by the daemon through
//! `NetService`; this crate only holds its request handler).
//!
//! Every channel goes through `Arc<dyn HttpTransport>` (from `pii-core`),
//! which is what keeps this crate testable with a fake and keeps
//! `cargo xtask net-check` clean — `pii-net` never appears in this crate's
//! dependency graph.

#![forbid(unsafe_code)]

pub mod cli;
pub mod discord;
pub mod github;
pub mod matrix;
pub mod mqtt;
pub mod observe;
pub mod poll;
pub mod post;
pub mod secret;
pub mod telegram;
pub mod webhook;

#[cfg(test)]
mod tests {
    use super::url_target;

    #[test]
    fn url_target_is_host_and_port() {
        let t = |u| url_target(u);
        assert_eq!(
            t("https://discord.com/api/v10").as_deref(),
            Some("discord.com:443")
        );
        assert_eq!(
            t("http://127.0.0.1:8080/x?y").as_deref(),
            Some("127.0.0.1:8080")
        );
        assert_eq!(t("http://u:p@h.test/").as_deref(), Some("h.test:80"));
        assert_eq!(t("nonsense"), None);
        assert_eq!(t("https:///x"), None);
    }
}

use pii_abi::PiiError;
use pii_core::Ctx;

/// One inbound message from a channel (F6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    /// Channel name.
    pub channel: String,
    /// Chat/conversation id to reply into.
    pub chat: String,
    /// Sending user.
    pub user: String,
    /// Message text.
    pub text: String,
    /// Monotonic cursor for this message (channel-defined ordering).
    pub cursor: u64,
}

/// Message counters a channel may expose (`pii metrics`, spec 117 R6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelCounters {
    /// Messages accepted from the remote side.
    pub received: u64,
    /// Messages dropped and counted (queue full, malformed, ...).
    pub dropped: u64,
    /// Messages handed to the remote side.
    pub published: u64,
}

/// A pollable, reply-capable channel (F6.2). The trait is the extension point
/// (spec 112, `writing-a-channel.md`); inbound push (webhook) is a queue that
/// `poll` drains, so the router stays poll-shaped.
pub trait Channel: Send + Sync {
    /// The channel's configured name (from `[[channel]]`).
    fn name(&self) -> &str;
    /// The channel kind (`"discord"`, `"cli"`, ...).
    fn kind(&self) -> &str;
    /// Fetches messages newer than `cursor` (`None` = from the beginning).
    ///
    /// # Errors
    /// [`PiiError::Unavailable`] on a transient failure (bad status,
    /// unparseable body) — the router retries next tick and never treats
    /// this as fatal.
    fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError>;
    /// Sends `text` to `to` (channel-defined recipient: chat id, file, ...).
    ///
    /// # Errors
    /// [`PiiError::Unavailable`] on a transient failure.
    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError>;
    /// [`Channel::send`] with an optional per-post `tag` (spec 132). A channel with no notion of
    /// tags ignores it, which is this default.
    ///
    /// # Errors
    /// As [`Channel::send`].
    fn send_tagged(
        &self,
        ctx: &Ctx,
        to: &str,
        text: &str,
        tag: Option<&str>,
    ) -> Result<(), PiiError> {
        let _ = tag;
        self.send(ctx, to, text)
    }
    /// The `host:port` targets this channel dials, for its egress grant (spec 132; the channel
    /// analogue of a model provider's `egress_host`). Empty = no network: the daemon grants no
    /// host, so a channel that forgets to declare one fails closed.
    fn egress_hosts(&self) -> Vec<String> {
        Vec::new()
    }
    /// The subset of [`Self::egress_hosts`] that did not come from the configuration file (an
    /// environment override), which the daemon refuses unless `[net] grants` or the channel's
    /// `hosts` sanction it (spec 137). Empty = every host was named by the config itself.
    fn env_derived_hosts(&self) -> Vec<String> {
        Vec::new()
    }
    /// Why this channel cannot work right now (missing token, credential
    /// rejected, ...), or `None` when healthy. A `Some` is reported as
    /// `Degraded` by the daemon's health (spec 112 R6). Never contains a secret.
    fn health_issue(&self) -> Option<String> {
        None
    }
    /// Message counters, for channels that keep them (`None` = none kept).
    fn counters(&self) -> Option<ChannelCounters> {
        None
    }
}

/// The `host:port` an HTTP(S) `url` dials (`https` defaults to 443, `http` to 80); userinfo, path
/// and query are dropped. `None` when `url` has no scheme or host.
#[must_use]
pub fn url_target(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or_default();
    let port = match scheme {
        "https" => 443,
        "http" => 80,
        _ => return None,
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !p.contains(']') => (h, p.parse::<u16>().ok()?),
        _ => (authority, port),
    };
    (!host.is_empty()).then(|| format!("{host}:{port}"))
}

/// Message prefix that marks a credential rejection (see [`auth_rejected`]).
const AUTH_REJECTED: &str = "auth rejected";

/// The error for an upstream 401/403: not transient, not an egress denial.
#[must_use]
pub fn auth_rejected(adapter: &str, status: u16) -> PiiError {
    PiiError::Invalid(format!("{adapter}: {AUTH_REJECTED} (status {status})"))
}

/// True when `e` came from [`auth_rejected`].
#[must_use]
pub fn is_auth_rejected(e: &PiiError) -> bool {
    matches!(e, PiiError::Invalid(m) if m.contains(AUTH_REJECTED))
}

/// The pause a provider asked for, in ms: reads `"retry_after":<secs>` (Discord's JSON body) or
/// `retry_after=<secs>s` from the error text, clamped to `1..=600_000`. `None` when absent or
/// unparseable.
#[must_use]
pub fn retry_after_ms(e: &PiiError) -> Option<u64> {
    let text = match e {
        PiiError::Unavailable(m) | PiiError::Invalid(m) => m.as_str(),
        _ => return None,
    };
    let rest = ["\"retry_after\":", "retry_after="]
        .iter()
        .find_map(|key| text.split_once(key).map(|(_, r)| r))?;
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    let secs: f64 = rest.get(..end)?.parse().ok()?;
    if !secs.is_finite() {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ms = (secs * 1000.0).round().min(1.0e9) as u64;
    Some(ms.clamp(1, 600_000))
}

/// Replaces every occurrence of `secret` in `text` (a token embedded in a URL
/// that a transport error echoed back must not reach logs).
#[must_use]
pub fn scrub(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_owned()
    } else {
        text.replace(secret, "***")
    }
}

// Top level so the spec's `--exact` filter names it without a module path.
#[cfg(test)]
#[test]
fn the_retry_after_token_is_read_from_a_real_refusal() {
    let real = PiiError::Unavailable(String::from(
        r#"discord: forum post status 429: {"message":"You are being rate limited.","retry_after":132.176,"global":false}"#,
    ));
    assert_eq!(retry_after_ms(&real), Some(132_176));
    let poll_form = PiiError::Unavailable(String::from("discord: poll retry_after=1.5s"));
    assert_eq!(retry_after_ms(&poll_form), Some(1_500));
    let old = PiiError::Unavailable(String::from("unavailable: discord: poll status 429"));
    assert_eq!(retry_after_ms(&old), None);
    let junk = PiiError::Unavailable(String::from(r#"{"retry_after":"soon"}"#));
    assert_eq!(retry_after_ms(&junk), None);
    let huge = PiiError::Unavailable(String::from(r#"{"retry_after":99999}"#));
    assert_eq!(retry_after_ms(&huge), Some(600_000));
    let tiny = PiiError::Unavailable(String::from(r#"{"retry_after":0}"#));
    assert_eq!(retry_after_ms(&tiny), Some(1));
}
